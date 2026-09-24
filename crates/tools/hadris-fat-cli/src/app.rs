//! Hadris FAT12/16/32 and exFAT analysis and management utility.

#[path = "output.rs"]
mod output;

use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fat::raw::{RawBpb, RawBpbExt16, RawBpbExt32};
use hadris_fat::sync::FatFs;
use hadris_fat::{FatKind, exfat};
use hadris_storage::host::FileDevice;
use output::Output;

use hadris_fs::sync::{DriverExt, FsDriver, extract_to_host, import_from_host};
use hadris_fs::tree::{FromFsOptions, NodeKind, Tree, TreeNode};
use hadris_fs::{
    Attributes, DirCursor, FileType, HeapTable, Metadata, NameBuf, NodeId, OpenOptions, SystemClock,
};

#[derive(Parser)]
#[command(name = "hadris-fat")]
#[command(author, version, about = "FAT12/16/32 and exFAT analysis and management utility", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display volume information
    Info {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
    },
    /// Display detailed filesystem statistics
    Stat {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
    },
    /// List directory contents
    #[command(alias = "list")]
    Ls {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Path within the filesystem (default: root)
        #[arg(default_value = "/")]
        path: String,
        /// Show long format with details
        #[arg(short, long)]
        long: bool,
    },
    /// Display directory tree
    Tree {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Starting path within the filesystem
        #[arg(default_value = "/")]
        path: String,
        /// Maximum depth to display
        #[arg(short, long)]
        depth: Option<usize>,
    },
    /// Analyze filesystem fragmentation
    Fragmentation {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Maximum number of fragmented files to show
        #[arg(short, long, default_value = "10")]
        top: usize,
    },
    /// Check filesystem integrity without changing the image
    #[command(alias = "check")]
    Verify {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Show verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show cluster chain for a file
    Chain {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Path to the file within the filesystem
        file_path: String,
    },
    /// Print a file's contents to stdout
    Cat {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Path to the file within the filesystem
        path: String,
    },
    /// Extract files from an image
    Extract {
        /// Path to the FAT or exFAT image file
        image: PathBuf,
        /// Output directory
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
        /// Path within the filesystem (default: extract all)
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Create a FAT or exFAT image from a host directory
    Create {
        /// Directory containing files to import
        source: PathBuf,
        /// Output image path
        #[arg(short, long)]
        output: PathBuf,
        /// Image size in bytes; calculated automatically when omitted
        #[arg(long)]
        size: Option<u64>,
        /// Filesystem type; a FAT variant is selected by size when omitted
        #[arg(long, value_enum, default_value_t = KindArg::Auto)]
        fat_type: KindArg,
        /// Volume label
        #[arg(short = 'V', long, alias = "volume-name", default_value = "HADRIS")]
        volume_label: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum KindArg {
    #[default]
    Auto,
    Fat12,
    Fat16,
    Fat32,
    Exfat,
}

/// Parse command-line arguments and run the FAT utility.
pub fn run() -> Result<()> {
    reset_sigpipe();
    let cli = Cli::parse();

    match cli.command {
        Commands::Info { image } => cmd_info(&image),
        Commands::Stat { image } => cmd_stat(&image),
        Commands::Ls { image, path, long } => {
            with_fs!(&mut open(&image)?, fs => cmd_ls(fs, &path, long))
        }
        Commands::Tree { image, path, depth } => {
            with_fs!(&mut open(&image)?, fs => cmd_tree(fs, &path, depth))
        }
        Commands::Fragmentation { image, top } => cmd_fragmentation(&image, top),
        Commands::Verify { image, verbose } => cmd_verify(&image, verbose),
        Commands::Chain { image, file_path } => cmd_chain(&image, &file_path),
        Commands::Cat { image, path } => with_fs!(&mut open(&image)?, fs => cmd_cat(fs, &path)),
        Commands::Extract {
            image,
            output,
            path,
        } => with_fs!(&mut open(&image)?, fs => cmd_extract(fs, &output, path.as_deref())),
        Commands::Create {
            source,
            output,
            size,
            fat_type,
            volume_label,
        } => cmd_create(&source, &output, size, fat_type, &volume_label),
    }
}

type Fat = FatFs<FileDevice, HeapTable, SystemClock>;
type ExFat = ExFatFs<FileDevice, HeapTable, SystemClock>;

/// A mounted FAT12/16/32 or exFAT volume.
enum Volume {
    Fat(Box<Fat>),
    ExFat(Box<ExFat>),
}

/// Runs `$body` with `$fs` bound to `&mut` the driver of a `&mut Volume`.
macro_rules! with_fs {
    ($volume:expr, $fs:ident => $body:expr) => {
        match $volume {
            Volume::Fat($fs) => $body,
            Volume::ExFat($fs) => $body,
        }
    };
}
use with_fs;

/// Reads the first sector of an image.
fn boot_sector(path: &Path) -> Result<[u8; 512]> {
    let mut sector = [0u8; 512];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut sector))
        .with_context(|| format!("Failed to read boot sector: {}", path.display()))?;
    Ok(sector)
}

fn is_exfat(sector: &[u8; 512]) -> bool {
    let boot: exfat::raw::BootSector = bytemuck::pod_read_unaligned(sector);
    boot.file_system_name == exfat::raw::FILE_SYSTEM_NAME
}

/// Mounts an image read-only as FAT12/16/32 or exFAT, whichever its boot
/// sector names.
fn open(path: &Path) -> Result<Volume> {
    let exfat = is_exfat(&boot_sector(path)?);
    let file = FileDevice::open(path)
        .with_context(|| format!("Failed to open image file: {}", path.display()))?;
    if exfat {
        let options = exfat::MountOptions::new()
            .with_table(HeapTable::new())
            .with_clock(SystemClock)
            .with_read_only();
        let fs = ExFatFs::open_with(file, options).context("Failed to parse exFAT filesystem")?;
        Ok(Volume::ExFat(Box::new(fs)))
    } else {
        let options = hadris_fat::MountOptions::new()
            .with_table(HeapTable::new())
            .with_clock(SystemClock)
            .with_read_only();
        let fs = FatFs::open_with(file, options).context("Failed to parse FAT filesystem")?;
        Ok(Volume::Fat(Box::new(fs)))
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end().to_string()
}

/// The boot sector fields of a FAT12/16/32 volume that `FatFs` does not
/// expose.
struct FatBoot {
    oem_name: String,
    volume_id: u32,
    label: String,
    fs_type: String,
}

fn fat_boot(sector: &[u8; 512], kind: FatKind) -> FatBoot {
    let (bpb, ext) = sector.split_at(size_of::<RawBpb>());
    let bpb: RawBpb = bytemuck::pod_read_unaligned(bpb);
    let (volume_id, label, fs_type) = if kind == FatKind::Fat32 {
        let ext: RawBpbExt32 = bytemuck::pod_read_unaligned(&ext[..size_of::<RawBpbExt32>()]);
        (ext.volume_id, ext.volume_label, ext.fs_type)
    } else {
        let ext: RawBpbExt16 = bytemuck::pod_read_unaligned(&ext[..size_of::<RawBpbExt16>()]);
        (ext.volume_id, ext.volume_label, ext.fs_type)
    };
    FatBoot {
        oem_name: text(&bpb.oem_name),
        volume_id: u32::from_le_bytes(volume_id),
        label: text(&label),
        fs_type: text(&fs_type),
    }
}

/// The volume label as the root directory stores it, which is what
/// Windows and `mkfs.fat` update. A FAT12/16/32 volume without one falls
/// back to the boot sector copy.
fn volume_label(volume: &mut Volume, sector: &[u8; 512]) -> Result<String> {
    const MESSAGE: &str = "Failed to read root directory from image (image may be truncated)";
    match volume {
        Volume::Fat(fs) => match fs.label().context(MESSAGE)? {
            Some(label) if !label.as_str().is_empty() && label.as_str() != "NO NAME" => {
                Ok(label.as_str().to_string())
            }
            _ => Ok(fat_boot(sector, fs.kind()).label),
        },
        Volume::ExFat(fs) => Ok(fs
            .label()
            .context(MESSAGE)?
            .map(|label| String::from_utf16_lossy(label.as_utf16()))
            .unwrap_or_default()),
    }
}

fn type_name(volume: &Volume) -> String {
    match volume {
        Volume::Fat(fs) => format!("{:?}", fs.kind()),
        Volume::ExFat(_) => "exFAT".to_string(),
    }
}

fn cmd_info(image: &Path) -> Result<()> {
    let sector = boot_sector(image)?;
    let mut volume = open(image)?;
    let label = volume_label(&mut volume, &sector)?;
    let stats = with_fs!(&mut volume, fs => fs.stats()).context("Failed to read the volume")?;

    match &volume {
        Volume::Fat(fs) => {
            let boot = fat_boot(&sector, fs.kind());
            println!("FAT Filesystem Information");
            println!("==========================");
            println!("FAT Type:        {:?}", fs.kind());
            println!("OEM Name:        {}", boot.oem_name);
            println!("Volume Label:    {label}");
            println!("Volume ID:       {:08X}", boot.volume_id);
            println!("FS Type String:  {}", boot.fs_type);
            println!("Cluster Size:    {} bytes", stats.block_size());
        }
        Volume::ExFat(fs) => {
            let boot: exfat::raw::BootSector = bytemuck::pod_read_unaligned(&sector);
            let revision = boot.file_system_revision.get();
            let flags = boot.volume_flags.get();
            println!("exFAT Filesystem Information");
            println!("============================");
            println!("FS Revision:     {}.{:02}", revision >> 8, revision & 0xFF);
            println!("Volume Label:    {label}");
            println!("Volume ID:       {:08X}", fs.volume_id());
            println!(
                "Sector Size:     {} bytes",
                1u32 << boot.bytes_per_sector_shift
            );
            println!("Cluster Size:    {} bytes", fs.cluster_size());
            println!("FAT Count:       {}", boot.number_of_fats);
            println!(
                "Volume Dirty:    {}",
                if flags & exfat::raw::VOLUME_DIRTY != 0 {
                    "yes"
                } else {
                    "no"
                }
            );
        }
    }
    println!("Total Clusters:  {}", stats.total_blocks());
    Ok(())
}

fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

/// The counts a check reports, for either kind of volume.
struct Counts {
    files: u32,
    directories: u32,
    used: u32,
    free: u32,
    bad: u32,
    lost: u32,
    findings: u32,
    clean: bool,
}

/// The [`Counts`] of a `check` or `check_with` result, keeping its error.
macro_rules! counts {
    ($report:expr) => {
        $report.map(|report| Counts {
            files: report.files(),
            directories: report.directories(),
            used: report.used_clusters(),
            free: report.free_clusters(),
            bad: report.bad_clusters(),
            lost: report.lost_clusters(),
            findings: report.findings(),
            clean: report.is_clean(),
        })
    };
}

fn cmd_stat(image: &Path) -> Result<()> {
    let sector = boot_sector(image)?;
    let mut volume = open(image)?;
    let label = volume_label(&mut volume, &sector)?;
    let stats = with_fs!(&mut volume, fs => fs.stats()).context("Failed to gather statistics")?;
    let report = match &mut volume {
        Volume::Fat(fs) => counts!(hadris_fat::sync::check(fs)),
        Volume::ExFat(fs) => counts!(exfat::sync::check(fs)),
    }
    .context("Failed to scan the filesystem")?;
    let cluster_size = u64::from(stats.block_size());
    let total = stats.total_bytes();
    let used = u64::from(report.used) * cluster_size;
    let free = u64::from(report.free) * cluster_size;
    let kind = type_name(&volume);

    println!("{kind} Filesystem Statistics");
    println!("{}", "=".repeat(kind.len() + 22));
    println!("Filesystem Type:     {kind}");
    println!("Volume Label:        {label}");
    println!();
    println!("Cluster Information:");
    println!("  Cluster Size:      {cluster_size} bytes");
    println!("  Total Clusters:    {}", stats.total_blocks());
    println!("  Used Clusters:     {}", report.used);
    println!("  Free Clusters:     {}", report.free);
    println!("  Bad Clusters:      {}", report.bad);
    println!();
    println!("Space Usage:");
    println!(
        "  Total Capacity:    {} ({total} bytes)",
        format_size(total)
    );
    println!(
        "  Used Space:        {} ({:.1}%)",
        format_size(used),
        percent(used, total)
    );
    println!(
        "  Free Space:        {} ({:.1}%)",
        format_size(free),
        percent(free, total)
    );
    println!();
    println!("File System Contents:");
    println!("  Files:             {}", report.files);
    println!("  Directories:       {}", report.directories);

    Ok(())
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The entries of the directory at `path` with their metadata, in directory
/// order.
fn list_dir<D: FsDriver>(fs: &mut D, path: &str) -> Result<Vec<(String, Metadata)>> {
    let mut names = Vec::new();
    for item in fs
        .read_dir(path)
        .with_context(|| format!("Failed to open directory: {path}"))?
    {
        let item = item.context("Failed to read directory entry")?;
        let name = item
            .name_str()
            .context("Directory entry name is not valid UTF-8")?;
        names.push(name.to_string());
    }
    names
        .into_iter()
        .map(|name| {
            let meta = fs
                .metadata(&join(path, &name))
                .with_context(|| format!("Failed to read metadata of {name}"))?;
            Ok((name, meta))
        })
        .collect()
}

fn attribute_flags(attrs: Attributes) -> String {
    [
        (Attributes::READ_ONLY, 'r'),
        (Attributes::HIDDEN, 'h'),
        (Attributes::SYSTEM, 's'),
        (Attributes::ARCHIVE, 'a'),
    ]
    .iter()
    .map(|&(flag, c)| if attrs.contains(flag) { c } else { '-' })
    .collect()
}

fn cmd_ls<D: FsDriver>(fs: &mut D, path: &str, long: bool) -> Result<()> {
    for (name, meta) in list_dir(fs, path)? {
        let is_dir = meta.file_type().is_dir();
        if long {
            println!(
                "{}{} {:>10}  {}",
                if is_dir { 'd' } else { '-' },
                attribute_flags(meta.attributes()),
                if is_dir {
                    "<DIR>".to_string()
                } else {
                    meta.len().to_string()
                },
                name
            );
        } else if is_dir {
            println!("{name}/");
        } else {
            println!("{name}");
        }
    }
    Ok(())
}

fn cmd_tree<D: FsDriver>(fs: &mut D, path: &str, max_depth: Option<usize>) -> Result<()> {
    println!("{path}");
    print_tree(fs, path, "", max_depth, 0)
}

fn print_tree<D: FsDriver>(
    fs: &mut D,
    path: &str,
    prefix: &str,
    max_depth: Option<usize>,
    current_depth: usize,
) -> Result<()> {
    if max_depth.is_some_and(|max| current_depth >= max) {
        return Ok(());
    }

    let entries = list_dir(fs, path)?;
    let count = entries.len();
    for (i, (name, meta)) in entries.into_iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };

        if meta.file_type().is_dir() {
            println!("{prefix}{connector}{name}/");
            let new_prefix = if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            print_tree(
                fs,
                &join(path, &name),
                &new_prefix,
                max_depth,
                current_depth + 1,
            )?;
        } else {
            println!("{prefix}{connector}{name}");
        }
    }
    Ok(())
}

/// Every file below `path`, with its path and size.
fn walk_files<D: FsDriver>(fs: &mut D, path: &str, out: &mut Vec<(String, u64)>) -> Result<()> {
    for (name, meta) in list_dir(fs, path)? {
        let child = join(path, &name);
        match meta.file_type() {
            FileType::Dir => walk_files(fs, &child, out)?,
            _ => out.push((child, meta.len())),
        }
    }
    Ok(())
}

/// The clusters of the file or directory at `path`.
fn cluster_chain(volume: &mut Volume, path: &str) -> Result<Vec<u32>> {
    let mut chain = Vec::new();
    let node = with_fs!(&mut *volume, fs => fs.resolve(path))
        .with_context(|| format!("Failed to open: {path}"))?;
    let result = match volume {
        Volume::Fat(fs) => fs
            .cluster_chain(node, |cluster| chain.push(cluster))
            .map(drop),
        Volume::ExFat(fs) => fs
            .cluster_chain(node, |cluster| chain.push(cluster))
            .map(drop),
    };
    with_fs!(&mut *volume, fs => fs.forget(node));
    result.with_context(|| format!("Failed to read cluster chain of {path}"))?;
    Ok(chain)
}

/// The number of contiguous runs in a chain.
fn count_fragments(chain: &[u32]) -> u32 {
    if chain.is_empty() {
        return 0;
    }
    1 + chain
        .windows(2)
        .filter(|pair| pair[1] != pair[0].wrapping_add(1))
        .count() as u32
}

fn cmd_fragmentation(image: &Path, top: usize) -> Result<()> {
    let mut volume = open(image)?;
    let mut files = Vec::new();
    with_fs!(&mut volume, fs => walk_files(fs, "/", &mut files))
        .context("Failed to analyze fragmentation")?;

    let mut report = Vec::with_capacity(files.len());
    for (path, size) in files {
        let fragments = count_fragments(&cluster_chain(&mut volume, &path)?);
        report.push((fragments, size, path));
    }
    let total_files = report.len() as u32;
    let total_fragments: u32 = report.iter().map(|(fragments, ..)| fragments).sum();
    let fragmented_files = report
        .iter()
        .filter(|(fragments, ..)| *fragments > 1)
        .count() as u32;
    let average = if total_files == 0 {
        0.0
    } else {
        f64::from(total_fragments) / f64::from(total_files)
    };
    report.sort_by(|a, b| b.0.cmp(&a.0));
    let most_fragmented: Vec<_> = report
        .into_iter()
        .filter(|(fragments, ..)| *fragments > 1)
        .take(top)
        .collect();

    println!("Fragmentation Analysis");
    println!("======================");
    println!("Total Files:             {total_files}");
    println!("Fragmented Files:        {fragmented_files}");
    println!(
        "Fragmentation Rate:      {:.1}%",
        percent(u64::from(fragmented_files), u64::from(total_files))
    );
    println!("Average Fragments/File:  {average:.2}");
    println!("Total Fragments:         {total_fragments}");

    if !most_fragmented.is_empty() {
        println!();
        println!("Most Fragmented Files:");
        println!("----------------------");
        for (fragments, size, path) in &most_fragmented {
            println!(
                "  {:>4} fragments  {:>10}  {}",
                fragments,
                format_size(*size),
                path
            );
        }
    }

    Ok(())
}

fn cmd_verify(image: &Path, verbose: bool) -> Result<()> {
    let mut volume = open(image)?;
    let clusters = with_fs!(&mut volume, fs => fs.stats())
        .context("Failed to read the allocation table")?
        .total_blocks()
        + 2;
    let mut bitmap = vec![0u8; clusters.div_ceil(8) as usize];
    let mut findings = Vec::new();
    let report = match &mut volume {
        Volume::Fat(fs) => counts!(hadris_fat::sync::check_with(fs, &mut bitmap, |finding| {
            findings.push(format!("{finding:?}"))
        })),
        Volume::ExFat(fs) => counts!(exfat::sync::check_with(fs, &mut bitmap, |finding| {
            findings.push(format!("{finding:?}"))
        })),
    }
    .context("Failed to verify filesystem")?;

    println!("Filesystem Verification");
    println!("=======================");
    println!("Filesystem Type:     {}", type_name(&volume));
    println!("Files Checked:       {}", report.files);
    println!("Directories Checked: {}", report.directories);
    println!("Clusters In Use:     {}", report.used);
    if verbose {
        println!("Free Clusters:       {}", report.free);
        println!("Bad Clusters:        {}", report.bad);
        println!("Lost Clusters:       {}", report.lost);
    }
    println!();

    if report.clean {
        println!("Result: PASS - No issues found");
        Ok(())
    } else {
        println!("Result: FAIL - {} issue(s) found", report.findings);
        println!();
        println!("Issues:");
        for finding in &findings {
            println!("  - {finding}");
        }
        bail!("{} issue(s) found", report.findings)
    }
}

fn print_chain(chain: &[u32], offset: usize) {
    for (i, cluster) in chain.iter().enumerate() {
        if i > 0 {
            if *cluster != chain[i - 1].wrapping_add(1) {
                print!(" -> [gap] -> ");
            } else {
                print!(" -> ");
            }
        } else if offset > 0 {
            print!("... ");
        }
        print!("{cluster}");
    }
}

fn cmd_chain(image: &Path, file_path: &str) -> Result<()> {
    let mut volume = open(image)?;
    let meta = with_fs!(&mut volume, fs => fs.metadata(file_path))
        .with_context(|| format!("Failed to open: {file_path}"))?;
    let chain = cluster_chain(&mut volume, file_path)?;
    if chain.is_empty() {
        println!("File '{file_path}' has no cluster chain (empty file)");
        return Ok(());
    }

    println!("Cluster chain for: {file_path}");
    println!("File size: {} bytes", meta.len());
    println!("Chain length: {} clusters", chain.len());
    println!();
    println!("Fragments: {}", count_fragments(&chain));
    println!();

    println!("Clusters:");
    if chain.len() <= 20 {
        print_chain(&chain, 0);
    } else {
        print_chain(&chain[..10], 0);
        println!(" ... ({} more) ...", chain.len() - 20);
        print_chain(&chain[chain.len() - 10..], chain.len() - 10);
    }
    println!();

    Ok(())
}

fn cmd_cat<D: FsDriver>(fs: &mut D, path: &str) -> Result<()> {
    let mut file = fs
        .open(path, OpenOptions::read())
        .with_context(|| format!("Failed to open file: {path}"))?;
    let mut stdout = std::io::stdout().lock();
    std::io::copy(&mut file, &mut stdout).context("Failed to write file to stdout")?;
    stdout.flush()?;
    Ok(())
}

/// Extracts `path` (the root when `None`) below `output`. The root is merged
/// into `output`; anything else lands at `output/<stored name>`.
fn cmd_extract<D: FsDriver>(fs: &mut D, output: &Path, path: Option<&str>) -> Result<()> {
    let from = path.unwrap_or("/");
    let destination = match stored_name(fs, from)? {
        None => output.to_path_buf(),
        Some(name) => {
            fs::create_dir_all(output).with_context(|| {
                format!("Failed to create output directory: {}", output.display())
            })?;
            output.join(name)
        }
    };
    extract_to_host(&mut *fs, from, &destination)
        .with_context(|| format!("Failed to extract {from} to {}", destination.display()))
}

/// The name `path` is stored under in its directory, or `None` for the root.
/// Fails on names that are not one plain host path component.
fn stored_name<D: FsDriver>(fs: &mut D, path: &str) -> Result<Option<String>> {
    let node = fs
        .resolve(path)
        .with_context(|| format!("Failed to open: {path}"))?;
    if node == fs.root() {
        fs.forget(node);
        return Ok(None);
    }
    let found = find_name(fs, path, node);
    fs.forget(node);
    let name = found?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        bail!("Refusing to extract {path}: its stored name {name:?} is not a plain file name");
    }
    Ok(Some(name))
}

/// Scans the parent of `path` for the entry listed with id `node`.
fn find_name<D: FsDriver>(fs: &mut D, path: &str, node: NodeId) -> Result<String> {
    let parent = fs
        .resolve(&format!("{path}/.."))
        .with_context(|| format!("Failed to open the parent of {path}"))?;
    let mut cursor = DirCursor::start();
    let mut name = NameBuf::new();
    let found = loop {
        match fs.read_dir_entry(parent, &mut cursor, &mut name) {
            Ok(Some(entry)) if entry.node() == node => break Ok(()),
            Ok(Some(_)) => {}
            Ok(None) => break Err(anyhow::anyhow!("{path} is not listed in its directory")),
            Err(err) => break Err(err).context("Failed to read directory entry"),
        }
    };
    fs.forget(parent);
    found?;
    let name =
        std::str::from_utf8(name.as_bytes()).context("Directory entry name is not valid UTF-8")?;
    Ok(name.to_string())
}

/// Total file bytes and entry count below `node`. Fails on entries FAT and
/// exFAT cannot store.
fn inventory(node: TreeNode<'_>, path: &str, bytes: &mut u64, entries: &mut u64) -> Result<()> {
    for (name, child) in node.children() {
        let child_path = format!("{path}/{name}");
        *entries += 1;
        match child.kind() {
            NodeKind::Dir => inventory(child, &child_path, bytes, entries)?,
            NodeKind::File(content) => {
                *bytes = bytes.saturating_add(content.len().unwrap_or(0));
            }
            NodeKind::Symlink(_) => bail!("Symbolic links are not supported: {child_path}"),
            _ => bail!("Unsupported host entry type: {child_path}"),
        }
    }
    Ok(())
}

fn estimate_image_size(bytes: u64, entries: u64, fat_type: KindArg) -> u64 {
    const MIB: u64 = 1024 * 1024;
    let minimum = match fat_type {
        KindArg::Fat12 => 2 * MIB,
        KindArg::Fat16 => 16 * MIB,
        KindArg::Fat32 => 64 * MIB,
        KindArg::Exfat => 8 * MIB,
        KindArg::Auto => 4 * MIB,
    };
    let estimated = bytes
        .saturating_add(bytes / 2)
        .saturating_add(entries.saturating_mul(4096))
        .saturating_add(2 * MIB);
    estimated.max(minimum).div_ceil(MIB) * MIB
}

/// Formats `file` as `fat_type` and mounts it for writing.
fn format_image(file: FileDevice, fat_type: KindArg, label: &str) -> Result<Volume> {
    if fat_type == KindArg::Exfat {
        let label = exfat::VolumeLabel::new(label)
            .map_err(|kind| anyhow::anyhow!("Invalid volume label {label:?}: {kind}"))?;
        let options = exfat::FormatOptions::new()
            .with_label(label)
            .with_clock(SystemClock);
        let formatted = exfat::sync::format(file, options)?;
        let options = exfat::MountOptions::new()
            .with_table(HeapTable::new())
            .with_clock(SystemClock);
        let fs = ExFatFs::open_with(formatted.into_inner(), options)
            .context("Failed to mount the formatted image")?;
        return Ok(Volume::ExFat(Box::new(fs)));
    }
    let label = hadris_fat::VolumeLabel::new(label)
        .map_err(|kind| anyhow::anyhow!("Invalid volume label {label:?}: {kind}"))?;
    let mut options = hadris_fat::FormatOptions::new()
        .with_label(label)
        .with_clock(SystemClock);
    if let Some(kind) = match fat_type {
        KindArg::Fat12 => Some(FatKind::Fat12),
        KindArg::Fat16 => Some(FatKind::Fat16),
        KindArg::Fat32 => Some(FatKind::Fat32),
        KindArg::Auto | KindArg::Exfat => None,
    } {
        options = options.with_kind(kind);
    }
    let formatted = hadris_fat::sync::format(file, options)?;
    let options = hadris_fat::MountOptions::new()
        .with_table(HeapTable::new())
        .with_clock(SystemClock);
    let fs = FatFs::open_with(formatted.into_inner(), options)
        .context("Failed to mount the formatted image")?;
    Ok(Volume::Fat(Box::new(fs)))
}

fn cmd_create(
    source: &Path,
    output: &Path,
    requested_size: Option<u64>,
    fat_type: KindArg,
    volume_label: &str,
) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to inspect source: {}", source.display()))?;
    if !metadata.is_dir() {
        bail!("Source must be a directory: {}", source.display());
    }
    let tree = Tree::from_fs(source, FromFsOptions::new())
        .with_context(|| format!("Failed to scan source: {}", source.display()))?;
    let (mut bytes, mut entries) = (0, 0);
    inventory(tree.root(), "", &mut bytes, &mut entries)?;

    let image_size =
        requested_size.unwrap_or_else(|| estimate_image_size(bytes, entries, fat_type));
    let (file, pending) = Output::create_new(output)
        .with_context(|| format!("Failed to create image: {}", output.display()))?;
    file.set_len(image_size)
        .with_context(|| format!("Failed to size image to {image_size} bytes"))?;

    let handle = file
        .try_clone()
        .and_then(FileDevice::new)
        .with_context(|| format!("Failed to create image: {}", output.display()))?;
    let mut volume = format_image(handle, fat_type, volume_label).with_context(|| {
        format!(
            "Failed to format {image_size}-byte image; choose a compatible type or increase --size"
        )
    })?;
    let kind = type_name(&volume);
    with_fs!(&mut volume, fs => {
        import_from_host(source, &mut *fs, "/").with_context(
            || "Failed to import source tree; increase --size if the image is out of space",
        )?;
        fs.sync().context("Failed to write the image")?;
    });
    drop(volume);
    pending
        .commit(file)
        .with_context(|| format!("Failed to write image: {}", output.display()))?;
    println!("Created {} ({kind}, {image_size} bytes)", output.display());
    Ok(())
}

/// Format a size in bytes to a human-readable string.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}
