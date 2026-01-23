mod args;
mod commands;

use args::{Args, Command};
use clap::Parser;

fn main() {
    let args = Args::parse();

    let result = match args.cmd {
        Command::Info(args) => commands::info(args),
        Command::Ls(args) => commands::ls(args),
        Command::Tree(args) => commands::tree(args),
        Command::Extract(args) => commands::extract(args),
        Command::Create(args) => commands::create(args),
        Command::Verify(args) => commands::verify(args),
        Command::Mkisofs(args) => commands::mkisofs(args),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
