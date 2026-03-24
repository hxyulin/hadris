use std::fs;
use std::path::Path;

use hadris_udf::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};
use hadris_udf::UdfRevision;

use crate::args::CreateArgs;

use super::Result;

/// Create a new UDF image
pub fn create(args: CreateArgs) -> Result<()> {
    if args.verbose {
        println!("Creating UDF image from: {}", args.source.display());
        if !args.dry_run {
            println!("Output: {}", args.output.display());
        }
    }

    // Parse the UDF revision string
    let revision = parse_revision(&args.revision)?;

    // Build the directory tree from the source directory
    let mut root = SimpleDir::root();
    let file_count = build_dir(&args.source, &mut root, args.verbose)?;

    if args.verbose {
        println!("Found {} files", file_count);
    }

    if args.dry_run {
        println!("Dry run: would create UDF image");
        println!("  Volume name: {}", args.volume_name);
        println!("  UDF revision: {}", revision);
        println!("  Files: {}", file_count);
        return Ok(());
    }

    let options = UdfWriteOptions {
        volume_id: args.volume_name.clone(),
        revision,
        ..UdfWriteOptions::default()
    };

    let output_file = fs::File::create(&args.output)?;
    let sectors = UdfWriter::format(output_file, &root, options)?;

    if args.verbose {
        println!(
            "Created UDF image: {} ({} sectors, {} bytes)",
            args.output.display(),
            sectors,
            sectors as u64 * 2048
        );
    } else {
        println!("Created: {}", args.output.display());
    }

    Ok(())
}

/// Recursively build a SimpleDir tree from a filesystem path.
/// Returns the total number of files added.
fn build_dir(path: &Path, dir: &mut SimpleDir, verbose: bool) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if file_type.is_dir() {
            let mut subdir = SimpleDir::new(&name);
            count += build_dir(&entry.path(), &mut subdir, verbose)?;
            dir.add_dir(subdir);
        } else if file_type.is_file() {
            if verbose {
                println!("  Adding: {}", entry.path().display());
            }
            let data = fs::read(entry.path())?;
            dir.add_file(SimpleFile::new(name, data));
            count += 1;
        }
    }
    Ok(count)
}

/// Parse a UDF revision string like "1.02" into a UdfRevision.
fn parse_revision(s: &str) -> Result<UdfRevision> {
    let err = || -> Box<dyn std::error::Error> {
        format!("invalid UDF revision '{}': expected format like 1.02, 2.50", s).into()
    };
    let (major_str, minor_str) = s.split_once('.').ok_or_else(err)?;
    let major = major_str.parse::<u8>().map_err(|_| err())?;
    let minor = u8::from_str_radix(minor_str, 16).map_err(|_| err())?;
    Ok(UdfRevision::from_raw((major as u16) << 8 | minor as u16))
}
