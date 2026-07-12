//! cpioutil - CPIO archive utility for listing, creating, extracting, and inspecting archives.

mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cpioutil")]
#[command(author, version, about = "CPIO archive utility", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List archive entries
    List {
        /// Path to the CPIO archive
        archive: PathBuf,
        /// Show long format with permissions, uid/gid, size, mtime
        #[arg(short, long)]
        long: bool,
    },
    /// Display detailed archive and entry information
    Info {
        /// Path to the CPIO archive
        archive: PathBuf,
    },
    /// Create a CPIO archive from a directory
    Create {
        /// Directory to pack
        directory: PathBuf,
        /// Output archive path
        #[arg(short, long)]
        output: PathBuf,
        /// Use CRC format (070702) instead of newc (070701)
        #[arg(long)]
        crc: bool,
    },
    /// Extract a CPIO archive to a directory
    Extract {
        /// Path to the CPIO archive
        archive: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Print a file's contents from the archive to stdout
    Cat {
        /// Path to the CPIO archive
        archive: PathBuf,
        /// Path of the file within the archive
        path: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List { archive, long } => commands::list(archive, long),
        Commands::Info { archive } => commands::info(archive),
        Commands::Create {
            directory,
            output,
            crc,
        } => commands::create(directory, output, crc),
        Commands::Extract { archive, output } => commands::extract(archive, output),
        Commands::Cat { archive, path } => commands::cat(archive, &path),
    }
}
