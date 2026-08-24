use std::fs;
use std::path::Path;

use hadris_udf::UdfRevision;
use hadris_udf::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

use super::super::args::CreateArgs;

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
        println!("Found {file_count} files");
    }

    if args.dry_run {
        println!("Dry run: would create UDF image");
        println!("  Volume name: {}", args.volume_name);
        println!("  UDF revision: {revision}");
        println!("  Files: {file_count}");
        return Ok(());
    }

    let options = UdfWriteOptions {
        volume_id: args.volume_name.clone(),
        revision,
        ..UdfWriteOptions::default()
    };

    let output_file = fs::File::create(&args.output)?;
    let sectors = UdfWriter::create(output_file, &root, options)?.sectors_written;

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

/// Parse a UDF revision string like "1.02" into a supported UdfRevision.
fn parse_revision(s: &str) -> Result<UdfRevision> {
    match s {
        "1.02" => Ok(UdfRevision::V1_02),
        "1.50" => Ok(UdfRevision::V1_50),
        "2.00" => Ok(UdfRevision::V2_00),
        "2.01" => Ok(UdfRevision::V2_01),
        "2.50" => Ok(UdfRevision::V2_50),
        "2.60" => Ok(UdfRevision::V2_60),
        _ => Err(format!(
            "invalid UDF revision '{s}': expected 1.02, 1.50, 2.00, 2.01, 2.50, or 2.60"
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_revision_supported() {
        assert_eq!(parse_revision("1.02").unwrap(), UdfRevision::V1_02);
        assert_eq!(parse_revision("2.60").unwrap(), UdfRevision::V2_60);
    }

    #[test]
    fn test_parse_revision_rejects_unsupported() {
        for input in ["9.99", "1.03", "2.5", "abc", ""] {
            assert!(parse_revision(input).is_err(), "should reject {input:?}");
        }
    }
}
