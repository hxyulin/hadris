//! `hadris ls|cat|extract|stat`: any filesystem `hadris::sync::open` mounts.

use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris::fs::sync::FileSystem;
use hadris::fs::{FileType, MountError, Resolve};
use hadris::host::FileDevice;
use hadris::sync::AnyFs;
use hadris::{ErrorKind, ImageFormat, Select};

use crate::common::{Device, Locate};
use crate::{detect, fat};

/// The image a generic command reads.
#[derive(clap::Args)]
pub struct Source {
    /// Path to the image or device; a partitioned disk without --partition
    /// mounts its first partition holding a filesystem
    image: PathBuf,
    #[command(flatten)]
    locate: Locate,
}

#[derive(clap::Args)]
pub struct LsArgs {
    #[command(flatten)]
    source: Source,
    /// Directory within the filesystem
    #[arg(default_value = "/")]
    path: String,
    /// Show type, attributes and size
    #[arg(short, long)]
    long: bool,
}

#[derive(clap::Args)]
pub struct CatArgs {
    #[command(flatten)]
    source: Source,
    /// File within the filesystem
    path: String,
}

#[derive(clap::Args)]
pub struct ExtractArgs {
    #[command(flatten)]
    source: Source,
    /// Output directory
    #[arg(short, long, default_value = ".")]
    output: PathBuf,
    /// Path within the filesystem (default: extract all); a path other than
    /// the root is written to `<output>/<name>`
    #[arg(short, long)]
    path: Option<String>,
}

#[derive(clap::Args)]
pub struct StatArgs {
    #[command(flatten)]
    source: Source,
    /// Path within the filesystem
    #[arg(default_value = "/")]
    path: String,
}

/// Mounts the filesystem `source` selects read-only: the partition in the
/// slot `--partition` names, the volume at `--offset`, the first partition
/// holding a filesystem when the image starts with a partition table, or
/// the image.
fn open(source: &Source) -> Result<AnyFs<Device>> {
    let image = &source.image;
    let options = hadris::host::mount_options().read_only();
    let context = || format!("cannot mount {}", image.display());
    let dev =
        FileDevice::open(image).with_context(|| format!("cannot open {}", image.display()))?;
    if let Some(slot) = source.locate.partition {
        return hadris::sync::open_partition(dev, Select::Index(slot), options)
            .map_err(MountError::into_error)
            .with_context(context);
    }
    let mut dev = source.locate.select(dev)?;
    let found = hadris::sync::detect(&mut dev)
        .with_context(|| format!("cannot read {}", image.display()))?;
    let table = found
        .first()
        .is_some_and(|c| matches!(c.format(), ImageFormat::Mbr | ImageFormat::Gpt));
    if table && dev.offset() == 0 {
        return hadris::sync::open_partition(dev.into_inner(), Select::First, options)
            .map_err(MountError::into_error)
            .with_context(context);
    }
    hadris::sync::open(dev, options)
        .map_err(MountError::into_error)
        .with_context(context)
}

pub fn ls(args: LsArgs) -> Result<()> {
    fat::cmd_ls(&mut open(&args.source)?, &args.path, args.long)
}

pub fn cat(args: CatArgs) -> Result<()> {
    fat::cmd_cat(&mut open(&args.source)?, &args.path)
}

pub fn extract(args: ExtractArgs) -> Result<()> {
    fat::cmd_extract(open(&args.source)?, &args.output, args.path.as_deref())
}

pub fn stat(args: StatArgs) -> Result<()> {
    let mut fs = open(&args.source)?;
    println!("Format:      {}", detect::name(fs.image_format()));
    let mut buf = [0u8; 256];
    if let Some(label) = fs.label(&mut buf).context("cannot read the label")? {
        println!("Label:       {label}");
    }
    match fs.volume_serial() {
        Ok(Some(serial)) => println!("Serial:      {serial:08X}"),
        Ok(None) => {}
        Err(err) if err.kind() == ErrorKind::Unsupported => {}
        Err(err) => return Err(err).context("cannot read the serial number"),
    }
    let stats = fs.statfs().context("cannot read the volume statistics")?;
    let block = u64::from(stats.block_size());
    println!("Block size:  {block} bytes");
    println!(
        "Capacity:    {} ({} bytes)",
        fat::format_size(stats.total_bytes()),
        stats.total_bytes()
    );
    println!(
        "Used:        {}",
        fat::format_size(stats.used_blocks() * block)
    );
    println!(
        "Free:        {}",
        fat::format_size(stats.free_blocks() * block)
    );

    let node = fs
        .resolve(args.path.as_bytes(), Resolve::Lexical)
        .with_context(|| format!("cannot open {}", args.path))?;
    let meta = fs.stat(node);
    fs.forget(node, 1);
    let meta = meta.with_context(|| format!("cannot read the metadata of {}", args.path))?;
    let kind = match meta.file_type() {
        FileType::File => "file",
        FileType::Dir => "directory",
        FileType::Symlink => "symbolic link",
        FileType::CharDevice => "character device",
        FileType::BlockDevice => "block device",
        FileType::Fifo => "named pipe",
        FileType::Socket => "socket",
        _ => "other",
    };
    println!();
    println!("Path:        {}", args.path);
    println!("Type:        {kind}");
    println!("Size:        {} bytes", meta.len());
    if let Some(time) = meta.modified() {
        let (date, clock) = time.to_civil_utc();
        println!(
            "Modified:    {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            date.year(),
            date.month(),
            date.day(),
            clock.hour(),
            clock.minute(),
            clock.second()
        );
    }
    Ok(())
}
