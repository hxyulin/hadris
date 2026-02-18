//! fatutil - FAT filesystem analysis and management utility

use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hadris_fat::{FatAnalysisExt, FatFs, FatVerifyExt};
use hadris_fat::{DirectoryEntry, FatDir, DirEntryAttrFlags};

#[derive(Parser)]
#[command(name = "fatutil")]
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Info { image } => cmd_info(image),
        Commands::Stat { image } => cmd_stat(image),
        Commands::Ls { image, path, long } => cmd_ls(image, &path, long),
        Commands::Tree { image, path, depth } => cmd_tree(image, &path, depth),
        Commands::Fragmentation { image, top } => cmd_fragmentation(image, top),
        Commands::Verify { image, verbose } => cmd_verify(image, verbose),
        Commands::Chain { image, file_path } => cmd_chain(image, &file_path),
    }
}

fn open_fat_fs(path: PathBuf) -> Result<FatFs<File>> {
    let file = File::open(&path)
        .with_context(|| format!("Failed to open image file: {}", path.display()))?;
    FatFs::open(file).context("Failed to parse FAT filesystem")
}

fn cmd_info(image: PathBuf) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let vol = fs.volume_info();

    println!("FAT Filesystem Information");
    println!("==========================");
    println!("FAT Type:        {:?}", fs.fat_type());
    println!("OEM Name:        {}", vol.oem_name());
    println!("Volume Label:    {}", vol.volume_label());
    println!("Volume ID:       {:08X}", vol.volume_id());
    println!("FS Type String:  {}", vol.fs_type_str());

    Ok(())
}

fn cmd_stat(image: PathBuf) -> Result<()> {
    let fs = open_fat_fs(image)?;
    let stats = fs.statistics().context("Failed to gather statistics")?;

    println!("FAT Filesystem Statistics");
    println!("=========================");
    println!("FAT Type:            {:?}", stats.fat_type);
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
            .with_context(|| format!("Failed to open directory: {}", path))?
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
                    file_entry.size().to_string()
                },
                name
            );
        } else if file_entry.is_directory() {
            println!("{}/", name);
        } else {
            println!("{}", name);
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
            .with_context(|| format!("Failed to open directory: {}", path))?
    };

    println!("{}", path);
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
            println!("{}{}{}/", prefix, connector, name);
            let new_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };

            let subdir = fs.open_dir_entry(&entry)?;
            print_tree(fs, &subdir, &new_prefix, max_depth, current_depth + 1)?;
        } else {
            println!("{}{}{}", prefix, connector, name);
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
            println!("  - {}", issue);
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
        .with_context(|| format!("Failed to open: {}", file_path))?;

    let first_cluster = entry.cluster().0 as u32;
    if first_cluster < 2 {
        println!("File '{}' has no cluster chain (empty file)", file_path);
        return Ok(());
    }

    let chain = fs
        .get_cluster_chain(first_cluster)
        .context("Failed to read cluster chain")?;

    println!("Cluster chain for: {}", file_path);
    println!("File size: {} bytes", entry.size());
    println!("Chain length: {} clusters", chain.len());
    println!();

    // Count fragments
    let mut fragments = 1;
    for window in chain.windows(2) {
        if window[1] != window[0] + 1 {
            fragments += 1;
        }
    }
    println!("Fragments: {}", fragments);
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
            print!("{}", cluster);
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
            print!("{}", cluster);
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
            print!("{}", cluster);
        }
        println!();
    }

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
        format!("{} B", bytes)
    }
}
