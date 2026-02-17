use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, Write};
use std::path::Path;

use hadris_iso::directory::{DirectoryRef, FileFlags};
use hadris_iso::read::IsoImage;

use crate::args::ExtractArgs;

use super::{Result, clean_name, navigate_to_path};

/// Extract files from an ISO image
pub fn extract(args: ExtractArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    // Create output directory
    fs::create_dir_all(&args.output)?;

    let start_ref = if let Some(ref path) = args.path {
        navigate_to_path(&iso, path)?
    } else {
        iso.root_dir().dir_ref()
    };

    let mut extracted_count = 0;
    extract_dir(
        &iso,
        start_ref,
        &args.output,
        args.verbose,
        &mut extracted_count,
    )?;

    println!(
        "Extracted {} files to {}",
        extracted_count,
        args.output.display()
    );
    Ok(())
}

fn extract_dir<R: Read + Seek>(
    iso: &IsoImage<R>,
    dir_ref: DirectoryRef,
    output_path: &Path,
    verbose: bool,
    count: &mut usize,
) -> Result<()> {
    let dir = iso.open_dir(dir_ref);

    for entry in dir.entries() {
        let entry = entry?;
        let name_bytes = entry.name();

        // Skip . and ..
        if entry.is_special() {
            continue;
        }

        let display_name = clean_name(name_bytes);
        let flags = FileFlags::from_bits_truncate(entry.header().flags);

        if flags.contains(FileFlags::DIRECTORY) {
            let child_path = output_path.join(&display_name);
            fs::create_dir_all(&child_path)?;
            if verbose {
                println!("Creating directory: {}", child_path.display());
            }
            let child_ref = entry.as_dir_ref(iso)?;
            extract_dir(iso, child_ref, &child_path, verbose, count)?;
        } else {
            let extent = entry.header().extent.read() as u64;
            let size = entry.header().data_len.read() as usize;

            let file_path = output_path.join(&display_name);
            if verbose {
                println!("Extracting: {} ({} bytes)", file_path.display(), size);
            }

            if size > 0 {
                let mut buffer = vec![0u8; size];
                iso.read_bytes_at(extent * 2048, &mut buffer)?;
                let mut output_file = File::create(&file_path)?;
                output_file.write_all(&buffer)?;
            } else {
                // Create empty file
                File::create(&file_path)?;
            }

            *count += 1;
        }
    }

    Ok(())
}
