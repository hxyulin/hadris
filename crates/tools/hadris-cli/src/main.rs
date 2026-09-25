//! The `hadris` command: one binary for every format Hadris reads and
//! writes, with one set of flags, overwrite rules and output handling.

mod common;
mod cpio;
mod detect;
mod fat;
mod iso;
mod output;
mod udf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hadris", author, version)]
#[command(about = "Create, inspect, extract and check FAT, exFAT, ISO 9660, UDF and cpio images")]
struct Cli {
    #[command(subcommand)]
    command: Format,
}

#[derive(Subcommand)]
enum Format {
    /// FAT12, FAT16, FAT32 and exFAT images
    Fat(fat::Args),
    /// ISO 9660 images with Joliet, Rock Ridge and El Torito
    Iso(iso::Args),
    /// UDF images and ISO 9660 and UDF bridge images
    Udf(udf::Args),
    /// cpio archives
    Cpio(cpio::Args),
    /// List every format an image or device holds
    Detect(detect::Args),
}

fn main() {
    reset_sigpipe();
    let result = match Cli::parse().command {
        Format::Fat(args) => fat::run(args),
        Format::Iso(args) => iso::run(args).map_err(|err| anyhow::anyhow!("{err}")),
        Format::Udf(args) => udf::run(args).map_err(|err| anyhow::anyhow!("{err}")),
        Format::Cpio(args) => cpio::run(args),
        Format::Detect(args) => detect::run(args),
    };
    if let Err(err) = result {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
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
