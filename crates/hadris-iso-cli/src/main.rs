use clap::Parser;
use hadris_iso::{
    options::FormatOptions,
    read::{IsoDir, IsoImage, PathSeparator},
    write::{File, InputFiles, IsoImageWriter},
};
use std::{fs::OpenOptions, path::PathBuf};

#[derive(Debug, Clone, Parser)]
pub struct Args {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    Read(ReadArgs),
    Write(WriteArgs),
    Xorriso(XorrisoArgs),
}

impl Command {
    pub fn verbose(&self) -> bool {
        match self {
            Command::Read(args) => args.verbose,
            Command::Write(args) => args.verbose,
            Command::Xorriso(_) => false,
        }
    }
}

/// A xorriso-like subcommand
#[derive(Debug, Clone, Parser)]
pub struct XorrisoArgs {
    #[arg(short = 'V')]
    volume_name: String,
}

#[derive(Debug, Clone, Parser)]
pub struct ReadArgs {
    input: PathBuf,
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct WriteArgs {
    isoroot: PathBuf,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();
    simple_logger::SimpleLogger::new()
        .with_level(if args.cmd.verbose() {
            log::LevelFilter::Trace
        } else {
            log::LevelFilter::Warn
        })
        .init()
        .unwrap();

    match args.cmd {
        Command::Read(args) => read(&args.input),
        Command::Write(args) => write(args.isoroot, &args.output),
        Command::Xorriso(args) => {
            println!("xorriso {:?}", args);
        }
    }
}

fn write(isoroot: PathBuf, output: &PathBuf) {
    let mut file = OpenOptions::new()
        .truncate(true)
        .read(true)
        .write(true)
        .create(true)
        .open(output)
        .unwrap();
    file.set_len(1_000_000).unwrap();
    let input = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![
            File::File {
                name: "test.txt".to_string(),
                contents: vec![b'H'; 5000],
            },
            File::Directory {
                name: "boot".to_string(),
                children: vec![File::Directory {
                    name: "efi".to_string(),
                    children: vec![File::File {
                        name: "BOOTX64.EFI".to_string(),
                        contents: vec![b'a'; 300],
                    }],
                }],
            },
        ],
    };
    let ops = FormatOptions {
        volume_name: "TESTISO".to_string(),
        sector_size: 2048,
    };
    IsoImageWriter::format_new(&mut file, input, ops).unwrap();
}

fn read(file: &PathBuf) {
    let mut file = OpenOptions::new().read(true).open(file).unwrap();
    let iso = IsoImage::parse(&mut file).unwrap();
    let root = iso.root_dir();
    read_dir(&iso, root);
}

fn read_dir(iso: &IsoImage<&mut std::fs::File>, dir: IsoDir<'_, &mut std::fs::File>) {
    let mut entries = dir.entries();
    while let Some(entry) = entries.next() {
        let entry = entry.unwrap();
        if entry.is_special() {
            continue;
        }
        if entry.is_directory() {
            println!("Directory: {}", entry.name);
            let dir = iso.read_dir(entry.as_dir_ref().unwrap());
            read_dir(iso, dir);
        } else {
            println!("File: {}", entry.name);
        }
    }
}
