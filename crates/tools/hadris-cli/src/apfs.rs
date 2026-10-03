use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris::apfs::VolumeSelector;
use hadris::apfs::sync::{ApfsFs, Container};
use hadris::fs::sync::{FileSystem, Volume, read_tree};
use hadris::fs::{DirCursor, MountOptions, OpenMode, Resolve, host};
use hadris::io::StdIo;
use hadris::storage::sync::StreamDevice;
use hadris::storage::{BlockSize, Partition, ReadOnly};

type Device = Partition<StreamDevice<ReadOnly<StdIo<std::fs::File>>>>;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Inspect the container and list its volumes
    Info(ImageArgs),
    /// List a directory in a selected volume
    #[command(name = "ls", alias = "list")]
    List(PathArgs),
    /// Show metadata for a path
    Stat(PathArgs),
    /// Write file contents to stdout
    Cat(PathArgs),
    /// Extract a directory or file into a host directory
    Extract {
        #[command(flatten)]
        source: PathArgs,
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },
}

#[derive(clap::Args)]
struct PathArgs {
    #[command(flatten)]
    image: ImageArgs,
    #[arg(default_value = "/")]
    path: String,
    /// Volume's APFS fs_index; required for multi-volume containers
    #[arg(long, group = "volume_selector")]
    volume: Option<u32>,
    /// Exact volume name; ambiguous names are refused
    #[arg(long, group = "volume_selector")]
    volume_name: Option<String>,
    /// Volume superblock object identifier
    #[arg(long, group = "volume_selector")]
    volume_object_id: Option<u64>,
    /// Volume UUID in canonical hexadecimal form
    #[arg(long, group = "volume_selector", value_parser = parse_uuid)]
    volume_uuid: Option<[u8; 16]>,
}

#[derive(clap::Args)]
struct ImageArgs {
    image: PathBuf,
    /// Select the sole APFS GPT partition in a whole-disk image
    #[arg(long)]
    gpt: bool,
    /// Partition table index (as stored in GPT/MBR)
    #[arg(long)]
    partition: Option<usize>,
    /// Logical disk sector size, in bytes
    #[arg(long, default_value_t = 512)]
    sector_size: u32,
}

fn open_device(args: &ImageArgs) -> Result<Device> {
    let block_size = BlockSize::new(args.sector_size).context("sector size must be nonzero")?;
    if args.sector_size > 4096 || 4096 % args.sector_size != 0 {
        bail!("sector size must divide 4096 and be at most 4096 bytes");
    }
    let file = std::fs::File::open(&args.image)
        .with_context(|| format!("cannot open {}", args.image.display()))?;
    let len = hadris::storage::host::file_len(&file)?;
    let mut device = StreamDevice::with_block_count(
        ReadOnly::new(StdIo::new(file)),
        block_size,
        len / u64::from(args.sector_size),
    );
    if args.gpt || args.partition.is_some() {
        let disk = hadris::part::sync::read(&mut device).context("cannot read partition table")?;
        let mut matches = disk.partitions().filter(|part| match args.partition {
            Some(index) => part.index() == index,
            None => {
                part.kind()
                    == hadris::part::PartitionKind::Gpt(hadris::part::gpt::types::APPLE_APFS)
            }
        });
        let selected = matches.next().context("no matching APFS partition found")?;
        if matches.next().is_some() {
            bail!("multiple APFS partitions found; select one with --partition");
        }
        Ok(hadris::part::sync::open(device, &selected)?)
    } else {
        Ok(Partition::new(device, 0, len))
    }
}

fn parse_uuid(value: &str) -> Result<[u8; 16], String> {
    if value.len() != 36
        || ![8, 13, 18, 23]
            .into_iter()
            .all(|i| value.as_bytes()[i] == b'-')
    {
        return Err("expected a UUID such as 01234567-89ab-cdef-0123-456789abcdef".into());
    }
    let digits: String = value.chars().filter(|c| *c != '-').collect();
    if digits.len() != 32 || !digits.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("UUID must contain 32 hexadecimal digits".into());
    }
    let mut uuid = [0; 16];
    for (i, byte) in uuid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).map_err(|err| err.to_string())?;
    }
    Ok(uuid)
}

fn mount(args: &PathArgs) -> Result<ApfsFs<Device>> {
    let device = open_device(&args.image)?;
    let options = MountOptions::new().read_only();
    let selector = args
        .volume
        .map(VolumeSelector::Index)
        .or_else(|| args.volume_name.as_deref().map(VolumeSelector::Name))
        .or_else(|| args.volume_object_id.map(VolumeSelector::ObjectId))
        .or_else(|| args.volume_uuid.map(VolumeSelector::Uuid));
    let mounted = match selector {
        Some(selector) => ApfsFs::mount_volume(device, options, selector),
        None => ApfsFs::mount(device, options),
    };
    mounted.with_context(|| format!("cannot mount APFS in {}", args.image.image.display()))
}

pub fn run(args: Args) -> Result<()> {
    match args.command {
        Command::Info(image) => {
            let mut container = Container::open(open_device(&image)?)?;
            let latest = container.latest_superblock()?;
            println!("Block size: {}", latest.block_size);
            println!("Block count: {}", latest.block_count);
            for volume in container.volume_superblocks(&latest)? {
                let uuid = volume.volume_id;
                let hex = uuid
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                println!(
                    "Object ID: {}; UUID: {}-{}-{}-{}-{}",
                    volume.object.identifier,
                    &hex[..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..]
                );
                println!(
                    "Volume {}: {} ({} files, {} directories, {} snapshots)",
                    volume.fs_index,
                    volume.name()?,
                    volume.number_files,
                    volume.number_directories,
                    volume.number_snapshots
                );
            }
        }
        Command::List(source) => {
            let mut fs = mount(&source)?;
            let dir = fs.resolve(source.path.as_bytes(), Resolve::Follow)?;
            let mut cursor = DirCursor::START;
            while let Some(entry) = fs.readdir(dir, cursor)? {
                cursor = entry.next_cursor();
                println!("{}", String::from_utf8_lossy(entry.name().as_bytes()));
            }
            fs.forget(dir, 1);
        }
        Command::Stat(source) => {
            let mut fs = mount(&source)?;
            let node = fs.resolve(source.path.as_bytes(), Resolve::Lexical)?;
            let result = fs.stat(node);
            fs.forget(node, 1);
            println!("{:#?}", result?);
        }
        Command::Cat(source) => {
            let mut fs = mount(&source)?;
            let node = fs.resolve(source.path.as_bytes(), Resolve::Follow)?;
            fs.open(node, OpenMode::Read)?;
            let copied = (|| -> Result<()> {
                let mut stdout = io::stdout().lock();
                let mut buffer = [0u8; 64 * 1024];
                let mut offset = 0;
                loop {
                    let count = fs.read(node, offset, &mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    stdout.write_all(&buffer[..count])?;
                    offset += count as u64;
                }
                stdout.flush()?;
                Ok(())
            })();
            let closed = fs.close(node);
            fs.forget(node, 1);
            copied?;
            closed?;
        }
        Command::Extract { source, output } => {
            if output.exists() && !output.is_dir() {
                bail!(
                    "extraction destination is not a directory: {}",
                    output.display()
                );
            }
            let mut fs = mount(&source)?;
            let node = fs.resolve(source.path.as_bytes(), Resolve::Lexical)?;
            let metadata = fs.stat(node);
            let root = node == fs.root();
            fs.forget(node, 1);
            let metadata = metadata?;
            let target = if root || !metadata.file_type().is_dir() {
                output
            } else {
                let parent =
                    fs.resolve(format!("{}/..", source.path).as_bytes(), Resolve::Lexical)?;
                let mut cursor = DirCursor::START;
                let name = loop {
                    match fs.readdir(parent, cursor)? {
                        Some(entry) if entry.node() == node => {
                            break entry.name().as_bytes().to_vec();
                        }
                        Some(entry) => cursor = entry.next_cursor(),
                        None => bail!("path is not listed in its parent directory"),
                    }
                };
                fs.forget(parent, 1);
                let name = std::str::from_utf8(&name)?;
                if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\']) {
                    bail!("directory name is not a plain host path component");
                }
                output.join(name)
            };
            let volume = Volume::new(fs);
            let tree = read_tree(&volume, &source.path)?;
            std::fs::create_dir_all(&target)?;
            let report = host::write_tree(&target, &tree)?;
            for warning in report.warnings() {
                eprintln!("warning: {warning}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn uuid_selectors_require_canonical_hexadecimal_bytes() {
        assert_eq!(
            super::parse_uuid("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
            [
                1, 35, 69, 103, 137, 171, 205, 239, 1, 35, 69, 103, 137, 171, 205, 239
            ]
        );
        for invalid in [
            "",
            "0123456789abcdef0123456789abcdef",
            "g1234567-89ab-cdef-0123-456789abcdef",
        ] {
            assert!(super::parse_uuid(invalid).is_err());
        }
    }

    #[test]
    fn selectors_are_explicit_and_mutually_exclusive() {
        assert!(
            crate::Cli::try_parse_from([
                "hadris",
                "apfs",
                "list",
                "image.apfs",
                "/",
                "--volume",
                "2"
            ])
            .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "hadris",
                "apfs",
                "cat",
                "image.apfs",
                "/file",
                "--volume-name",
                "Data"
            ])
            .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "hadris",
                "apfs",
                "list",
                "image.apfs",
                "--volume",
                "2",
                "--volume-name",
                "Data"
            ])
            .is_err()
        );
    }
}
