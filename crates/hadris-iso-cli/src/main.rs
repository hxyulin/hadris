use clap::Parser;
use hadris_iso::{
    BootEntryOptions, BootOptions, BootSectionOptions, EmulationType, FileInput, FormatOptions,
    IsoImage, PartitionOptions, PlatformId,
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
}

impl Command {
    pub fn verbose(&self) -> bool {
        match self {
            Command::Read(args) => args.verbose,
            Command::Write(args) => args.verbose,
        }
    }
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
    }
}

fn write(isoroot: PathBuf, output: &PathBuf) {
    let options = FormatOptions::new()
        .with_volume_name("LIMINEBOOT".to_string())
        .with_files(FileInput::from_fs(isoroot).unwrap())
        .with_format_options(PartitionOptions::PROTECTIVE_MBR | PartitionOptions::GPT)
        .with_boot_options(BootOptions {
            write_boot_catalogue: true,
            default: BootEntryOptions {
                emulation: EmulationType::NoEmulation,
                load_size: 4,
                boot_image_path: "limine-bios-cd.bin".to_string(),
                boot_info_table: true,
                grub2_boot_info: false,
            },
            entries: vec![(
                BootSectionOptions {
                    platform_id: PlatformId::UEFI,
                },
                BootEntryOptions {
                    emulation: EmulationType::NoEmulation,
                    load_size: 0,
                    boot_image_path: "limine-uefi-cd.bin".to_string(),
                    boot_info_table: false,
                    grub2_boot_info: false,
                },
            )],
        });

    // TODO: Empty directories don't work
    IsoImage::format_file(output, options).unwrap();
}

fn read(file: &PathBuf) {
    let mut file = OpenOptions::new().read(true).open(file).unwrap();
    let mut iso = hadris_iso::IsoImage::parse(&mut file).unwrap();
    let mut _root_dir = iso.root_directory();
}
