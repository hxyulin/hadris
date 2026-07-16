use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Parser, Subcommand};
use hadris_cd::{CdOptions, CdWriter, FileTree, JolietLevel};
use hadris_iso::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use hadris_iso::boot::{EmulationType, PlatformId};
use hadris_iso::directory::DirectoryRef;
use hadris_iso::read::IsoImage;
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::HybridBootOptions;
use hadris_udf::{UdfFs, UdfRevision};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(
    name = "hadris-cd",
    author,
    version,
    about = "Create and verify hybrid ISO 9660/UDF optical images"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an ISO/UDF bridge image from a host directory
    Create(CreateArgs),
    /// Report the filesystems and volume metadata present in an image
    Info(ImageArgs),
    /// Compare the complete ISO and UDF namespace trees
    Verify(ImageArgs),
}

#[derive(clap::Args)]
struct ImageArgs {
    /// Optical image to inspect
    input: PathBuf,
}

#[derive(clap::Args)]
struct CreateArgs {
    /// Directory containing files to include
    source: PathBuf,
    /// Output image path
    #[arg(short, long)]
    output: PathBuf,
    /// Volume identifier used by both filesystems
    #[arg(short = 'V', long, default_value = "CDROM")]
    volume_name: String,
    /// UDF revision: 1.02, 1.50, 2.00, 2.01, 2.50, or 2.60
    #[arg(long, default_value = "1.02")]
    udf_revision: RevisionArg,
    /// Disable the default Joliet level 3 namespace
    #[arg(long)]
    no_joliet: bool,
    /// Enable Rock Ridge metadata
    #[arg(short = 'R', long)]
    rock_ridge: bool,
    /// Image-relative path to an El Torito BIOS boot image
    #[arg(short, long)]
    boot: Option<String>,
    /// Image-relative path to an El Torito UEFI boot image
    #[arg(long)]
    efi_boot: Option<String>,
    /// Number of 512-byte sectors loaded for the BIOS boot image
    #[arg(long, default_value = "4")]
    boot_load_size: u16,
    /// Add an El Torito boot information table to the BIOS image
    #[arg(long, requires = "boot")]
    boot_info_table: bool,
    /// Add an isohybrid MBR
    #[arg(long)]
    hybrid_mbr: bool,
    /// Add a hybrid GPT
    #[arg(long)]
    hybrid_gpt: bool,
}

#[derive(Clone, Copy)]
struct RevisionArg(UdfRevision);

impl FromStr for RevisionArg {
    type Err = &'static str;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let revision = match value {
            "1.02" => UdfRevision::V1_02,
            "1.50" => UdfRevision::V1_50,
            "2.00" => UdfRevision::V2_00,
            "2.01" => UdfRevision::V2_01,
            "2.50" => UdfRevision::V2_50,
            "2.60" => UdfRevision::V2_60,
            _ => return Err("expected 1.02, 1.50, 2.00, 2.01, 2.50, or 2.60"),
        };
        Ok(Self(revision))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Directory,
    File(Vec<u8>),
}

fn main() {
    let result = match Args::parse().command {
        Command::Create(args) => create(args),
        Command::Info(args) => info(&args.input),
        Command::Verify(args) => verify(&args.input),
    };
    if let Err(error) = result {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn create(args: CreateArgs) -> Result<()> {
    if !args.source.is_dir() {
        return Err(format!("source is not a directory: {}", args.source.display()).into());
    }
    validate_host_tree(&args.source)?;

    let tree = FileTree::from_fs(&args.source)?;
    let source_bytes = tree
        .root
        .iter_files()
        .into_iter()
        .try_fold(0_u64, |total, file| {
            file.size().map(|size| total.saturating_add(size))
        })?;
    let entry_count = tree.total_files() + tree.total_dirs();
    let capacity = source_bytes
        .saturating_add((entry_count as u64).saturating_mul(64 * 1024))
        .saturating_add(8 * 1024 * 1024)
        .max(16 * 1024 * 1024);
    let capacity = usize::try_from(capacity).map_err(|_| "image is too large for this platform")?;

    let mut options = CdOptions::default().volume_id(args.volume_name.clone());
    options.udf.revision = args.udf_revision.0;
    options.iso.joliet = (!args.no_joliet).then_some(JolietLevel::Level3);
    options.iso.rock_ridge = args.rock_ridge.then(RripOptions::default);
    options.boot = boot_options(&args);
    options.hybrid_boot = match (args.hybrid_mbr, args.hybrid_gpt) {
        (true, true) => Some(HybridBootOptions::hybrid()),
        (true, false) => Some(HybridBootOptions::mbr()),
        (false, true) => Some(HybridBootOptions::gpt()),
        (false, false) => None,
    };

    let cursor = Cursor::new(vec![0_u8; capacity]);
    let output = CdWriter::create(cursor, tree, options)?;
    let data = output.into_inner();
    let mut file = File::create(&args.output)?;
    file.write_all(&data)?;
    println!("Created: {}", args.output.display());
    Ok(())
}

fn boot_options(args: &CreateArgs) -> Option<BootOptions> {
    let default_path = args.boot.as_ref().or(args.efi_boot.as_ref())?;
    let efi_only = args.boot.is_none();
    let mut options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: normalize(default_path),
            load_size: if efi_only {
                None
            } else {
                NonZeroU16::new(args.boot_load_size)
            },
            boot_info_table: !efi_only && args.boot_info_table,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: Vec::new(),
    };
    if let Some(efi) = &args.efi_boot {
        // The default entry has no platform field in El Torito's Rust model.
        // Emit an explicit UEFI section as well so EFI-only catalogs carry the
        // platform ID expected by firmware, while retaining the EFI image as
        // the catalog's default entry.
        options.entries.push((
            BootSectionOptions {
                platform: PlatformId::UEFI,
            },
            BootEntryOptions {
                boot_image_path: normalize(efi),
                load_size: None,
                boot_info_table: false,
                grub2_boot_info: false,
                emulation: EmulationType::NoEmulation,
            },
        ));
    }
    Some(options)
}

fn validate_host_tree(path: &Path) -> Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let name = entry.file_name();
        if name.to_str().is_none() {
            return Err(
                format!("host filename is not valid UTF-8: {}", entry_path.display()).into(),
            );
        }
        let metadata = std::fs::symlink_metadata(&entry_path)?;
        if metadata.file_type().is_symlink() {
            return Err(
                format!("symbolic links are not supported: {}", entry_path.display()).into(),
            );
        }
        if metadata.is_dir() {
            validate_host_tree(&entry_path)?;
        } else if !metadata.is_file() {
            return Err(format!(
                "special host entries are not supported: {}",
                entry_path.display()
            )
            .into());
        }
    }
    Ok(())
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

fn info(path: &Path) -> Result<()> {
    let iso = IsoImage::open(BufReader::new(File::open(path)?)).ok();
    let udf = UdfFs::open(File::open(path)?).ok();
    if iso.is_none() && udf.is_none() {
        return Err("image contains neither a readable ISO 9660 nor UDF filesystem".into());
    }

    println!("Optical image: {}", path.display());
    println!("  ISO 9660: {}", yes_no(iso.is_some()));
    println!("  UDF:      {}", yes_no(udf.is_some()));
    println!("  Bridge:   {}", yes_no(iso.is_some() && udf.is_some()));
    if let Some(iso) = iso {
        let pvd = iso.read_pvd()?;
        println!("  ISO volume: {}", pvd.volume_identifier);
        println!("  ISO size:   {} sectors", pvd.volume_space_size.read());
        println!("  Rock Ridge: {}", yes_no(iso.supports_rrip()));
    }
    if let Some(udf) = udf {
        println!(
            "  UDF volume: {}",
            udf.info().volume_id.trim_end_matches('\0')
        );
        println!("  UDF revision: {}", udf.info().udf_revision);
    }
    Ok(())
}

fn verify(path: &Path) -> Result<()> {
    let iso = IsoImage::open(BufReader::new(File::open(path)?))
        .map_err(|error| format!("ISO namespace is not readable: {error}"))?;
    let udf = UdfFs::open(File::open(path)?)
        .map_err(|error| format!("UDF namespace is not readable: {error}"))?;

    let mut iso_nodes = BTreeMap::new();
    collect_iso(&iso, iso.root_dir().dir_ref(), "", &mut iso_nodes)?;
    let mut udf_nodes = BTreeMap::new();
    let root = udf.root_dir()?;
    collect_udf(&udf, &root, "", &mut udf_nodes)?;

    if iso_nodes != udf_nodes {
        for key in iso_nodes.keys().chain(udf_nodes.keys()) {
            if iso_nodes.get(key) != udf_nodes.get(key) {
                eprintln!("  mismatch: {key}");
            }
        }
        return Err("ISO and UDF namespaces differ".into());
    }
    println!(
        "Verified: {} ({} shared entries)",
        path.display(),
        iso_nodes.len()
    );
    Ok(())
}

fn collect_iso<R: Read + Seek>(
    iso: &IsoImage<R>,
    directory: DirectoryRef,
    prefix: &str,
    nodes: &mut BTreeMap<String, Node>,
) -> Result<()> {
    for entry in iso.open_dir(directory).entries() {
        let entry = entry?;
        if entry.is_special() {
            continue;
        }
        let name = clean_iso_name(entry.name());
        let path = join(prefix, &name);
        if entry.is_directory() {
            nodes.insert(path.clone(), Node::Directory);
            collect_iso(iso, entry.as_dir_ref(iso)?, &path, nodes)?;
        } else {
            nodes.insert(path, Node::File(iso.read_file(&entry)?));
        }
    }
    Ok(())
}

fn collect_udf(
    udf: &UdfFs<File>,
    directory: &hadris_udf::UdfDir,
    prefix: &str,
    nodes: &mut BTreeMap<String, Node>,
) -> Result<()> {
    for entry in directory.entries().filter(|entry| !entry.is_parent()) {
        let path = join(prefix, entry.name());
        if entry.is_dir() {
            nodes.insert(path.clone(), Node::Directory);
            let child = udf.read_directory(&entry.icb)?;
            collect_udf(udf, &child, &path, nodes)?;
        } else {
            nodes.insert(path, Node::File(udf.read_file(entry)?));
        }
    }
    Ok(())
}

fn clean_iso_name(bytes: &[u8]) -> String {
    let decoded;
    let name = if bytes.len().is_multiple_of(2) && bytes.chunks_exact(2).any(|pair| pair[0] == 0) {
        let utf16 = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        decoded = String::from_utf16_lossy(&utf16);
        std::borrow::Cow::Borrowed(decoded.as_str())
    } else {
        String::from_utf8_lossy(bytes)
    };
    if let Some((base, _)) = name.rsplit_once(';') {
        base.to_string()
    } else {
        name.into_owned()
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
