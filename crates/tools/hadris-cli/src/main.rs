//! The `hadris` command: one binary for every format Hadris reads and
//! writes, with one set of flags, overwrite rules and output handling.

mod apfs;
mod common;
mod cpio;
mod detect;
mod fat;
mod generic;
mod iso;
mod output;
mod udf;
mod vfs;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hadris", author, version)]
#[command(
    about = "Create, inspect, extract and check FAT, exFAT, ISO 9660, UDF, APFS and cpio images"
)]
struct Cli {
    #[command(subcommand)]
    command: Format,
}

#[derive(Subcommand)]
enum Format {
    /// FAT12, FAT16, FAT32 and exFAT images
    Fat(fat::Args),
    /// Experimental read-only APFS containers and volumes
    Apfs(apfs::Args),
    /// ISO 9660 images with Joliet, Rock Ridge and El Torito
    Iso(iso::Args),
    /// UDF images and ISO 9660 and UDF bridge images
    Udf(udf::Args),
    /// cpio archives
    Cpio(cpio::Args),
    /// List every format an image or device holds
    Detect(detect::Args),
    /// List a directory of any filesystem an image holds
    #[command(alias = "list")]
    Ls(generic::LsArgs),
    /// Print a file of any filesystem an image holds to stdout
    Cat(generic::CatArgs),
    /// Extract a file or directory of any filesystem an image holds
    Extract(generic::ExtractArgs),
    /// Show the volume and a path of any filesystem an image holds
    Stat(generic::StatArgs),
}

fn main() {
    reset_sigpipe();
    let result = match Cli::parse().command {
        Format::Apfs(args) => apfs::run(args),
        Format::Fat(args) => fat::run(args),
        Format::Iso(args) => iso::run(args),
        Format::Udf(args) => udf::run(args),
        Format::Cpio(args) => cpio::run(args),
        Format::Detect(args) => detect::run(args),
        Format::Ls(args) => generic::ls(args),
        Format::Cat(args) => generic::cat(args),
        Format::Extract(args) => generic::extract(args),
        Format::Stat(args) => generic::stat(args),
    };
    if let Err(err) = result {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: SIGPIPE is a valid signal and SIG_DFL is the default disposition,
    // not a Rust callback. This runs during startup before the CLI starts work.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}
