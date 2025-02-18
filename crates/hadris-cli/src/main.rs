use std::{path::PathBuf, str::FromStr};

use clap::Parser;
use hadris::FileSystemType;

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
    input: PathBuf,
    #[clap(subcommand)]
    command: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    Read,
    Write,
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
            std::fs::write(args.input, bytes).unwrap();
        }
        _ => unimplemented!(),
    }
}
