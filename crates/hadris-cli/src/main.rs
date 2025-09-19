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
    let mut fat_fs = FatFs::open(&mut file).unwrap();
    let root = fat_fs.root_dir();
    let mut root = root.entries(&mut fat_fs).unwrap();
    while let Some(entry) = root.next().unwrap() {
        println!("{:#?}", entry);
        let mut reader = entry.handle().read(root.fs).unwrap();
        let mut contents = String::new();
        reader.read_to_string(&mut contents).unwrap();
        println!("Contents: {}", contents);
        println!("Len: {}", contents.len());
    }
}
