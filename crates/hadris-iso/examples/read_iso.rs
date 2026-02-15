//! Example: Reading an ISO Image
//!
//! This example demonstrates how to open an ISO image and list its contents.
//!
//! Run with: `cargo run --example read_iso -- path/to/image.iso`

use std::env;
use std::fs::File;
use std::io::BufReader;

use hadris_iso::directory::FileFlags;
use hadris_iso::read::IsoImage;
use hadris_iso::volume::VolumeDescriptor;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <iso-file>", args[0]);
        eprintln!();
        eprintln!("Example: {} ubuntu.iso", args[0]);
        std::process::exit(1);
    }

    let iso_path = &args[1];
    println!("Opening ISO: {}", iso_path);
    println!();

    // Open the ISO file
    let file = File::open(iso_path).expect("Failed to open ISO file");
    let reader = BufReader::new(file);
    let image = IsoImage::open(reader).expect("Failed to parse ISO image");

    // Display volume information
    println!("=== Volume Information ===");
    let pvd = image.read_pvd();
    println!(
        "Volume Identifier: {}",
        pvd.volume_identifier.to_str().trim()
    );
    println!(
        "Volume Set Identifier: {}",
        pvd.volume_set_identifier.to_str().trim()
    );
    println!("Publisher: {}", pvd.publisher_identifier.to_str().trim());
    println!("Data Preparer: {}", pvd.preparer_identifier.to_str().trim());
    println!("Volume Size: {} sectors", pvd.volume_space_size.read());
    println!(
        "Logical Block Size: {} bytes",
        pvd.logical_block_size.read()
    );
    println!();

    // List volume descriptors
    println!("=== Volume Descriptors ===");
    for (i, vd_result) in image.read_volume_descriptors().enumerate() {
        match vd_result {
            Ok(vd) => {
                let desc = match &vd {
                    VolumeDescriptor::Primary(_) => "Primary Volume Descriptor",
                    VolumeDescriptor::Supplementary(svd) => {
                        if svd.header.version == 2 {
                            "Enhanced Volume Descriptor"
                        } else {
                            "Supplementary Volume Descriptor (Joliet)"
                        }
                    }
                    VolumeDescriptor::BootRecord(_) => "Boot Record (El-Torito)",
                    VolumeDescriptor::End(_) => "Volume Set Terminator",
                    VolumeDescriptor::Unknown(_) => "Unknown Volume Descriptor",
                };
                println!("  [{}] {}", i, desc);
            }
            Err(e) => {
                eprintln!("  [{}] Error reading descriptor: {:?}", i, e);
            }
        }
    }
    println!();

    // List root directory contents
    println!("=== Root Directory Contents ===");
    let root = image.root_dir();
    list_directory(&image, &root, 0);
}

fn list_directory<R: hadris_io::Read + hadris_io::Seek>(
    image: &IsoImage<R>,
    dir: &hadris_iso::read::RootDir,
    indent: usize,
) {
    let prefix = "  ".repeat(indent);
    let iter = dir.iter(image);

    for entry_result in iter.entries() {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("{}Error reading entry: {:?}", prefix, e);
                continue;
            }
        };

        let name = String::from_utf8_lossy(entry.name());

        // Skip special entries (. and ..)
        if name == "\x00" || name == "\x01" {
            continue;
        }

        let header = entry.header();
        let flags = FileFlags::from_bits_truncate(header.flags);
        let size = header.data_len.read();

        if flags.contains(FileFlags::DIRECTORY) {
            println!("{}{}/", prefix, name);
        } else {
            println!("{}{} ({} bytes)", prefix, name, size);
        }
    }
}
