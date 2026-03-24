use std::fs::File;

use hadris_udf::UdfFs;

use crate::args::VerifyArgs;

use super::Result;

/// Verify UDF image structural integrity
pub fn verify(args: VerifyArgs) -> Result<()> {
    println!("Verifying: {}", args.input.display());

    let file = File::open(&args.input)?;

    // Step 1: Open the image (validates VRS, AVDP, VDS, FSD)
    let udf = match UdfFs::open(file) {
        Ok(fs) => {
            println!("  [OK] Volume Recognition Sequence");
            println!("  [OK] Anchor Volume Descriptor Pointer");
            println!("  [OK] Volume Descriptor Sequence");
            println!("  [OK] File Set Descriptor");
            fs
        }
        Err(e) => {
            println!("  [FAIL] Could not open UDF image: {}", e);
            return Err(e.into());
        }
    };

    let info = udf.info();
    println!("  Volume ID:   {}", info.volume_id);
    println!("  UDF revision: {}", info.udf_revision);

    // Step 2: Validate root directory
    let root = match udf.root_dir() {
        Ok(dir) => {
            println!("  [OK] Root directory readable");
            dir
        }
        Err(e) => {
            println!("  [FAIL] Root directory: {}", e);
            return Err(e.into());
        }
    };

    // Step 3: Walk the full directory tree if verbose
    if args.verbose {
        let mut files = 0usize;
        let mut dirs = 0usize;
        let mut errors = 0usize;
        walk_tree(&udf, &root, &mut files, &mut dirs, &mut errors);
        println!("  Directory tree: {} files, {} directories, {} errors",
            files, dirs, errors);
        if errors > 0 {
            println!("  [WARN] {} directories could not be read", errors);
        } else {
            println!("  [OK] Directory tree fully traversable");
        }
    }

    println!("Verification complete.");
    Ok(())
}

fn walk_tree(
    udf: &UdfFs<File>,
    dir: &hadris_udf::UdfDir,
    files: &mut usize,
    dirs: &mut usize,
    errors: &mut usize,
) {
    for entry in dir.entries() {
        if entry.is_dir() {
            *dirs += 1;
            let icb = entry.icb;
            match udf.read_directory(&icb) {
                Ok(subdir) => walk_tree(udf, &subdir, files, dirs, errors),
                Err(_) => *errors += 1,
            }
        } else {
            *files += 1;
        }
    }
}
