use clap::Parser;
use hadris_iso::write::options::BaseIsoLevel;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Clone, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Command,
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
    pub volume_name: String,
}

#[derive(Debug, Clone, Parser)]
pub struct ReadArgs {
    pub input: PathBuf,
    #[arg(short, long)]
    pub verbose: bool,
    #[arg(short)]
    pub display_info: bool,
    #[arg(long)]
    pub extract: Option<PathBuf>,
    #[arg(long = "ls")]
    pub list: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArgLevel(pub BaseIsoLevel);

impl Default for ArgLevel {
    fn default() -> Self {
        Self(BaseIsoLevel::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        })
    }
}

impl FromStr for ArgLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "1" => Self(BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            }),
            "2" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: false,
                supports_rrip: false,
            }),
            "1l" => Self(BaseIsoLevel::Level1 {
                supports_lowercase: true,
                supports_rrip: false,
            }),
            "2l" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: true,
                supports_rrip: false,
            }),
            _ => return Err("invalid level"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct IsoExtensions {
    pub level3: bool,
    pub joliet: bool,
}

impl FromStr for IsoExtensions {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut exts = Self {
            level3: false,
            joliet: false,
        };
        if s.is_empty() {
            return Ok(exts);
        }
        for ext in s.split(',') {
            let ext = ext.trim();
            match ext {
                "l3" | "level3" => exts.level3 = true,
                "joliet" => exts.joliet = true,
                _ => return Err("invalid extension"),
            }
        }
        Ok(exts)
    }
}

#[derive(Debug, Clone, Parser)]
pub struct WriteArgs {
    pub isoroot: PathBuf,
    #[arg(short, long)]
    pub output: PathBuf,
    #[arg(short, long)]
    pub verbose: bool,
    #[arg(short, long, default_value = "1")]
    pub level: ArgLevel,
    #[arg(long = "ex", default_value = "")]
    pub extensions: IsoExtensions,
    #[arg(short, long)]
    pub boot: Option<String>,
}
