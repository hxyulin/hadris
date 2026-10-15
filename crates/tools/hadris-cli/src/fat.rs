//! `hadris fat`: FAT12/16/32 and exFAT images.

use std::path::{Path, PathBuf};

use crate::common::{Ancestors, Device, Locate, Target, format_size, join, read_source};
use crate::vfs::{self, list_dir, resolve};
use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use hadris::ImageFormat;
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fat::sync::FatFs;
use hadris_fat::{CacheOptions, FatKind, exfat};
use hadris_fat_raw::{RawBpb, RawBpbExt16, RawBpbExt32};
use hadris_storage::host::FileDevice;
use hadris_storage::sync::BlockDevice;

use hadris_fs::TreeEntry;
use hadris_fs::host;
use hadris_fs::sync::{FileSystem, copy_tree};
use hadris_fs::{Clock, FileType, Finding, MountError, MountOptions, SystemClock};

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display volume information
    Info {
        #[command(flatten)]
        image: Image,
    },
    /// Display detailed filesystem statistics
    Stat {
        #[command(flatten)]
        image: Image,
    },
    /// List directory contents
    #[command(alias = "list")]
    Ls {
        #[command(flatten)]
        image: Image,
        /// Path within the filesystem (default: root)
        #[arg(default_value = "/")]
        path: String,
        /// Show long format with details
        #[arg(short, long)]
        long: bool,
    },
    /// Display directory tree
    Tree {
        #[command(flatten)]
        image: Image,
        /// Starting path within the filesystem
        #[arg(default_value = "/")]
        path: String,
        /// Maximum depth to display
        #[arg(short, long)]
        depth: Option<usize>,
    },
    /// Analyze filesystem fragmentation
    Fragmentation {
        #[command(flatten)]
        image: Image,
        /// Maximum number of fragmented files to show
        #[arg(short, long, default_value = "10")]
        top: usize,
    },
    /// Check filesystem integrity without changing the image
    #[command(alias = "check")]
    Verify {
        #[command(flatten)]
        image: Image,
        /// Show verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show cluster chain for a file
    Chain {
        #[command(flatten)]
        image: Image,
        /// Path to the file within the filesystem
        file_path: String,
    },
    /// Print a file's contents to stdout
    Cat {
        #[command(flatten)]
        image: Image,
        /// Path to the file within the filesystem
        path: String,
    },
    /// Extract files from an image
    Extract {
        #[command(flatten)]
        image: Image,
        /// Output directory
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
        /// Path within the filesystem (default: extract all)
        #[arg(short, long)]
        path: Option<String>,
        #[command(flatten)]
        cache: ExtractCache,
    },
    /// Create a FAT or exFAT image from a host directory
    Create {
        /// Directory containing files to import
        source: PathBuf,
        #[command(flatten)]
        target: Target,
        /// Image size in bytes; calculated automatically when omitted
        #[arg(long)]
        size: Option<u64>,
        /// Filesystem type; a FAT variant is selected by size when omitted
        #[arg(long, value_enum, default_value_t = KindArg::Auto)]
        fat_type: KindArg,
        /// Volume label
        #[arg(short = 'V', long, alias = "volume-label", default_value = "HADRIS")]
        volume_name: String,
    },
}

/// The FAT or exFAT volume a command reads.
#[derive(clap::Args)]
struct Image {
    /// Path to the FAT or exFAT image file
    image: PathBuf,
    #[command(flatten)]
    locate: Locate,
    /// Mount read-only from the backup boot sector (FAT32 sector 6, the exFAT
    /// backup boot region) when the primary one is damaged
    #[arg(long)]
    backup_boot: bool,
}

impl Image {
    fn device(&self) -> Result<Device> {
        self.locate.open(&self.image)
    }

    fn options(&self) -> MountOptions {
        let options = MountOptions::new().with_clock(&SystemClock).read_only();
        if self.backup_boot {
            options.backup_boot()
        } else {
            options
        }
    }
}

#[derive(clap::Args)]
struct ExtractCache {
    /// Disable all optional FAT extraction caches
    #[arg(long, conflicts_with_all = ["cache_blocks", "cache_chain_positions", "cache_directory_entries", "no_directory_hint", "read_ahead_blocks"])]
    no_cache: bool,
    /// Total storage read-ahead budget in logical blocks (default: 0)
    #[arg(long)]
    read_ahead_blocks: Option<usize>,
    /// FAT metadata-block cache capacity (default: 0)
    #[arg(long)]
    cache_blocks: Option<usize>,
    /// FAT chain-position cache capacity (default: 0)
    #[arg(long)]
    cache_chain_positions: Option<usize>,
    /// FAT directory-prefix cache capacity (default: 0)
    #[arg(long)]
    cache_directory_entries: Option<usize>,
    /// Disable reuse of the most recently listed FAT entry
    #[arg(long)]
    no_directory_hint: bool,
}

impl ExtractCache {
    fn options(&self) -> CacheOptions {
        if self.no_cache {
            return CacheOptions::sequential().with_directory_hint(false);
        }
        CacheOptions::sequential()
            .with_blocks(self.cache_blocks.unwrap_or(0))
            .with_chain_positions(self.cache_chain_positions.unwrap_or(0))
            .with_directory_entries(self.cache_directory_entries.unwrap_or(0))
            .with_directory_hint(!self.no_directory_hint)
    }

    fn fat_only(&self) -> bool {
        self.cache_blocks.is_some()
            || self.cache_chain_positions.is_some()
            || self.cache_directory_entries.is_some()
            || self.no_directory_hint
    }
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

/// Runs a `hadris fat` command.
pub fn run(cli: Args) -> Result<()> {
    match cli.command {
        Commands::Info { image } => cmd_info(&image),
        Commands::Stat { image } => cmd_stat(&image),
        Commands::Ls { image, path, long } => {
            with_fs!(&mut open(&image)?, fs => vfs::ls(fs, &path, long))
        }
        Commands::Tree { image, path, depth } => {
            with_fs!(&mut open(&image)?, fs => cmd_tree(fs, &path, depth))
        }
        Commands::Fragmentation { image, top } => cmd_fragmentation(&image, top),
        Commands::Verify { image, verbose } => cmd_verify(&image, verbose),
        Commands::Chain { image, file_path } => cmd_chain(&image, &file_path),
        Commands::Cat { image, path } => with_fs!(&mut open(&image)?, fs => vfs::cat(fs, &path)),
        Commands::Extract {
            image,
            output,
            path,
            cache,
        } => {
            let dev = hadris_storage::sync::ReadAhead::new(
                image.device()?,
                cache.read_ahead_blocks.unwrap_or(0),
            );
            match mount(dev, &image)? {
                Volume::ExFat(_) if cache.fat_only() => {
                    bail!("FAT cache settings apply to FAT12/16/32 images, not exFAT")
                }
                Volume::ExFat(fs) => vfs::extract(*fs, &output, path.as_deref()),
                Volume::Fat(fs) => {
                    vfs::extract(fs.with_cache(cache.options()), &output, path.as_deref())
                }
            }
        }
        Commands::Create {
            source,
            target,
            size,
            fat_type,
            volume_name,
        } => cmd_create(&source, &target, size, fat_type, &volume_name),
    }
}

/// A mounted FAT12/16/32 or exFAT volume.
enum Volume<D = Device> {
    Fat(Box<FatFs<D>>),
    ExFat(Box<ExFatFs<D>>),
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

/// Mounts the FAT12/16/32 or exFAT volume [`detect`](hadris::sync::detect)
/// finds on `dev`. With `--backup-boot`, a device whose primary boot sector
/// names no format is tried as exFAT, then as FAT32, from the backup.
fn mount<D: BlockDevice<Error = std::io::Error>>(mut dev: D, image: &Image) -> Result<Volume<D>> {
    let options = image.options();
    let found = hadris::sync::detect(&mut dev)
        .with_context(|| format!("Failed to read image file: {}", image.image.display()))?;
    let format = found
        .iter()
        .map(|candidate| candidate.format())
        .find(|format| matches!(format, ImageFormat::Fat(_) | ImageFormat::ExFat));
    match format {
        Some(ImageFormat::ExFat) => {
            let fs = ExFatFs::mount(dev, options)
                .map_err(MountError::into_error)
                .context("Failed to parse exFAT filesystem")?;
            Ok(Volume::ExFat(Box::new(fs)))
        }
        Some(_) => {
            let fs = FatFs::mount(dev, options)
                .map_err(MountError::into_error)
                .context("Failed to parse FAT filesystem")?;
            Ok(Volume::Fat(Box::new(fs)))
        }
        None if image.backup_boot => {
            let dev = match ExFatFs::mount(dev, options.clone()) {
                Ok(fs) => return Ok(Volume::ExFat(Box::new(fs))),
                Err(err) => err.into_parts().1,
            };
            let fs = FatFs::mount(dev, options)
                .map_err(MountError::into_error)
                .context("No valid exFAT or FAT32 backup boot sector")?;
            Ok(Volume::Fat(Box::new(fs)))
        }
        None if found
            .iter()
            .any(|c| matches!(c.format(), ImageFormat::Mbr | ImageFormat::Gpt)) =>
        {
            bail!(
                "No FAT or exFAT volume in {}: it holds a partition table; select a partition with --partition",
                image.image.display()
            )
        }
        None => bail!("No FAT or exFAT volume in {}", image.image.display()),
    }
}

/// Mounts an image read-only as FAT12/16/32 or exFAT, whichever
/// [`detect`](hadris::sync::detect) finds.
fn open(image: &Image) -> Result<Volume> {
    mount(image.device()?, image)
}

/// The boot sector the volume was mounted from.
fn boot_sector(volume: &mut Volume, image: &Image) -> Result<[u8; 512]> {
    let at = match &*volume {
        Volume::Fat(fs) if image.backup_boot && fs.info().kind() == FatKind::Fat32 => {
            6 * u64::from(fs.info().sector_size())
        }
        Volume::ExFat(fs) if image.backup_boot => 12 * fs.info().sector_size(),
        _ => 0,
    };
    let mut sector = [0u8; 512];
    with_fs!(volume, fs => fs.read_raw(at, &mut sector)).context("Failed to read boot sector")?;
    Ok(sector)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end().to_string()
}

/// The boot sector fields of a FAT12/16/32 volume that `FatFs` does not
/// expose.
struct FatBoot {
    oem_name: String,
    label: String,
    fs_type: String,
}

fn fat_boot(sector: &[u8; 512], kind: FatKind) -> FatBoot {
    let (bpb, ext) = sector.split_at(size_of::<RawBpb>());
    let bpb: RawBpb = bytemuck::pod_read_unaligned(bpb);
    let (label, fs_type) = if kind == FatKind::Fat32 {
        let ext: RawBpbExt32 = bytemuck::pod_read_unaligned(&ext[..size_of::<RawBpbExt32>()]);
        (ext.volume_label, ext.fs_type)
    } else {
        let ext: RawBpbExt16 = bytemuck::pod_read_unaligned(&ext[..size_of::<RawBpbExt16>()]);
        (ext.volume_label, ext.fs_type)
    };
    FatBoot {
        oem_name: text(&bpb.oem_name),
        label: text(&label),
        fs_type: text(&fs_type),
    }
}

/// The volume label as the root directory stores it, which is what
/// Windows and `mkfs.fat` update. A FAT12/16/32 volume without one falls
/// back to the boot sector copy.
fn volume_label(volume: &mut Volume, sector: &[u8; 512]) -> Result<String> {
    const MESSAGE: &str = "Failed to read root directory from image (image may be truncated)";
    let mut buf = [0u8; 64];
    let label =
        with_fs!(&mut *volume, fs => fs.label(&mut buf).map(|label| label.map(str::to_string)))
            .context(MESSAGE)?;
    match volume {
        Volume::Fat(fs) => match label {
            Some(label) if !label.is_empty() && label != "NO NAME" => Ok(label),
            _ => Ok(fat_boot(sector, fs.info().kind()).label),
        },
        Volume::ExFat(_) => Ok(label.unwrap_or_default()),
    }
}

fn type_name<D: BlockDevice>(volume: &Volume<D>) -> String {
    match volume {
        Volume::Fat(fs) => format!("{:?}", fs.info().kind()),
        Volume::ExFat(_) => "exFAT".to_string(),
    }
}

fn cmd_info(image: &Image) -> Result<()> {
    let mut volume = open(image)?;
    let sector = boot_sector(&mut volume, image)?;
    let label = volume_label(&mut volume, &sector)?;
    let stats = with_fs!(&mut volume, fs => fs.statfs()).context("Failed to read the volume")?;

    match &volume {
        Volume::Fat(fs) => {
            let boot = fat_boot(&sector, fs.info().kind());
            println!("FAT Filesystem Information");
            println!("==========================");
            println!("FAT Type:        {:?}", fs.info().kind());
            println!("OEM Name:        {}", boot.oem_name);
            println!("Volume Label:    {label}");
            match fs.volume_serial() {
                Some(serial) => println!("Volume ID:       {serial:08X}"),
                None => println!("Volume ID:       none"),
            }
            println!("FS Type String:  {}", boot.fs_type);
            println!("Cluster Size:    {} bytes", stats.block_size());
        }
        Volume::ExFat(fs) => {
            let boot: hadris_fat_raw::exfat::BootSector = bytemuck::pod_read_unaligned(&sector);
            let revision = boot.file_system_revision.get();
            let flags = boot.volume_flags.get();
            println!("exFAT Filesystem Information");
            println!("============================");
            println!("FS Revision:     {}.{:02}", revision >> 8, revision & 0xFF);
            println!("Volume Label:    {label}");
            println!("Volume ID:       {:08X}", fs.volume_serial());
            println!(
                "Sector Size:     {} bytes",
                1u32 << boot.bytes_per_sector_shift
            );
            println!("Cluster Size:    {} bytes", fs.info().cluster_size());
            println!("FAT Count:       {}", boot.number_of_fats);
            println!(
                "Volume Dirty:    {}",
                if flags & hadris_fat_raw::exfat::VOLUME_DIRTY != 0 {
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

/// Files and directories below `path`, counting `path` as a directory.
fn count_tree<D: FileSystem>(
    fs: &mut D,
    path: &str,
    ancestors: &mut Ancestors,
    files: &mut u64,
    dirs: &mut u64,
) -> Result<()> {
    *dirs += 1;
    for (name, meta, node) in list_dir(fs, path)? {
        match meta.file_type() {
            FileType::Dir => {
                let child = join(path, &name);
                ancestors.enter(node, &child)?;
                count_tree(fs, &child, ancestors, files, dirs)?;
                ancestors.leave();
            }
            _ => *files += 1,
        }
    }
    Ok(())
}

/// The directories a recursive walk starts from: the node at `path`.
fn walk_root<D: FileSystem>(fs: &mut D, path: &str) -> Result<Ancestors> {
    let node = resolve(fs, path)?;
    fs.forget(node, 1);
    Ok(Ancestors::new(node))
}

/// Checks the image with the checker of its kind, and returns each finding
/// as text.
fn check_image(image: &Image, volume: &Volume, clusters: u64) -> Result<Vec<String>> {
    let mut dev = image.device()?;
    let mut scratch = vec![0u8; 1024 + (clusters.div_ceil(8) as usize).max(512)];
    let mut findings = Vec::new();
    let mut note = |finding: &Finding<'_>| {
        findings.push(format!("{finding} [{:?}]", finding.severity()));
    };
    match volume {
        Volume::Fat(_) => hadris_fat::sync::check(&mut dev, &mut scratch, &mut note)?,
        Volume::ExFat(_) => exfat::sync::check(&mut dev, &mut scratch, &mut note)?,
    };
    Ok(findings)
}

fn cmd_stat(image: &Image) -> Result<()> {
    let mut volume = open(image)?;
    let sector = boot_sector(&mut volume, image)?;
    let label = volume_label(&mut volume, &sector)?;
    let stats = with_fs!(&mut volume, fs => fs.statfs()).context("Failed to gather statistics")?;
    let (mut files, mut directories) = (0, 0);
    with_fs!(&mut volume, fs => walk_root(fs, "/").and_then(|mut ancestors| count_tree(fs, "/", &mut ancestors, &mut files, &mut directories)))
        .context("Failed to scan the filesystem")?;
    let cluster_size = u64::from(stats.block_size());
    let total = stats.total_bytes();
    let used = stats.used_blocks() * cluster_size;
    let free = stats.free_blocks() * cluster_size;
    let kind = type_name(&volume);

    println!("{kind} Filesystem Statistics");
    println!("{}", "=".repeat(kind.len() + 22));
    println!("Filesystem Type:     {kind}");
    println!("Volume Label:        {label}");
    println!();
    println!("Cluster Information:");
    println!("  Cluster Size:      {cluster_size} bytes");
    println!("  Total Clusters:    {}", stats.total_blocks());
    println!("  Used Clusters:     {}", stats.used_blocks());
    println!("  Free Clusters:     {}", stats.free_blocks());
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
    println!("  Files:             {files}");
    println!("  Directories:       {directories}");

    Ok(())
}

fn cmd_tree<D: FileSystem>(fs: &mut D, path: &str, max_depth: Option<usize>) -> Result<()> {
    println!("{path}");
    let mut ancestors = walk_root(fs, path)?;
    print_tree(fs, path, &mut ancestors, "", max_depth, 0)
}

fn print_tree<D: FileSystem>(
    fs: &mut D,
    path: &str,
    ancestors: &mut Ancestors,
    prefix: &str,
    max_depth: Option<usize>,
    current_depth: usize,
) -> Result<()> {
    if max_depth.is_some_and(|max| current_depth >= max) {
        return Ok(());
    }

    let entries = list_dir(fs, path)?;
    let count = entries.len();
    for (i, (name, meta, node)) in entries.into_iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };

        if meta.file_type().is_dir() {
            println!("{prefix}{connector}{name}/");
            let new_prefix = if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            let child = join(path, &name);
            ancestors.enter(node, &child)?;
            print_tree(
                fs,
                &child,
                ancestors,
                &new_prefix,
                max_depth,
                current_depth + 1,
            )?;
            ancestors.leave();
        } else {
            println!("{prefix}{connector}{name}");
        }
    }
    Ok(())
}

/// Every file below `path`, with its path and size.
fn walk_files<D: FileSystem>(
    fs: &mut D,
    path: &str,
    ancestors: &mut Ancestors,
    out: &mut Vec<(String, u64)>,
) -> Result<()> {
    for (name, meta, node) in list_dir(fs, path)? {
        let child = join(path, &name);
        match meta.file_type() {
            FileType::Dir => {
                ancestors.enter(node, &child)?;
                walk_files(fs, &child, ancestors, out)?;
                ancestors.leave();
            }
            _ => out.push((child, meta.len())),
        }
    }
    Ok(())
}

/// The clusters of the file or directory at `path`, from its extents.
fn cluster_chain(volume: &mut Volume, path: &str) -> Result<Vec<u32>> {
    let node = with_fs!(&mut *volume, fs => resolve(&mut **fs, path))?;
    let (heap, size) = match &*volume {
        Volume::Fat(fs) => (fs.info().data_start(), fs.info().cluster_size() as u64),
        Volume::ExFat(fs) => (fs.info().heap_start(), fs.info().cluster_size()),
    };
    let mut chain: Vec<u32> = Vec::new();
    let mut out = [hadris_fs::Extent::new(0, 0); 16];
    let mut from = 0;
    let mut outside = false;
    let result = 'extents: loop {
        let n = match with_fs!(&mut *volume, fs => fs.extents(node, from, &mut out)) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(err) => break Err(err),
        };
        for extent in &out[..n] {
            if extent.offset() < heap {
                outside = true;
                break 'extents Ok(());
            }
            let first = (extent.offset() - heap) / size + 2;
            let last = (extent.end() - 1 - heap) / size + 2;
            for cluster in first..=last {
                if chain.last() != Some(&(cluster as u32)) {
                    chain.push(cluster as u32);
                }
            }
            from = extent.file_offset() + extent.len();
        }
    };
    with_fs!(&mut *volume, fs => fs.forget(node, 1));
    result.with_context(|| format!("Failed to read cluster chain of {path}"))?;
    if outside {
        bail!("{path} is the fixed FAT12/16 root directory, which has no cluster chain");
    }
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

fn cmd_fragmentation(image: &Image, top: usize) -> Result<()> {
    let mut volume = open(image)?;
    let mut files = Vec::new();
    with_fs!(&mut volume, fs => walk_root(fs, "/").and_then(|mut ancestors| walk_files(fs, "/", &mut ancestors, &mut files)))
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

fn cmd_verify(image: &Image, verbose: bool) -> Result<()> {
    let mut volume = open(image)?;
    let stats =
        with_fs!(&mut volume, fs => fs.statfs()).context("Failed to read the allocation table")?;
    let findings = check_image(image, &volume, stats.total_blocks() + 2)
        .context("Failed to verify filesystem")?;

    println!("Filesystem Verification");
    println!("=======================");
    println!("Filesystem Type:     {}", type_name(&volume));
    println!("Clusters In Use:     {}", stats.used_blocks());
    if verbose {
        println!("Free Clusters:       {}", stats.free_blocks());
    }
    println!();

    if findings.is_empty() {
        println!("Result: PASS - No issues found");
        Ok(())
    } else {
        println!("Result: FAIL - {} issue(s) found", findings.len());
        println!();
        println!("Issues:");
        for finding in &findings {
            println!("  - {finding}");
        }
        bail!("{} issue(s) found", findings.len())
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

fn cmd_chain(image: &Image, file_path: &str) -> Result<()> {
    let mut volume = open(image)?;
    let meta = with_fs!(&mut volume, fs => {
        let node = resolve(&mut **fs, file_path)?;
        let meta = fs.stat(node);
        fs.forget(node, 1);
        meta
    })
    .with_context(|| format!("Failed to read metadata of {file_path}"))?;
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

/// Total file bytes and entry count below `node`. Fails on entries FAT and
/// exFAT cannot store.
fn inventory(node: TreeEntry<'_>, path: &str, bytes: &mut u64, entries: &mut u64) -> Result<()> {
    for (name, child) in node.children() {
        let child_path = format!("{path}/{name}");
        *entries += 1;
        match child.node().file_type() {
            FileType::Dir => inventory(child, &child_path, bytes, entries)?,
            FileType::File => {
                let len = child.node().content().map_or(0, |content| content.len());
                *bytes = bytes.saturating_add(len);
            }
            FileType::Symlink => bail!("Symbolic links are not supported: {child_path}"),
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

/// Formats `file` as `fat_type`, stamped with `SOURCE_DATE_EPOCH` or the
/// current time, and mounts it for writing.
fn format_image(
    mut file: FileDevice,
    fat_type: KindArg,
    label: &str,
) -> Result<Volume<FileDevice>> {
    let time = host::source_date_epoch()?.unwrap_or_else(|| SystemClock.now());
    let mount = MountOptions::new().with_clock(&SystemClock);
    if fat_type == KindArg::Exfat {
        let label = exfat::VolumeLabel::new(label)
            .map_err(|kind| anyhow::anyhow!("Invalid volume label {label:?}: {kind}"))?;
        let options = exfat::ExFatOptions::new().with_label(label).with_time(time);
        exfat::sync::format(&mut file, &options)?;
        let fs = ExFatFs::mount(file, mount).context("Failed to mount the formatted image")?;
        return Ok(Volume::ExFat(Box::new(fs)));
    }
    let label = hadris_fat::VolumeLabel::new(label)
        .map_err(|kind| anyhow::anyhow!("Invalid volume label {label:?}: {kind}"))?;
    let mut options = hadris_fat::FatOptions::new()
        .with_label(label)
        .with_time(time);
    if let Some(kind) = match fat_type {
        KindArg::Fat12 => Some(FatKind::Fat12),
        KindArg::Fat16 => Some(FatKind::Fat16),
        KindArg::Fat32 => Some(FatKind::Fat32),
        KindArg::Auto | KindArg::Exfat => None,
    } {
        options = options.with_kind(kind);
    }
    hadris_fat::sync::format(&mut file, &options)?;
    let fs = FatFs::mount(file, mount).context("Failed to mount the formatted image")?;
    Ok(Volume::Fat(Box::new(fs)))
}

fn cmd_create(
    source: &Path,
    target: &Target,
    requested_size: Option<u64>,
    fat_type: KindArg,
    volume_label: &str,
) -> Result<()> {
    let output = &target.output;
    let tree = read_source(source)?;
    let (mut bytes, mut entries) = (0, 0);
    inventory(tree.root(), "", &mut bytes, &mut entries)?;

    let image_size =
        requested_size.unwrap_or_else(|| estimate_image_size(bytes, entries, fat_type));
    let (file, pending) = target.create()?;
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
        let root = fs.root();
        copy_tree(&tree, &mut *fs, root).with_context(
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
