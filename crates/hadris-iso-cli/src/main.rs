mod args;
mod read;
mod write;
mod xorriso;

use args::Args;

use crate::args::Command;

fn main() {
    use clap::Parser;
    let args = Args::parse();
    match args.cmd {
        Command::Read(args) => read::read(args),
        Command::Write(args) => write::write(args),
        Command::Xorriso(args) => xorriso::xorriso(args),
    }
}
