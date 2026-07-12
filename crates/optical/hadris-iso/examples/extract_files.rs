//! Example: Extracting Files from an ISO
//!
//! This example demonstrates how to extract files from an ISO image
//! to a directory on disk.
//!
//! Run with: `cargo run --example extract_files -- image.iso output_dir`

use std::env;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use hadris_iso::directory::FileFlags;
use hadris_iso::read::IsoImage;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <iso-file> <output-dir>", args[0]);
        eprintln!();
        eprintln!("Example: {} ubuntu.iso ./extracted", args[0]);
        std::process::exit(1);
    }

    let iso_path = &args[1];
    let output_dir = Path::new(&args[2]);

    println!("Extracting {} to {:?}", iso_path, output_dir);
    println!();

    // Create output directory
    fs::create_dir_all(output_dir).expect("Failed to create output directory");

    // Open the ISO file for parsing structure
    let file = File::open(iso_path).expect("Failed to open ISO file");
    let reader = BufReader::new(file);
    let image = IsoImage::open(reader).expect("Failed to parse ISO image");

    // Open another handle for reading file content
    let file2 = File::open(iso_path).expect("Failed to open ISO file for reading");
    let mut content_reader = BufReader::new(file2);

    // Extract root directory
    let root = image.root_dir();
    let mut count = 0;
    extract_directory(&image, &mut content_reader, &root, output_dir, &mut count);

    println!();
    println!("Extracted {} files", count);
}

fn extract_directory<R: hadris_io::Read + hadris_io::Seek, C: Read + Seek>(
    image: &IsoImage<R>,
    content_reader: &mut C,
    dir: &hadris_iso::read::RootDir,
    output_path: &Path,
    count: &mut usize,
) {
    let iter = dir.iter(image);

    for entry_result in iter.entries() {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Error reading entry: {:?}", e);
                continue;
            }
        };

        let name = String::from_utf8_lossy(entry.name()).to_string();

        // Skip special entries (. and ..)
        if name == "\x00" || name == "\x01" {
            continue;
        }

        // Clean up the filename (remove version suffix like ";1")
        let clean_name = if let Some(pos) = name.find(';') {
            &name[..pos]
        } else {
            &name
        };

        let header = entry.header();
        let flags = FileFlags::from_bits_truncate(header.flags);
        let file_path = output_path.join(clean_name);

        if flags.contains(FileFlags::DIRECTORY) {
            // Create directory and recurse
            println!("Creating directory: {:?}", file_path);
            fs::create_dir_all(&file_path).expect("Failed to create directory");
            // Note: To fully extract subdirectories, you would need to
            // navigate into them using the directory extent
        } else {
            // Extract file
            let extent = header.extent.read() as u64;
            let size = header.data_len.read() as usize;

            println!("Extracting: {} ({} bytes)", clean_name, size);

            // Seek to file content
            let offset = extent * 2048;
            content_reader
                .seek(SeekFrom::Start(offset))
                .expect("Failed to seek");

            // Read and write content
            let mut content = vec![0u8; size];
            content_reader
                .read_exact(&mut content)
                .expect("Failed to read file content");

            let mut out_file = File::create(&file_path).expect("Failed to create output file");
            out_file.write_all(&content).expect("Failed to write file");

            *count += 1;
        }
    }
}
