//! Hadris FAT filesystem analysis and management utility.

use std::fs::{self, File, OpenOptions as HostOpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use hadris_fat::raw::{RawBpb, RawBpbExt16, RawBpbExt32};
use hadris_fat::sync::{FatFs, check, check_with, format};
use hadris_fat::{FatKind, FormatOptions, MountOptions, VolumeLabel};
use hadris_fs::sync::{DriverExt, FsDriver, extract_to_host, import_from_host};
use hadris_fs::{
    Attributes, DirCursor, FileType, HeapTable, Metadata, NameBuf, OpenOptions, SystemClock,
};

#[derive(Parser)]
#[command(name = "hadris-fat")]
#[command(author, version, about = "FAT filesystem analysis and management utility", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display volume information
    Info {
        /// Path to the FAT image file
        image: PathBuf,
    },
    /// Display detailed filesystem statistics
    Stat {
        /// Path to the FAT image file
        image: PathBuf,
    },
    /// List directory contents
    Ls {
        /// Path to the FAT image file
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
        /// Path to the FAT image file
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
        /// Path to the FAT image file
        image: PathBuf,
        /// Maximum number of fragmented files to show
        #[arg(short, long, default_value = "10")]
        top: usize,
    },
    /// Verify filesystem integrity
    Verify {
        /// Path to the FAT image file
        image: PathBuf,
        /// Show verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show cluster chain for a file
    Chain {
        /// Path to the FAT image file
        image: PathBuf,
        /// Path to the file within the filesystem
        file_path: String,
    },
    /// Print a file's contents to stdout
    Cat {
        /// Path to the FAT image file
        image: PathBuf,
        /// Path to the file within the filesystem
        path: String,
    },
    /// Extract files from a FAT image
    Extract {
        /// Path to the FAT image file
        image: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
        /// Path within the filesystem (default: extract all)
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Create a FAT image from a host directory
    Create {
        /// Directory containing files to import
        source: PathBuf,
        /// Output image path
        #[arg(short, long)]
        output: PathBuf,
        /// Image size in bytes; calculated automatically when omitted
        #[arg(long)]
        size: Option<u64>,
        /// FAT type; selected automatically when omitted
        #[arg(long, value_enum, default_value_t = KindArg::Auto)]
        fat_type: KindArg,
        /// Volume label
        #[arg(short = 'V', long, default_value = "HADRIS")]
        volume_label: String,
    },
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum KindArg {
    #[default]
    Auto,
    Fat12,
    Fat16,
    Fat32,
}

/// Parse command-line arguments and run the FAT utility.
pub fn run() -> Result<()> {
    reset_sigpipe();
    let cli = Cli::parse();

    match cli.command {
        Commands::Info { image } => cmd_info(image),
        Commands::Stat { image } => cmd_stat(image),
        Commands::Ls { image, path, long } => cmd_ls(image, &path, long),
        Commands::Tree { image, path, depth } => cmd_tree(image, &path, depth),
        Commands::Fragmentation { image, top } => cmd_fragmentation(image, top),
        Commands::Verify { image, verbose } => cmd_verify(image, verbose),
        Commands::Chain { image, file_path } => cmd_chain(image, &file_path),
        Commands::Cat { image, path } => cmd_cat(image, &path),
        Commands::Extract {
            image,
            output,
            path,
        } => cmd_extract(image, &output, path.as_deref()),
        Commands::Create {
            source,
            output,
            size,
            fat_type,
            volume_label,
        } => cmd_create(&source, &output, size, fat_type, &volume_label),
    }
}

type Fs = FatFs<File, HeapTable, SystemClock>;

fn mount_options() -> MountOptions<HeapTable, SystemClock> {
    MountOptions::new()
        .with_table(HeapTable::new())
        .with_clock(SystemClock)
}

/// Mounts an image read-only.
fn open_fat_fs(path: &Path) -> Result<Fs> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open image file: {}", path.display()))?;
    FatFs::open_with(file, mount_options().with_read_only(true))
        .context("Failed to parse FAT filesystem")
}

/// The boot sector fields `FatFs` does not expose.
struct BootInfo {
    oem_name: String,
    volume_id: u32,
    label: String,
    fs_type: String,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end().to_string()
}

fn read_boot_info(path: &Path, kind: FatKind) -> Result<BootInfo> {
    let mut sector = [0u8; 512];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut sector))
        .with_context(|| format!("Failed to read boot sector: {}", path.display()))?;
    let (bpb, ext) = sector.split_at(size_of::<RawBpb>());
    let bpb: RawBpb = bytemuck::pod_read_unaligned(bpb);
    let (volume_id, label, fs_type) = if kind == FatKind::Fat32 {
        let ext: RawBpbExt32 = bytemuck::pod_read_unaligned(&ext[..size_of::<RawBpbExt32>()]);
        (ext.volume_id, ext.volume_label, ext.fs_type)
    } else {
        let ext: RawBpbExt16 = bytemuck::pod_read_unaligned(&ext[..size_of::<RawBpbExt16>()]);
        (ext.volume_id, ext.volume_label, ext.fs_type)
    };
    Ok(BootInfo {
        oem_name: text(&bpb.oem_name),
        volume_id: u32::from_le_bytes(volume_id),
        label: text(&label),
        fs_type: text(&fs_type),
    })
}

/// Prefer the root-directory volume label (what Windows/mkfs.fat update) over
/// the BPB copy, which can drift. Fall back to the BPB label when no root
/// entry exists.
fn display_volume_label(fs: &mut Fs, boot: &BootInfo) -> Result<String> {
    let label = fs
        .label()
        .context("Failed to read root directory from image (image may be truncated)")?;
    match label {
        Some(label) if !label.as_str().is_empty() && label.as_str() != "NO NAME" => {
            Ok(label.as_str().to_string())
        }
        _ => Ok(boot.label.clone()),
    }
}

fn cmd_info(image: PathBuf) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let boot = read_boot_info(&image, fs.kind())?;
    let label = display_volume_label(&mut fs, &boot)?;

    println!("FAT Filesystem Information");
    println!("==========================");
    println!("FAT Type:        {:?}", fs.kind());
    println!("OEM Name:        {}", boot.oem_name);
    println!("Volume Label:    {label}");
    println!("Volume ID:       {:08X}", boot.volume_id);
    println!("FS Type String:  {}", boot.fs_type);

    Ok(())
}

fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

fn cmd_stat(image: PathBuf) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let boot = read_boot_info(&image, fs.kind())?;
    let label = display_volume_label(&mut fs, &boot)?;
    let stats = fs.stats().context("Failed to gather statistics")?;
    let report = check(&mut fs).context("Failed to scan the filesystem")?;
    let cluster_size = u64::from(stats.block_size());
    let total = stats.total_bytes();
    let used = u64::from(report.used_clusters()) * cluster_size;
    let free = u64::from(report.free_clusters()) * cluster_size;

    println!("FAT Filesystem Statistics");
    println!("=========================");
    println!("FAT Type:            {:?}", fs.kind());
    println!("Volume Label:        {label}");
    println!();
    println!("Cluster Information:");
    println!("  Cluster Size:      {cluster_size} bytes");
    println!("  Total Clusters:    {}", stats.total_blocks());
    println!("  Used Clusters:     {}", report.used_clusters());
    println!("  Free Clusters:     {}", report.free_clusters());
    println!("  Bad Clusters:      {}", report.bad_clusters());
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
    println!("  Files:             {}", report.files());
    println!("  Directories:       {}", report.directories());

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
fn list_dir(fs: &mut Fs, path: &str) -> Result<Vec<(String, Metadata)>> {
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

fn cmd_ls(image: PathBuf, path: &str, long: bool) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;

    for (name, meta) in list_dir(&mut fs, path)? {
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

fn cmd_tree(image: PathBuf, path: &str, max_depth: Option<usize>) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    println!("{path}");
    print_tree(&mut fs, path, "", max_depth, 0)
}

fn print_tree(
    fs: &mut Fs,
    path: &str,
    prefix: &str,
    max_depth: Option<usize>,
    current_depth: usize,
) -> Result<()> {
    if let Some(max) = max_depth
        && current_depth >= max
    {
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
fn walk_files(fs: &mut Fs, path: &str, out: &mut Vec<(String, u64)>) -> Result<()> {
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
fn cluster_chain(fs: &mut Fs, path: &str) -> Result<Vec<u32>> {
    let node = fs
        .resolve(path)
        .with_context(|| format!("Failed to open: {path}"))?;
    let mut chain = Vec::new();
    let result = fs.cluster_chain(node, |cluster| chain.push(cluster));
    fs.forget(node);
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

fn cmd_fragmentation(image: PathBuf, top: usize) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let mut files = Vec::new();
    walk_files(&mut fs, "/", &mut files).context("Failed to analyze fragmentation")?;

    let mut report = Vec::with_capacity(files.len());
    for (path, size) in files {
        let fragments = count_fragments(&cluster_chain(&mut fs, &path)?);
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

fn cmd_verify(image: PathBuf, verbose: bool) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let clusters = fs.stats().context("Failed to read the FAT")?.total_blocks() + 2;
    let mut bitmap = vec![0u8; clusters.div_ceil(8) as usize];
    let mut findings = Vec::new();
    let report = check_with(&mut fs, &mut bitmap, |finding| findings.push(finding))
        .context("Failed to verify filesystem")?;

    println!("Filesystem Verification");
    println!("=======================");
    println!("Files Checked:       {}", report.files());
    println!("Directories Checked: {}", report.directories());
    println!("Clusters In Use:     {}", report.used_clusters());
    if verbose {
        println!("Free Clusters:       {}", report.free_clusters());
        println!("Bad Clusters:        {}", report.bad_clusters());
        println!("Lost Clusters:       {}", report.lost_clusters());
    }
    println!();

    if report.is_clean() {
        println!("Result: PASS - No issues found");
    } else {
        println!("Result: FAIL - {} issue(s) found", report.findings());
        println!();
        println!("Issues:");
        for finding in &findings {
            println!("  - {finding:?}");
        }
    }

    Ok(())
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

fn cmd_chain(image: PathBuf, file_path: &str) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let meta = fs
        .metadata(file_path)
        .with_context(|| format!("Failed to open: {file_path}"))?;
    let chain = cluster_chain(&mut fs, file_path)?;
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

fn cmd_cat(image: PathBuf, path: &str) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let mut file = fs
        .open(path, OpenOptions::read())
        .with_context(|| format!("Failed to open file: {path}"))?;
    let mut stdout = std::io::stdout().lock();
    std::io::copy(&mut file, &mut stdout).context("Failed to write file to stdout")?;
    stdout.flush()?;
    Ok(())
}

fn cmd_extract(image: PathBuf, output: &Path, path: Option<&str>) -> Result<()> {
    let mut fs = open_fat_fs(&image)?;
    let from = path.unwrap_or("/");
    let destination = match stored_name(&mut fs, from)? {
        None => output.to_path_buf(),
        Some(name) => {
            fs::create_dir_all(output).with_context(|| {
                format!("Failed to create output directory: {}", output.display())
            })?;
            output.join(name)
        }
    };
    extract_to_host(&mut fs, from, &destination)
        .with_context(|| format!("Failed to extract {from} to {}", destination.display()))
}

/// The name `path` is stored under in its directory, or `None` for the root.
/// Fails on names that are not one plain host path component.
fn stored_name(fs: &mut Fs, path: &str) -> Result<Option<String>> {
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

fn find_name(fs: &mut Fs, path: &str, node: hadris_fs::NodeId) -> Result<String> {
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
    let label = VolumeLabel::new(volume_label)
        .map_err(|kind| anyhow::anyhow!("Invalid volume label {volume_label:?}: {kind}"))?;

    let inventory = inventory_source(source)?;
    let image_size = requested_size.unwrap_or_else(|| estimate_image_size(&inventory, fat_type));
    let file = HostOpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| format!("Failed to create image: {}", output.display()))?;
    file.set_len(image_size)
        .with_context(|| format!("Failed to size image to {image_size} bytes"))?;

    let mut options = FormatOptions::new()
        .with_label(label)
        .with_clock(SystemClock);
    if let Some(kind) = match fat_type {
        KindArg::Auto => None,
        KindArg::Fat12 => Some(FatKind::Fat12),
        KindArg::Fat16 => Some(FatKind::Fat16),
        KindArg::Fat32 => Some(FatKind::Fat32),
    } {
        options = options.with_kind(kind);
    }
    let formatted = format(file, options).with_context(|| {
        format!(
            "Failed to format {image_size}-byte image; choose a compatible FAT type or increase --size"
        )
    })?;
    let kind = formatted.kind();
    let mut fs = FatFs::open_with(formatted.into_inner(), mount_options())
        .context("Failed to mount the formatted image")?;
    import_from_host(source, &mut fs, "/").with_context(
        || "Failed to import source tree; increase --size if the image is out of space",
    )?;
    fs.sync().context("Failed to write the image")?;
    println!(
        "Created {} ({:?}, {} bytes)",
        output.display(),
        kind,
        image_size
    );
    Ok(())
}

#[derive(Default)]
struct SourceInventory {
    bytes: u64,
    entries: u64,
}

fn inventory_source(root: &Path) -> Result<SourceInventory> {
    let mut inventory = SourceInventory::default();
    inventory_directory(root, &mut inventory)?;
    Ok(inventory)
}

fn inventory_directory(directory: &Path, inventory: &mut SourceInventory) -> Result<()> {
    for entry in sorted_host_entries(directory)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        inventory.entries += 1;
        if metadata.file_type().is_symlink() {
            bail!("Symbolic links are not supported: {}", path.display());
        } else if metadata.is_dir() {
            inventory_directory(&path, inventory)?;
        } else if metadata.is_file() {
            inventory.bytes = inventory.bytes.saturating_add(metadata.len());
        } else {
            bail!("Unsupported host entry type: {}", path.display());
        }
    }
    Ok(())
}

fn sorted_host_entries(directory: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("Failed to read directory: {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn estimate_image_size(inventory: &SourceInventory, fat_type: KindArg) -> u64 {
    const MIB: u64 = 1024 * 1024;
    let minimum = match fat_type {
        KindArg::Fat12 => 2 * MIB,
        KindArg::Fat16 => 16 * MIB,
        KindArg::Fat32 => 64 * MIB,
        KindArg::Auto => 4 * MIB,
    };
    let estimated = inventory
        .bytes
        .saturating_add(inventory.bytes / 2)
        .saturating_add(inventory.entries.saturating_mul(4096))
        .saturating_add(2 * MIB);
    estimated.max(minimum).div_ceil(MIB) * MIB
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
