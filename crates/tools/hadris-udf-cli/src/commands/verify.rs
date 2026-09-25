use std::io;

use hadris_fs::FileType;

use super::super::args::VerifyArgs;
use super::{Result, Udf, copy_file, entries, join};

/// Verify UDF image structural integrity: open the volume, then walk the
/// whole tree and read every file.
pub fn verify(args: VerifyArgs) -> Result<()> {
    println!("Verifying: {}", args.input.display());

    let mut udf = match super::open(&args.input) {
        Ok(udf) => {
            println!("  [OK] Anchor, volume descriptors and file set descriptor");
            udf
        }
        Err(e) => {
            println!("  [FAIL] Could not open UDF image: {e}");
            return Err(e);
        }
    };
    println!(
        "  Volume ID:    {}",
        udf.info().id(hadris_udf::UdfId::Volume)
    );
    println!("  UDF revision: {}", udf.info().revision());

    let mut tally = Tally::default();
    walk(&mut udf, "/", args.verbose, &mut tally);
    println!(
        "  Directory tree: {} files, {} directories, {} errors",
        tally.files,
        tally.dirs,
        tally.errors.len()
    );
    if tally.errors.is_empty() {
        println!("Verification passed: No issues found");
        return Ok(());
    }
    for error in &tally.errors {
        println!("  [FAIL] {error}");
    }
    Err(format!("{} error(s) found", tally.errors.len()).into())
}

#[derive(Default)]
struct Tally {
    files: usize,
    dirs: usize,
    errors: Vec<String>,
}

fn walk(udf: &mut Udf, path: &str, verbose: bool, tally: &mut Tally) {
    let items = match entries(udf, path) {
        Ok(items) => items,
        Err(e) => {
            tally.errors.push(format!("{path}: {e}"));
            return;
        }
    };
    for item in items {
        let child = join(path, &String::from_utf8_lossy(item.name().as_bytes()));
        if verbose {
            println!("  {child}");
        }
        match item.file_type() {
            FileType::Dir => {
                tally.dirs += 1;
                walk(udf, &child, verbose, tally);
            }
            FileType::File => {
                tally.files += 1;
                let read = copy_file(udf, &child, &mut io::sink());
                if let Err(e) = read {
                    tally.errors.push(format!("{child}: {e}"));
                }
            }
            _ => tally.files += 1,
        }
    }
}
