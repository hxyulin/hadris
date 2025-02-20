use std::{path::PathBuf, str::FromStr};

use clap::Parser;
use hadris::{FileSystemType, OpenOptions};

#[derive(Debug, Clone, Copy)]
struct ByteSize(pub u64);

impl FromStr for ByteSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const UNITS: [(&str, u64); 4] = [
            ("KiB", 1024),
            ("MiB", 1024 * 1024),
            ("GiB", 1024 * 1024 * 1024),
            ("B", 1),
        ];
        for unit in UNITS {
            if !s.ends_with(unit.0) {
                continue;
            }
            let value = s
                .trim_end_matches(unit.0)
                .parse::<u64>()
                .map_err(|e| format!("Invalid byte size: {e}"))?;
            return Ok(ByteSize(value * unit.1));
        }
        Ok(ByteSize(
            s.parse::<u64>()
                .map_err(|_| "Invalid byte size".to_string())?,
        ))
    }
}

#[derive(Debug, Parser)]
struct Arguments {
    #[clap(value_parser)]
    image: PathBuf,
    #[clap(subcommand)]
    command: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    Read(ReadCommand),
    Write(WriteCommand),
    Create(CreateCommand),
}

/// Command used to create a filesystem image
#[derive(Debug, clap::Args)]
struct CreateCommand {
    #[clap(value_parser)]
    size: ByteSize,
    #[clap(short = 't', long = "type")]
    ty: FsType,
}

#[derive(Debug, clap::Args)]
struct WriteCommand {
    file: PathBuf,
}

#[derive(Debug, clap::Args)]
struct ReadCommand {
    file: PathBuf,
    #[clap(short = 'b', long = "buffer-size", default_value = "8192")]
    buf_size: usize,
}

#[derive(Debug, clap::ValueEnum, Clone, Copy)]
enum FsType {
    #[clap(name = "fat32", alias = "fat")]
    Fat32,
}

impl Into<FileSystemType> for FsType {
    fn into(self) -> FileSystemType {
        match self {
            FsType::Fat32 => FileSystemType::Fat32,
        }
    }
}

fn main() {
    use hadris::FileSystem;
    let args = Arguments::parse();
    match args.command {
        Subcommand::Create(CreateCommand { size, ty }) => {
            let mut bytes = vec![0; size.0 as usize];
            {
                _ = FileSystem::create_with_bytes(ty.into(), &mut bytes)
            };
            std::fs::write(args.image, bytes).unwrap();
        }
        Subcommand::Write(WriteCommand { file }) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            if atty::isnt(atty::Stream::Stdin) {
                let mut stdin = std::io::stdin();
                stdin.read_to_end(&mut bytes).unwrap();
            } else {
                println!("Reading from stdin...");
                std::io::stdin().read_to_end(&mut bytes).unwrap();
            }

            let mut fs_bytes = std::fs::read(&args.image).unwrap();
            let mut fs = FileSystem::read_from_bytes(FileSystemType::Fat32, &mut fs_bytes);
            let file = fs
                .open_file(
                    file.as_os_str().to_str().unwrap(),
                    OpenOptions::WRITE | OpenOptions::CREATE,
                )
                .unwrap();
            file.write(&mut fs, &bytes).unwrap();
            drop(file);
            drop(fs);
            std::fs::write(&args.image, &fs_bytes).unwrap();
        }
        Subcommand::Read(ReadCommand { file, buf_size }) => {
            let mut fs_bytes = std::fs::read(&args.image).unwrap();
            let mut fs = FileSystem::read_from_bytes(FileSystemType::Fat32, &mut fs_bytes);
            let file = fs
                .open_file(file.as_os_str().to_str().unwrap(), OpenOptions::READ)
                .unwrap();
            let mut buf = Vec::with_capacity(buf_size);
            buf.resize(buf_size, 0);
            let mut read = file.read(&mut fs, &mut buf).unwrap();
            while read != 0 {
                print!("{}", std::str::from_utf8(&buf[..read]).unwrap());
                read = file.read(&mut fs, &mut buf).unwrap();
            }
        }
    }
}
