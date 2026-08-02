use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use hadris_iso::directory::{DirectoryRef, FileFlags};
use hadris_iso::read::IsoImage;

use super::super::args::ExtractArgs;

use super::{Result, display_name, navigate_to_path};

/// Extract files from an ISO image
pub fn extract(args: ExtractArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;
    let entry_type = iso.root_dir().entry_type();

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
        entry_type,
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
    entry_type: hadris_iso::file::EntryType,
    output_path: &Path,
    verbose: bool,
    count: &mut usize,
) -> Result<()> {
    let dir = iso.open_dir(dir_ref);

    for entry in dir.entries() {
        let entry = entry?;
        // Skip . and ..
        if entry.is_special() {
            continue;
        }

        let display_name = display_name(&entry, entry_type);
        let entry_path = safe_entry_path(output_path, &display_name)?;
        let flags = FileFlags::from_bits_truncate(entry.header().flags);

        if flags.contains(FileFlags::DIRECTORY) {
            fs::create_dir_all(&entry_path)?;
            if verbose {
                println!("Creating directory: {}", entry_path.display());
            }
            let child_ref = entry.as_dir_ref(iso)?;
            extract_dir(iso, child_ref, entry_type, &entry_path, verbose, count)?;
        } else {
            let extent = entry.header().extent.read() as u64;
            let size = entry.header().data_len.read() as usize;

            if verbose {
                println!("Extracting: {} ({} bytes)", entry_path.display(), size);
            }

            if size > 0 {
                let mut buffer = vec![0u8; size];
                iso.read_bytes_at(extent * 2048, &mut buffer)?;
                let mut output_file = File::create(&entry_path)?;
                output_file.write_all(&buffer)?;
            } else {
                // Create empty file
                File::create(&entry_path)?;
            }

            *count += 1;
        }
    }

    Ok(())
}

fn safe_entry_path(output_path: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if component == std::ffi::OsStr::new(name) => {
            Ok(output_path.join(path))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsafe filename in ISO image: {name:?}"),
        )
        .into()),
    }
}
