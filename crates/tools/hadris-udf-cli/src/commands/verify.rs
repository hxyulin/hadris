use hadris_fs::sync::DriverExt;

use super::super::args::VerifyArgs;
use super::{Result, Udf, entries, join};

/// Verify UDF image structural integrity
pub fn verify(args: VerifyArgs) -> Result<()> {
    println!("Verifying: {}", args.input.display());

    let mut udf = match super::open(&args.input) {
        Ok(udf) => {
            println!("  [OK] Anchor Volume Descriptor Pointer");
            println!("  [OK] Volume Recognition Sequence");
            println!("  [OK] Volume Descriptor Sequence");
            println!("  [OK] File Set Descriptor");
            udf
        }
        Err(e) => {
            println!("  [FAIL] Could not open UDF image: {e}");
            return Err(e);
        }
    };
    println!("  Volume ID:   {}", udf.volume_id());
    println!("  UDF revision: {}", udf.revision());

    if let Err(e) = entries(&mut udf, "/") {
        println!("  [FAIL] Root directory: {e}");
        return Err(e);
    }
    println!("  [OK] Root directory readable");

    if args.verbose {
        let (mut files, mut dirs, mut errors) = (0usize, 0usize, 0usize);
        walk(&mut udf, "/", &mut files, &mut dirs, &mut errors);
        println!("  Directory tree: {files} files, {dirs} directories, {errors} errors");
        if errors > 0 {
            println!("  [WARN] {errors} entries could not be read");
        } else {
            println!("  [OK] Directory tree fully traversable");
        }
    }

    println!("Verification complete.");
    Ok(())
}

fn walk(udf: &mut Udf, path: &str, files: &mut usize, dirs: &mut usize, errors: &mut usize) {
    let Ok(items) = entries(udf, path) else {
        *errors += 1;
        return;
    };
    for item in items {
        let child = join(path, &String::from_utf8_lossy(item.name_bytes()));
        if item.file_type().is_dir() {
            *dirs += 1;
            walk(udf, &child, files, dirs, errors);
        } else {
            *files += 1;
            if item.file_type() == hadris_fs::FileType::File && udf.read_to_vec(&child).is_err() {
                *errors += 1;
            }
        }
    }
}
