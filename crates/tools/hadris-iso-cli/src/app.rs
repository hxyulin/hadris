#[path = "args.rs"]
mod args;
#[path = "commands/mod.rs"]
mod commands;

use args::{Args, Command};
use clap::Parser;

/// Parse command-line arguments and run the ISO utility.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

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
