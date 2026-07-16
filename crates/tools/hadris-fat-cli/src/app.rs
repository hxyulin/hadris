//! Hadris FAT filesystem analysis and management utility.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as StdRead, Write as StdWrite};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::{DirEntryAttrFlags, DirectoryEntry, FatDir};
use hadris_fat::{FatAnalysisExt, FatFs, FatFsWriteExt, FatVerifyExt, Read as FatRead};

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
        #[arg(long, value_enum, default_value_t = FatKind::Auto)]
        fat_type: FatKind,
        /// Volume label
        #[arg(short = 'V', long, default_value = "HADRIS")]
        volume_label: String,
    },
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum FatKind {
    #[default]
    Auto,
    Fat12,
    Fat16,
    Fat32,
}

/// Parse command-line arguments and run the FAT utility.
pub fn run() -> Result<()> {
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

fn open_fat_fs(path: PathBuf) -> Result<FatFs<File>> {
    let file = File::open(&path)
        .with_context(|| format!("Failed to open image file: {}", path.display()))?;
    FatFs::open(file).context("Failed to parse FAT filesystem")
}

/// Prefer the root-directory volume label (what Windows/mkfs.fat update) over
/// the BPB copy, which can drift. Fall back to the BPB label when no root
/// entry exists.
fn display_volume_label(fs: &FatFs<File>) -> Result<String> {
    if let Some(raw) = fs
        .read_root_label()
        .context("Failed to read root volume label")?
    {
        let label = core::str::from_utf8(&raw)
            .unwrap_or("")
            .trim_end()
            .to_string();
        if !label.is_empty() && label != "NO NAME" {
            return Ok(label);
        }
    }
    Ok(fs.volume_info().volume_label().to_string())
}

fn cmd_info(image: PathBuf) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let vol = fs.volume_info();
    let label = display_volume_label(&fs)?;

    println!("FAT Filesystem Information");
    println!("==========================");
    println!("FAT Type:        {:?}", fs.fat_type());
    println!("OEM Name:        {}", vol.oem_name());
    println!("Volume Label:    {label}");
    println!("Volume ID:       {:08X}", vol.volume_id());
    println!("FS Type String:  {}", vol.fs_type_str());

    Ok(())
}

fn cmd_stat(image: PathBuf) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let stats = fs.statistics().context("Failed to gather statistics")?;
    let label = display_volume_label(&fs)?;

    println!("FAT Filesystem Statistics");
    println!("=========================");
    println!("FAT Type:            {:?}", stats.fat_type);
    println!("Volume Label:        {label}");
    println!();
    println!("Cluster Information:");
    println!("  Cluster Size:      {} bytes", stats.cluster_size);
    println!("  Total Clusters:    {}", stats.total_clusters);
    println!("  Used Clusters:     {}", stats.used_clusters);
    println!("  Free Clusters:     {}", stats.free_clusters);
    println!("  Bad Clusters:      {}", stats.bad_clusters);
    println!("  Reserved:          {}", stats.reserved_clusters);
    println!();
    println!("Space Usage:");
    println!(
        "  Total Capacity:    {} ({} bytes)",
        format_size(stats.total_capacity),
        stats.total_capacity
    );
    println!(
        "  Used Space:        {} ({:.1}%)",
        format_size(stats.used_space),
        stats.used_percentage()
    );
    println!(
        "  Free Space:        {} ({:.1}%)",
        format_size(stats.free_space),
        stats.free_percentage()
    );
    println!();
    println!("File System Contents:");
    println!("  Files:             {}", stats.file_count);
    println!("  Directories:       {}", stats.directory_count);

    Ok(())
}

fn cmd_ls(image: PathBuf, path: &str, long: bool) -> Result<()> {
    let fs = open_fat_fs(image)?;

    let dir = if path == "/" {
        fs.root_dir()
    } else {
        fs.open_dir_path(path)
            .with_context(|| format!("Failed to open directory: {path}"))?
    };

    for entry in dir.entries() {
        let entry = entry.context("Failed to read directory entry")?;
        let DirectoryEntry::Entry(file_entry) = entry;

        let name = file_entry.name();
        if name == "." || name == ".." {
            continue;
        }

        if long {
            let type_char = if file_entry.is_directory() { 'd' } else { '-' };
            let attrs = file_entry.attributes();
            let r = if attrs.contains(DirEntryAttrFlags::READ_ONLY) {
                'r'
            } else {
                '-'
            };
            let h = if attrs.contains(DirEntryAttrFlags::HIDDEN) {
                'h'
            } else {
                '-'
            };
            let s = if attrs.contains(DirEntryAttrFlags::SYSTEM) {
                's'
            } else {
                '-'
            };
            let a = if attrs.contains(DirEntryAttrFlags::ARCHIVE) {
                'a'
            } else {
                '-'
            };

            println!(
                "{}{}{}{}{} {:>10}  {}",
                type_char,
                r,
                h,
                s,
                a,
                if file_entry.is_directory() {
                    "<DIR>".to_string()
                } else {
                    file_entry.len().to_string()
                },
                name
            );
        } else if file_entry.is_directory() {
            println!("{name}/");
        } else {
            println!("{name}");
        }
    }

    Ok(())
}

fn cmd_tree(image: PathBuf, path: &str, max_depth: Option<usize>) -> Result<()> {
    let fs = open_fat_fs(image)?;

    let dir = if path == "/" {
        fs.root_dir()
    } else {
        fs.open_dir_path(path)
            .with_context(|| format!("Failed to open directory: {path}"))?
    };

    println!("{path}");
    print_tree(&fs, &dir, "", max_depth, 0)?;

    Ok(())
}

fn print_tree<DATA: std::io::Read + std::io::Seek>(
    fs: &FatFs<DATA>,
    dir: &FatDir<'_, DATA>,
    prefix: &str,
    max_depth: Option<usize>,
    current_depth: usize,
) -> Result<()> {
    if let Some(max) = max_depth
        && current_depth >= max
    {
        return Ok(());
    }

    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let DirectoryEntry::Entry(fe) = e;
            let name = fe.name().to_string();
            if name == "." || name == ".." {
                None
            } else {
                Some(fe)
            }
        })
        .collect();

    let count = entries.len();
    for (i, entry) in entries.into_iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let name = entry.name();

        if entry.is_directory() {
            println!("{prefix}{connector}{name}/");
            let new_prefix = if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };

            let subdir = fs.open_dir_entry(&entry)?;
            print_tree(fs, &subdir, &new_prefix, max_depth, current_depth + 1)?;
        } else {
            println!("{prefix}{connector}{name}");
        }
    }

    Ok(())
}

fn cmd_fragmentation(image: PathBuf, top: usize) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let report = fs
        .fragmentation_report(top)
        .context("Failed to analyze fragmentation")?;

    println!("Fragmentation Analysis");
    println!("======================");
    println!("Total Files:             {}", report.total_files);
    println!("Fragmented Files:        {}", report.fragmented_files);
    println!(
        "Fragmentation Rate:      {:.1}%",
        report.fragmentation_percentage
    );
    println!("Average Fragments/File:  {:.2}", report.average_fragments);
    println!("Total Fragments:         {}", report.total_fragments);

    if !report.most_fragmented.is_empty() {
        println!();
        println!("Most Fragmented Files:");
        println!("----------------------");
        for file in &report.most_fragmented {
            println!(
                "  {:>4} fragments  {:>10}  {}",
                file.fragments,
                format_size(file.size as u64),
                file.path
            );
        }
    }

    Ok(())
}

fn cmd_verify(image: PathBuf, verbose: bool) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let report = fs.verify().context("Failed to verify filesystem")?;

    println!("Filesystem Verification");
    println!("=======================");
    println!("Files Checked:       {}", report.files_checked);
    println!("Directories Checked: {}", report.directories_checked);
    println!("Clusters Verified:   {}", report.clusters_verified);
    println!();

    if report.is_valid() {
        println!("Result: PASS - No issues found");
    } else {
        println!("Result: FAIL - {} issue(s) found", report.issue_count());
        println!();
        println!("Issues:");
        for issue in &report.issues {
            println!("  - {issue}");
            if verbose {
                // Additional details could be printed here
            }
        }
    }

    Ok(())
}

fn cmd_chain(image: PathBuf, file_path: &str) -> Result<()> {
    let fs = open_fat_fs(image)?;

    let entry = fs
        .open_path(file_path)
        .with_context(|| format!("Failed to open: {file_path}"))?;

    let first_cluster = entry.cluster().0 as u32;
    if first_cluster < 2 {
        println!("File '{file_path}' has no cluster chain (empty file)");
        return Ok(());
    }

    let chain = fs
        .get_cluster_chain(first_cluster)
        .context("Failed to read cluster chain")?;

    println!("Cluster chain for: {file_path}");
    println!("File size: {} bytes", entry.len());
    println!("Chain length: {} clusters", chain.len());
    println!();

    // Count fragments
    let mut fragments = 1;
    for window in chain.windows(2) {
        if window[1] != window[0] + 1 {
            fragments += 1;
        }
    }
    println!("Fragments: {fragments}");
    println!();

    // Print chain (abbreviated if very long)
    println!("Clusters:");
    if chain.len() <= 20 {
        for (i, cluster) in chain.iter().enumerate() {
            if i > 0 {
                let prev = chain[i - 1];
                if *cluster != prev + 1 {
                    print!(" -> [gap] -> ");
                } else {
                    print!(" -> ");
                }
            }
            print!("{cluster}");
        }
        println!();
    } else {
        // Show first 10, ..., last 10
        for (i, cluster) in chain[..10].iter().enumerate() {
            if i > 0 {
                let prev = chain[i - 1];
                if *cluster != prev + 1 {
                    print!(" -> [gap] -> ");
                } else {
                    print!(" -> ");
                }
            }
            print!("{cluster}");
        }
        println!(" ... ({} more) ...", chain.len() - 20);
        for (i, cluster) in chain[chain.len() - 10..].iter().enumerate() {
            if i > 0 {
                let prev = chain[chain.len() - 11 + i];
                if *cluster != prev + 1 {
                    print!(" -> [gap] -> ");
                } else {
                    print!(" -> ");
                }
            } else {
                print!("... ");
            }
            print!("{cluster}");
        }
        println!();
    }

    Ok(())
}

fn cmd_cat(image: PathBuf, path: &str) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let mut reader = fs
        .open_file_path(path)
        .with_context(|| format!("Failed to open file: {path}"))?;
    let mut stdout = std::io::stdout().lock();
    copy_from_fat(&mut reader, &mut stdout).context("Failed to write file to stdout")?;
    Ok(())
}

fn cmd_extract(image: PathBuf, output: &Path, path: Option<&str>) -> Result<()> {
    let fs = open_fat_fs(image)?;
    fs::create_dir_all(output)
        .with_context(|| format!("Failed to create output directory: {}", output.display()))?;

    match path {
        None | Some("/") => extract_dir(&fs, &fs.root_dir(), output),
        Some(path) => {
            let entry = fs
                .open_path(path)
                .with_context(|| format!("Failed to open: {path}"))?;
            let destination = output.join(entry.name().as_ref());
            if entry.is_directory() {
                fs::create_dir_all(&destination)?;
                let dir = fs.open_dir_entry(&entry)?;
                extract_dir(&fs, &dir, &destination)
            } else {
                extract_file(&fs, &entry, &destination)
            }
        }
    }
}

fn extract_dir<DATA: FatRead + hadris_fat::Seek>(
    fs: &FatFs<DATA>,
    dir: &FatDir<'_, DATA>,
    destination: &Path,
) -> Result<()> {
    for entry in dir.entries() {
        let DirectoryEntry::Entry(entry) = entry.context("Failed to read directory entry")?;
        let name = entry.name();
        if name == "." || name == ".." {
            continue;
        }
        if name.contains(['/', '\\']) {
            bail!("Refusing unsafe FAT entry name: {name}");
        }
        let path = destination.join(name.as_ref());
        if entry.is_directory() {
            fs::create_dir_all(&path)
                .with_context(|| format!("Failed to create directory: {}", path.display()))?;
            let child = fs.open_dir_entry(&entry)?;
            extract_dir(fs, &child, &path)?;
        } else {
            extract_file(fs, &entry, &path)?;
        }
    }
    Ok(())
}

fn extract_file<DATA: FatRead + hadris_fat::Seek>(
    fs: &FatFs<DATA>,
    entry: &hadris_fat::FileEntry,
    destination: &Path,
) -> Result<()> {
    let mut reader = hadris_fat::read::FileReader::new(fs, entry)?;
    let mut output = File::create(destination)
        .with_context(|| format!("Failed to create: {}", destination.display()))?;
    copy_from_fat(&mut reader, &mut output)
        .with_context(|| format!("Failed to extract: {}", destination.display()))?;
    Ok(())
}

fn cmd_create(
    source: &Path,
    output: &Path,
    requested_size: Option<u64>,
    fat_type: FatKind,
    volume_label: &str,
) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to inspect source: {}", source.display()))?;
    if !metadata.is_dir() {
        bail!("Source must be a directory: {}", source.display());
    }

    let inventory = inventory_source(source)?;
    let image_size = requested_size.unwrap_or_else(|| estimate_image_size(&inventory, fat_type));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| format!("Failed to create image: {}", output.display()))?;
    file.set_len(image_size)
        .with_context(|| format!("Failed to size image to {image_size} bytes"))?;

    let selection = match fat_type {
        FatKind::Auto => FatTypeSelection::Auto,
        FatKind::Fat12 => FatTypeSelection::Fat12,
        FatKind::Fat16 => FatTypeSelection::Fat16,
        FatKind::Fat32 => FatTypeSelection::Fat32,
    };
    let options = FatFormatOptions::new(image_size)
        .volume_label(volume_label)
        .fat_type(selection);
    let fs = FatVolumeFormatter::format(file, options).with_context(|| {
        format!(
            "Failed to format {image_size}-byte image; choose a compatible FAT type or increase --size"
        )
    })?;
    let root = fs.root_dir();
    import_directory(&fs, &root, source).with_context(
        || "Failed to import source tree; increase --size if the image is out of space",
    )?;
    println!(
        "Created {} ({:?}, {} bytes)",
        output.display(),
        fs.fat_type(),
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

fn estimate_image_size(inventory: &SourceInventory, fat_type: FatKind) -> u64 {
    const MIB: u64 = 1024 * 1024;
    let minimum = match fat_type {
        FatKind::Fat12 => 2 * MIB,
        FatKind::Fat16 => 16 * MIB,
        FatKind::Fat32 => 64 * MIB,
        FatKind::Auto => 4 * MIB,
    };
    let estimated = inventory
        .bytes
        .saturating_add(inventory.bytes / 2)
        .saturating_add(inventory.entries.saturating_mul(4096))
        .saturating_add(2 * MIB);
    estimated.max(minimum).div_ceil(MIB) * MIB
}

fn import_directory<DATA: FatRead + hadris_fat::Write + hadris_fat::Seek>(
    fs: &FatFs<DATA>,
    destination: &FatDir<'_, DATA>,
    source: &Path,
) -> Result<()> {
    for entry in sorted_host_entries(source)? {
        let source_path = entry.path();
        let metadata = fs::symlink_metadata(&source_path)?;
        let name = entry.file_name().into_string().map_err(|_| {
            anyhow::anyhow!(
                "Host filename is not valid UTF-8: {}",
                source_path.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            bail!(
                "Symbolic links are not supported: {}",
                source_path.display()
            );
        } else if metadata.is_dir() {
            let child = fs
                .create_dir(destination, &name)
                .with_context(|| format!("Failed to create directory in image: {name}"))?;
            import_directory(fs, &child, &source_path)?;
        } else if metadata.is_file() {
            let image_entry = fs
                .create_file(destination, &name)
                .with_context(|| format!("Failed to create file in image: {name}"))?;
            let mut input = File::open(&source_path)?;
            let mut writer = fs.write_file(&image_entry)?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = StdRead::read(&mut input, &mut buffer)?;
                if count == 0 {
                    break;
                }
                let mut offset = 0;
                while offset < count {
                    let written = writer.write(&buffer[offset..count]).with_context(|| {
                        format!("Failed to copy file into image: {}", source_path.display())
                    })?;
                    if written == 0 {
                        bail!("FAT writer made no progress for {}", source_path.display());
                    }
                    offset += written;
                }
            }
            writer.finish()?;
        } else {
            bail!("Unsupported host entry type: {}", source_path.display());
        }
    }
    Ok(())
}

fn copy_from_fat<DATA, W>(
    reader: &mut hadris_fat::read::FileReader<'_, DATA>,
    output: &mut W,
) -> Result<u64>
where
    DATA: FatRead + hadris_fat::Seek,
    W: StdWrite,
{
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        StdWrite::write_all(output, &buffer[..count])?;
        copied += count as u64;
    }
    Ok(copied)
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
