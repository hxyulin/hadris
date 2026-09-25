use hadris_fs::MountOptions;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Parser, Subcommand};
use hadris_cd::{CdOptions, UdfOptions};
use std::hash::Hasher;

use hadris_fs::host::{self, OnError, TreeOptions};
use hadris_fs::sync::FileSystem;
use hadris_fs::{Clock, DirCursor, FileType, NodeId, OpenMode, Resolve, SystemClock, WarningKind};
use hadris_iso::sync::IsoFs;
use hadris_iso::{BootEntry, BootInfo, ElTorito, Hybrid, IsoId, Namespace};
use hadris_storage::host::FileDevice;
use hadris_udf::sync::UdfFs;
use hadris_udf::{UdfId, UdfRevision};

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
    let (tree, skipped) = host::read_tree(
        &args.source,
        &TreeOptions::new().with_on_error(OnError::Skip),
    )?;
    for err in skipped {
        eprintln!("warning: skipped {err}");
    }

    let defaults = CdOptions::default();
    let mut iso = defaults
        .iso()
        .clone()
        .with_id(IsoId::Volume, &args.volume_name);
    if args.no_joliet {
        iso = hadris_cd::IsoOptions::default()
            .with_id(IsoId::Volume, &args.volume_name)
            .with_level(defaults.iso().level())
            .with_iso1999();
    } else {
        iso = iso.with_joliet();
    }
    if args.rock_ridge {
        iso = iso.with_rock_ridge();
    }
    if let Some(el_torito) = boot_options(&args) {
        iso = iso.with_el_torito(el_torito);
    }
    match (args.hybrid_mbr, args.hybrid_gpt) {
        (true, true) => iso = iso.with_hybrid(Hybrid::gpt_hybrid_mbr()),
        (true, false) => iso = iso.with_hybrid(Hybrid::mbr()),
        (false, true) => iso = iso.with_hybrid(Hybrid::gpt()),
        (false, false) => {}
    }
    let udf = UdfOptions::default()
        .with_id(UdfId::Volume, &args.volume_name)
        .with_revision(args.udf_revision.0);
    let options = CdOptions::default()
        .with_iso(iso)
        .with_udf(udf)
        .with_time(host::source_date_epoch()?.unwrap_or_else(|| SystemClock.now()));

    let (file, pending) = Output::create(&args.output)
        .map_err(|err| format!("cannot create {}: {err}", args.output.display()))?;
    let mut dev = FileDevice::new(file)
        .map_err(|err| format!("cannot create {}: {err}", args.output.display()))?;
    let report = hadris_cd::sync::write(&mut dev, &tree, &options)?;
    pending
        .commit(dev.into_inner())
        .map_err(|err| format!("cannot write {}: {err}", args.output.display()))?;
    for warning in report.warnings() {
        if !matches!(warning.kind(), WarningKind::Dropped(_)) {
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
        .map(|efi| BootEntry::uefi(&normalize(efi)));
    let Some(bios) = &args.boot else {
        return efi.map(|efi| ElTorito::new().with_entry(efi));
    };
    let mut bios = BootEntry::bios(&normalize(bios));
    if args.boot_load_size != 0 {
        bios = bios.with_load_size(args.boot_load_size);
    }
    if args.boot_info_table {
        bios = bios.with_boot_info(BootInfo::Table);
    }
    let mut el_torito = ElTorito::new().with_entry(bios);
    if let Some(efi) = efi {
        el_torito = el_torito.with_entry(efi);
    }
    Some(el_torito)
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

fn info(path: &Path) -> Result<()> {
    let iso = IsoFs::mount(FileDevice::open(path)?, MountOptions::new()).ok();
    let udf = UdfFs::mount(FileDevice::open(path)?, MountOptions::new()).ok();
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
    let mut iso = FileDevice::open(path)?;
    IsoFs::mount(&mut iso, MountOptions::new())
        .map_err(|error| format!("ISO namespace is not readable: {}", error.error()))?;
    let mut udf = UdfFs::mount(FileDevice::open(path)?, MountOptions::new())
        .map_err(|error| format!("UDF namespace is not readable: {error}"))?;

    let mut view = IsoFs::mount(&mut iso, MountOptions::new()).map_err(|err| err.into_parts().0)?;
    let namespace = view.namespace();
    let names_match = namespace != Namespace::Primary;
    let mut iso_nodes = BTreeMap::new();
    let root = view.root();
    collect(&mut view, root, "", &mut iso_nodes)?;
    let mut udf_nodes = BTreeMap::new();
    let root = udf.root();
    collect(&mut udf, root, "", &mut udf_nodes)?;
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
    iso: &mut FileDevice,
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
    let mut primary = IsoFs::mount_namespace(iso, MountOptions::new(), Namespace::Primary)
        .map_err(|err| err.into_parts().0)?;
    let mut found = Vec::new();
    for path in candidates {
        let dir = primary.resolve(format!("/{path}").as_bytes(), Resolve::Lexical)?;
        let listed = primary.readdir(dir, DirCursor::START);
        primary.forget(dir, 1);
        if listed?.is_some() {
            found.push(path.clone());
        }
    }
    Ok(found)
}

fn collect<F: FileSystem>(
    fs: &mut F,
    dir: NodeId,
    prefix: &str,
    nodes: &mut BTreeMap<String, Node>,
) -> Result<()> {
    let mut cursor = DirCursor::START;
    while let Some(entry) = fs.readdir(dir, cursor)? {
        cursor = entry.next_cursor();
        let path = join(prefix, &String::from_utf8_lossy(entry.name().as_bytes()));
        let node = fs.lookup(dir, entry.name())?;
        let result = match entry.file_type() {
            FileType::Dir => {
                nodes.insert(path.clone(), Node::Directory);
                collect(fs, node, &path, nodes)
            }
            FileType::File => hash_file(fs, node).map(|(len, hash)| {
                nodes.insert(path, Node::File(len, hash));
            }),
            _ => Ok(()),
        };
        fs.forget(node, 1);
        result?;
    }
    Ok(())
}

/// The length and hash of the contents of the pinned file `node`.
fn hash_file<F: FileSystem>(fs: &mut F, node: NodeId) -> Result<(u64, u64)> {
    fs.open(node, OpenMode::Read)?;
    let mut hasher = std::hash::DefaultHasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    let read = loop {
        match fs.read(node, len, &mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                hasher.write(&buf[..n]);
                len += n as u64;
            }
            Err(err) => break Err(err),
        }
    };
    let closed = fs.close(node);
    read?;
    closed?;
    Ok((len, hasher.finish()))
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
