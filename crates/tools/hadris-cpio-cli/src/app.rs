//! Hadris CPIO archive utility for listing, creating, extracting, and inspecting archives.

#[path = "commands/mod.rs"]
mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "hadris-cpio")]
#[command(author, version, about = "CPIO archive utility", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Header format of a new archive.
#[derive(Clone, Copy, ValueEnum)]
pub enum ArchiveFormat {
    /// newc (070701), as Linux initramfs uses
    Newc,
    /// newc with per-file checksums (070702)
    Crc,
    /// old portable ASCII (070707)
    Odc,
}

#[derive(Subcommand)]
enum Commands {
    /// List archive entries
    #[command(alias = "list")]
    Ls {
        /// Path to the CPIO archive, or - for standard input
        archive: PathBuf,
        /// Show long format with permissions, uid/gid, size, mtime
        #[arg(short, long)]
        long: bool,
    },
    /// Display detailed archive and entry information
    Info {
        /// Path to the CPIO archive, or - for standard input
        archive: PathBuf,
    },
    /// Create a CPIO archive from a directory
    Create {
        /// Directory to pack
        directory: PathBuf,
        /// Output archive path, or - for standard output
        #[arg(short, long)]
        output: PathBuf,
        /// Header format
        #[arg(long, value_enum, default_value = "newc")]
        format: ArchiveFormat,
        /// Use CRC format (070702); the same as --format crc
        #[arg(long)]
        crc: bool,
        /// Also list metadata the format cannot store
        #[arg(short, long)]
        verbose: bool,
    },
    /// Extract a CPIO archive to a directory
    Extract {
        /// Path to the CPIO archive, or - for standard input
        archive: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Print a file's contents from the archive to stdout
    Cat {
        /// Path to the CPIO archive, or - for standard input
        archive: PathBuf,
        /// Path of the file within the archive
        path: String,
    },
}

/// Parse command-line arguments and run the CPIO utility.
pub fn run() -> Result<()> {
    reset_sigpipe();
    let cli = Cli::parse();

    match cli.command {
        Commands::Ls { archive, long } => commands::list(archive, long),
        Commands::Info { archive } => commands::info(archive),
        Commands::Create {
            directory,
            output,
            format,
            crc,
            verbose,
        } => {
            let format = if crc { ArchiveFormat::Crc } else { format };
            commands::create(directory, output, format, verbose)
        }
        Commands::Extract { archive, output } => commands::extract(archive, output),
        Commands::Cat { archive, path } => commands::cat(archive, &path),
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
