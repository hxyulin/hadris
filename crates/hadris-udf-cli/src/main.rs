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
        Command::Create(args) => commands::create(args),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
