//! `hadris cpio`: cpio archives.

mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Subcommand, ValueEnum};

use crate::common::Target;

#[derive(clap::Args)]
pub struct Args {
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
        /// `-o -` writes the archive to standard output
        #[command(flatten)]
        target: Target,
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
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
        /// Path within the archive to extract (default: extract all); a path
        /// other than the root is written to `<output>/<name>`
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Print a file's contents from the archive to stdout
    Cat {
        /// Path to the CPIO archive, or - for standard input
        archive: PathBuf,
        /// Path of the file within the archive
        path: String,
    },
}

/// Runs a `hadris cpio` command.
pub fn run(cli: Args) -> Result<()> {
    match cli.command {
        Commands::Ls { archive, long } => commands::list(archive, long),
        Commands::Info { archive } => commands::info(archive),
        Commands::Create {
            directory,
            target,
            format,
            crc,
            verbose,
        } => {
            let format = if crc { ArchiveFormat::Crc } else { format };
            commands::create(directory, target, format, verbose)
        }
        Commands::Extract {
            archive,
            output,
            path,
        } => commands::extract(archive, output, path.as_deref()),
        Commands::Cat { archive, path } => commands::cat(archive, &path),
    }
}
