//! `hadris udf`: UDF images and ISO 9660 and UDF bridge images.

mod args;
mod bridge;
mod commands;

pub use args::Args;

use args::Command;

/// Runs a `hadris udf` command.
pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.cmd {
        Command::Info(args) => commands::info(args),
        Command::Ls(args) => commands::ls(args),
        Command::Tree(args) => commands::tree(args),
        Command::Cat(args) => commands::cat(args),
        Command::Extract(args) => commands::extract(args),
        Command::Create(args) => commands::create(args),
        Command::Verify(args) => commands::verify(args),
        Command::Bridge(args) => bridge::create(args),
        Command::Compare(args) => bridge::compare(&args.input),
    }
}
