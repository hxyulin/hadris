//! `hadris iso`: ISO 9660 images.

mod args;
mod commands;

pub use args::{Args, IsoFlags};
pub use commands::iso_options;

use args::Command;

/// Runs a `hadris iso` command.
pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.cmd {
        Command::Info(args) => commands::info(args),
        Command::Ls(args) => commands::ls(args),
        Command::Tree(args) => commands::tree(args),
        Command::Extract(args) => commands::extract(args),
        Command::Create(args) => commands::create(args),
        Command::Verify(args) => commands::verify(args),
        Command::Mkisofs(args) => commands::mkisofs(args),
        Command::Cat(args) => commands::cat(args),
    }
}
