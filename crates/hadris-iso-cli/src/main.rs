use clap::Parser;
use hadris_iso::{
    read::{IsoDir, IsoImage, PathSeparator},
    write::{
        InputFiles, IsoImageWriter,
        options::{BaseIsoLevel, CreationFeatures, FormatOptions},
    },
};
use std::{fs::OpenOptions, path::PathBuf, str::FromStr};

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

#[derive(Debug, Clone)]
struct ArgLevel(BaseIsoLevel);

impl Default for ArgLevel {
    fn default() -> Self {
        Self(BaseIsoLevel::Level1)
    }
}

impl FromStr for ArgLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "1" => Self(BaseIsoLevel::Level1),
            "2" => Self(BaseIsoLevel::Level2),
            _ => return Err("invalid level"),
        })
    }
}

#[derive(Debug, Clone, Parser)]
pub struct WriteArgs {
    isoroot: PathBuf,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(short, long)]
    verbose: bool,
    #[arg(short, long, default_value = "1")]
    level: ArgLevel,
}

fn main() {
    let args = Args::parse();
    match args.cmd {
        Command::Read(args) => read(&args.input),
        Command::Write(args) => write(args.isoroot, &args.output, args.level.0),
        Command::Xorriso(args) => {
            println!("xorriso {:?}", args);
        }
    }
}
fn write(isoroot: PathBuf, output: &PathBuf, level: BaseIsoLevel) {
    let mut file = OpenOptions::new()
        .truncate(true)
        .read(true)
        .write(true)
        .create(true)
        .open(output)
        .unwrap();
    file.set_len(10_000_000).unwrap();
    let input = InputFiles::from_fs(&isoroot, PathSeparator::ForwardSlash).unwrap();
    let ops = FormatOptions {
        volume_name: "TESTISO".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: level,
            ..Default::default()
        },
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
            println!("Directory: {}", core::str::from_utf8(&entry.name).unwrap());
            let dir = iso.read_dir(entry.as_dir_ref().unwrap());
            read_dir(iso, dir);
        } else {
            println!("File: {:?}", core::str::from_utf8(&entry.name).unwrap());
        }
    }
}
