use std::fs;

use super::super::args::ExtractArgs;
use super::{Result, open};

/// Extract files from a UDF image
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    fs::create_dir_all(&args.output)?;
    let from = args.path.as_deref().unwrap_or("/");
    if args.verbose {
        println!("Extracting {from} to {}", args.output.display());
    }
    hadris_fs::sync::extract_to_host(&mut udf, from, &args.output)?;
    let count = walkdir_count(&args.output)?;
    println!("Extracted {count} files to {}", args.output.display());
    Ok(())
}

fn walkdir_count(path: &std::path::Path) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            count += walkdir_count(&entry.path())?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}
