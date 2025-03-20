use clap::Parser;
use hadris_iso::{
    BootEntryOptions, BootOptions, EmulationType, FileInput, FormatOptions, PartitionOptions,
};
use std::{fs::OpenOptions, io::Write, path::PathBuf};

#[derive(Parser)]
pub struct Args {
    input: PathBuf,
}

fn main() {
    let args = Args::parse();
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Trace)
        .init()
        .unwrap();

    write(&args.input);
    read(&args.input);
}

fn write(file: &PathBuf) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(file)
        .unwrap();
    // We zero the file out to make sure we don't have any old data
    file.set_len(0).unwrap();
    file.sync_data().unwrap();
    file.set_len(128 * 2048 * 2048).unwrap();
    let options = FormatOptions::new()
        .with_files(FileInput::from_fs("target/isoroot".into()).unwrap())
        .with_format_options(PartitionOptions::PROTECTIVE_MBR)
        .with_boot_options(BootOptions {
            write_boot_catalogue: true,
            entries: vec![BootEntryOptions {
                emulation: EmulationType::NoEmulation,
                load_size: 4,
                boot_image_path: "limine-bios-cd.bin".to_string(),
                boot_info_table: true,
                grub2_boot_info: false,
            }],
        });
    hadris_iso::IsoImage::format_new(&mut file, options).unwrap();
    file.flush().unwrap();
}

fn read(file: &PathBuf) {
    let mut file = OpenOptions::new().read(true).open(file).unwrap();
    let mut iso = hadris_iso::IsoImage::new(&mut file).unwrap();
    let mut root_dir = iso.root_directory();
    //println!("Root Directory: {:#?}", root_dir.entries());
    //println!("Path table: {:#?}", iso.path_table().entries());
}
