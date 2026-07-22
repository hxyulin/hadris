use std::fs::{self, File};
use std::path::Path;

use hadris_udf::{UdfDir, UdfVolume};

use super::super::args::ExtractArgs;

use super::{Result, navigate_to_path};

/// Extract files from a UDF image
pub fn extract(args: ExtractArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let udf = UdfVolume::open(file)?;

    fs::create_dir_all(&args.output)?;

    let start = if let Some(ref path) = args.path {
        navigate_to_path(&udf, path)?
    } else {
        udf.root_dir()?
    };

    let mut extracted_count = 0;
    extract_dir(
        &udf,
        &start,
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

fn extract_dir(
    udf: &UdfVolume<File>,
    dir: &UdfDir,
    output_path: &Path,
    verbose: bool,
    count: &mut usize,
) -> Result<()> {
    for entry in dir.entries() {
        if entry.is_parent() {
            continue;
        }

        let name = entry.name();
        if entry.is_dir() {
            let child_path = output_path.join(name);
            fs::create_dir_all(&child_path)?;
            if verbose {
                println!("Creating directory: {}", child_path.display());
            }
            let child = udf.read_directory(&entry.icb)?;
            extract_dir(udf, &child, &child_path, verbose, count)?;
        } else {
            let file_path = output_path.join(name);
            if verbose {
                println!("Extracting: {} ({} bytes)", file_path.display(), entry.size);
            }
            let bytes = udf.read_file(entry)?;
            fs::write(&file_path, bytes)?;
            *count += 1;
        }
    }
    Ok(())
}
