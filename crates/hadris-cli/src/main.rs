use std::{fs::OpenOptions, io::Read, path::PathBuf};

use clap::Parser;
use hadris_fat::FatFs;

#[derive(Debug, clap::Parser)]
pub struct Args {
    input: PathBuf,
}
fn main() {
    let args = Args::parse();
    let mut file = OpenOptions::new().read(true).open(args.input).unwrap();
    let fat_fs = FatFs::open(&mut file).unwrap();
    let root = fat_fs.root_dir();
    for entry in root.entries() {
        let entry = entry.unwrap();
        println!("{:#?}", entry);
    }
}
