use clap::Parser;
use hadris_iso::{
    BootEntryOptions, BootOptions, EmulationType, FileInput, FormatOptions, PartitionOptions,
};
use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};

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
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)
        .unwrap();
    let options = FormatOptions::new()
        .with_files(FileInput::from_fs(isoroot).unwrap())
        .with_format_options(PartitionOptions::PROTECTIVE_MBR)
        .with_boot_options(BootOptions {
            write_boot_catalogue: true,
            default: BootEntryOptions {
                emulation: EmulationType::NoEmulation,
                load_size: 4,
                boot_image_path: "limine-bios-cd.bin".to_string(),
                boot_info_table: true,
                grub2_boot_info: false,
            },
            entries: vec![],
        });

    let (min, max) = options.image_len();
    log::debug!("Calculate minimum and maximum size of image: {min}b to {max}b");
    file.set_len(max).unwrap();
    hadris_iso::IsoImage::format_new(&mut file, options).unwrap();
    let written = file.stream_position().unwrap();
    log::debug!("Written {written}b to image, trimming...");
    file.set_len(written).unwrap();
    file.flush().unwrap();
}

fn read(file: &PathBuf) {
    let mut file = OpenOptions::new().read(true).open(file).unwrap();
    let mut iso = hadris_iso::IsoImage::new(&mut file).unwrap();
    let mut root_dir = iso.root_directory();
    println!("Root Directory: {:#?}", root_dir.entries());
    println!("Path table: {:#?}", iso.path_table().entries());
}
