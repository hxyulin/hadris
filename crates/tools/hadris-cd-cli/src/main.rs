use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Parser, Subcommand};
use hadris_cd::{CdOptions, UdfOptions};
use std::hash::Hasher;

use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::{FromFsOptions, OnError, Tree, WarningKind};
use hadris_fs::{FileType, OpenOptions};
use hadris_iso::sync::IsoImage;
use hadris_iso::{
    BootEntry, BootInfo, ElTorito, HybridBoot, JolietLevel, Namespace, Platform, RockRidge,
    VolumeIdentifiers,
};
use hadris_udf::UdfRevision;
use hadris_udf::sync::UdfFs;

mod output;

use output::Output;

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
    #[command(alias = "check")]
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

/// A directory, or a file's length and a hash of its contents.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Directory,
    File(u64, u64),
}

#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() {
    reset_sigpipe();
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
    let tree = Tree::from_fs(
        &args.source,
        FromFsOptions::new().with_on_error(OnError::Warn),
    )?;
    for warning in tree.warnings() {
        eprintln!("warning: {warning}");
    }

    let defaults = CdOptions::default();
    let mut iso = defaults
        .iso()
        .clone()
        .with_volume(VolumeIdentifiers::new(args.volume_name.clone()));
    if args.no_joliet {
        iso = hadris_cd::IsoOptions::default()
            .with_volume(VolumeIdentifiers::new(args.volume_name.clone()))
            .with_level(defaults.iso().level())
            .with_enhanced_tree();
    } else {
        iso = iso.with_joliet(JolietLevel::L3);
    }
    if args.rock_ridge {
        iso = iso.with_rock_ridge(RockRidge::default());
    }
    if let Some(el_torito) = boot_options(&args) {
        iso = iso.with_el_torito(el_torito);
    }
    match (args.hybrid_mbr, args.hybrid_gpt) {
        (true, true) => iso = iso.with_hybrid(HybridBoot::hybrid()),
        (true, false) => iso = iso.with_hybrid(HybridBoot::mbr()),
        (false, true) => iso = iso.with_hybrid(HybridBoot::gpt()),
        (false, false) => {}
    }
    let udf = UdfOptions::default()
        .with_volume_id(args.volume_name.clone())
        .with_revision(args.udf_revision.0);
    let options = CdOptions::default()
        .with_iso(iso)
        .with_udf(udf)
        .with_clock(hadris_fs::SystemClock);

    let (mut file, pending) = Output::create(&args.output)
        .map_err(|err| format!("cannot create {}: {err}", args.output.display()))?;
    let report = hadris_cd::sync::write(&mut file, &tree, &options)?;
    pending
        .commit(file)
        .map_err(|err| format!("cannot write {}: {err}", args.output.display()))?;
    for warning in report.warnings() {
        if warning.kind() != WarningKind::IgnoredMetadata {
            eprintln!("warning: {warning}");
        }
    }
    println!("Created: {}", args.output.display());
    Ok(())
}

fn boot_options(args: &CreateArgs) -> Option<ElTorito> {
    let efi = args
        .efi_boot
        .as_ref()
        .map(|efi| BootEntry::new(normalize(efi)).with_platform(Platform::Efi));
    let Some(bios) = &args.boot else {
        return efi.map(ElTorito::new);
    };
    let mut bios = BootEntry::new(normalize(bios));
    if args.boot_load_size != 0 {
        bios = bios.with_load_size(args.boot_load_size);
    }
    if args.boot_info_table {
        bios = bios.with_boot_info_table(BootInfo::Standard);
    }
    let mut el_torito = ElTorito::new(bios);
    if let Some(efi) = efi {
        el_torito = el_torito.with_entry(efi);
    }
    Some(el_torito)
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

fn info(path: &Path) -> Result<()> {
    let iso = IsoImage::open(File::open(path)?).ok();
    let udf = UdfFs::open(File::open(path)?).ok();
    if iso.is_none() && udf.is_none() {
        return Err("image contains neither a readable ISO 9660 nor UDF filesystem".into());
    }

    println!("Optical image: {}", path.display());
    println!("  ISO 9660: {}", yes_no(iso.is_some()));
    println!("  UDF:      {}", yes_no(udf.is_some()));
    println!("  Bridge:   {}", yes_no(iso.is_some() && udf.is_some()));
    if let Some(mut iso) = iso {
        let pvd = iso.primary_descriptor()?;
        println!(
            "  ISO volume: {}",
            String::from_utf8_lossy(pvd.volume_identifier.trimmed())
        );
        println!("  ISO size:   {} sectors", iso.volume_blocks());
        println!(
            "  Rock Ridge: {}",
            yes_no(iso.namespaces().contains(Namespace::RockRidge))
        );
    }
    if let Some(udf) = udf {
        println!("  UDF volume: {}", udf.volume_id());
        println!("  UDF revision: {}", udf.revision());
    }
    Ok(())
}

fn verify(path: &Path) -> Result<()> {
    let mut iso = IsoImage::open(File::open(path)?)
        .map_err(|error| format!("ISO namespace is not readable: {error}"))?;
    let mut udf = UdfFs::open(File::open(path)?)
        .map_err(|error| format!("UDF namespace is not readable: {error}"))?;

    let mut view = iso.view(Namespace::Preferred)?;
    let namespace = view.namespace();
    let names_match = namespace != Namespace::Primary;
    let mut iso_nodes = BTreeMap::new();
    collect(&mut view, "", &mut iso_nodes)?;
    let mut udf_nodes = BTreeMap::new();
    collect(&mut udf, "", &mut udf_nodes)?;
    if namespace == Namespace::RockRidge {
        for name in relocation_dirs(&mut iso, &iso_nodes, &udf_nodes)? {
            iso_nodes.remove(&name);
        }
    }

    if !names_match {
        println!("  ISO tree has only ISO 9660 identifiers; comparing contents, not names");
        let mut iso_contents: Vec<_> = iso_nodes.into_values().collect();
        let mut udf_contents: Vec<_> = udf_nodes.into_values().collect();
        iso_contents.sort();
        udf_contents.sort();
        if iso_contents != udf_contents {
            return Err("ISO and UDF namespaces differ".into());
        }
        println!(
            "Verified: {} ({} shared entries)",
            path.display(),
            iso_contents.len()
        );
        return Ok(());
    }

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

/// Root directories of the Rock Ridge tree that hold relocated deep
/// directories: absent from UDF, empty in the Rock Ridge view because their
/// children are shown in their real place, and not empty in the primary tree.
fn relocation_dirs(
    iso: &mut IsoImage<File>,
    iso_nodes: &BTreeMap<String, Node>,
    udf_nodes: &BTreeMap<String, Node>,
) -> Result<Vec<String>> {
    let candidates: Vec<&String> = iso_nodes
        .iter()
        .filter(|(path, node)| {
            **node == Node::Directory
                && !path.contains('/')
                && !udf_nodes.contains_key(*path)
                && !iso_nodes.keys().any(|other| {
                    other
                        .strip_prefix(path.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
        })
        .map(|(path, _)| path)
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut primary = iso.view(Namespace::Primary)?;
    let mut found = Vec::new();
    for path in candidates {
        let mut children = primary.read_dir(&format!("/{path}"))?;
        if children.next().is_some() {
            found.push(path.clone());
        }
    }
    Ok(found)
}

fn collect<F: FsDriver>(fs: &mut F, prefix: &str, nodes: &mut BTreeMap<String, Node>) -> Result<()>
where
    F::DeviceError: std::error::Error + Send + Sync + 'static,
{
    let dir = if prefix.is_empty() { "/" } else { prefix };
    let mut entries = Vec::new();
    for item in fs.read_dir(dir)? {
        let item = item?;
        entries.push((
            String::from_utf8_lossy(item.name_bytes()).into_owned(),
            item.file_type(),
        ));
    }
    for (name, file_type) in entries {
        let path = join(prefix, &name);
        match file_type {
            FileType::Dir => {
                nodes.insert(path.clone(), Node::Directory);
                collect(fs, &path, nodes)?;
            }
            FileType::File => {
                let mut file = fs.open(&format!("/{path}"), OpenOptions::read())?;
                let mut hasher = std::hash::DefaultHasher::new();
                let mut buf = vec![0u8; 64 * 1024];
                let mut len = 0u64;
                loop {
                    let n = std::io::Read::read(&mut file, &mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.write(&buf[..n]);
                    len += n as u64;
                }
                nodes.insert(path, Node::File(len, hasher.finish()));
            }
            _ => {}
        }
    }
    Ok(())
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
