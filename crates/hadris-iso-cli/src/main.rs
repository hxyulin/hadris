use clap::Parser;
use hadris_iso::{
    boot::{options::{BootEntryOptions, BootOptions, BootSectionOptions}, EmulationType},
    read::{IsoDir, IsoImage, PathSeparator},
    write::{
        options::{BaseIsoLevel, CreationFeatures, FormatOptions, JolietLevel}, InputFiles, IsoImageWriter
    },
};
use std::{fs::OpenOptions, io::Read, num::NonZeroU16, path::PathBuf, str::FromStr};

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
        Self(BaseIsoLevel::Level1 {
            supports_lowercase: false,
        })
    }
}

impl FromStr for ArgLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "1" => Self(BaseIsoLevel::Level1 {
                supports_lowercase: false,
            }),
            "2" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: false,
            }),
            "1l" => Self(BaseIsoLevel::Level1 {
                supports_lowercase: true,
            }),
            "2l" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: true,
            }),
            _ => return Err("invalid level"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct IsoExtensions {
    level3: bool,
    joliet: bool,
}

impl FromStr for IsoExtensions {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut exts = Self {
            level3: false,
            joliet: false,
        };
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
    isoroot: PathBuf,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(short, long)]
    verbose: bool,
    #[arg(short, long, default_value = "1")]
    level: ArgLevel,
    #[arg(long = "ex", default_value = "")]
    extensions: IsoExtensions,
    #[arg(short, long)]
    boot: Option<String>,
}

fn main() {
    let args = Args::parse();
    match args.cmd {
        Command::Read(args) => read(&args.input),
        Command::Write(args) => write(args.isoroot, &args.output, args.level.0, args.extensions, args.boot),
        Command::Xorriso(args) => {
            println!("xorriso {:?}", args);
        }
    }
}
fn write(
    isoroot: PathBuf,
    output: &PathBuf,
    level: BaseIsoLevel,
    exts: IsoExtensions,
    boot: Option<String>,
) {
    let mut file = OpenOptions::new()
        .truncate(true)
        .read(true)
        .write(true)
        .create(true)
        .open(output)
        .unwrap();
    file.set_len(10_000_000).unwrap();
    let input = InputFiles::from_fs(&isoroot, PathSeparator::ForwardSlash).unwrap();
    let mut ops = FormatOptions {
        volume_name: "TESTISO".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: level,
            long_filenames: exts.level3,
            joliet: if exts.joliet {
                Some(JolietLevel::Level1)
            } else {
                None
            },
            ..Default::default()
        },
    };
    ops.features.el_torito = Some(BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            load_size: NonZeroU16::new(4),
            boot_image_path: "limine-bios-cd.bin".to_string(),
            boot_info_table: true,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![
            (
            BootSectionOptions {
                platform: hadris_iso::boot::PlatformId::UEFI,
            },
                BootEntryOptions {
                    load_size: None,
                    boot_image_path: "limine-uefi-cd.bin".to_string(),
                    boot_info_table: false,
                    grub2_boot_info: false,
                    emulation: EmulationType::NoEmulation,
                },
        )
        ]
    });
    IsoImageWriter::format_new(&mut file, input, ops).unwrap();
}

fn read(file: &PathBuf) {
    let mut file = OpenOptions::new().read(true).open(file).unwrap();
    let iso = IsoImage::parse(&mut file).unwrap();
    dbg!(&iso);
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
            let name = core::str::from_utf8(&entry.name).unwrap();
            println!("File: {:?}", name);
        }
    }
}
