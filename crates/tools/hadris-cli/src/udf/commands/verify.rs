use std::io;

use anyhow::{Result, anyhow};
use hadris_fs::sync::FileSystem;
use hadris_fs::{FileType, NodeId};

use super::super::args::VerifyArgs;
use super::{Udf, copy_node, list};
use crate::common::{Ancestors, join};

/// Verify UDF image structural integrity: open the volume, then walk the
/// whole tree and read every file.
pub fn verify(args: VerifyArgs) -> Result<()> {
    println!("Verifying: {}", args.input.display());

    let mut udf = match super::open(&args.input, &args.locate) {
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
    let root = udf.root();
    walk(
        &mut udf,
        root,
        "/",
        &mut Ancestors::new(root),
        args.verbose,
        &mut tally,
    );
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
    Err(anyhow!("{} error(s) found", tally.errors.len()))
}

#[derive(Default)]
struct Tally {
    files: usize,
    dirs: usize,
    errors: Vec<String>,
}

fn walk(
    udf: &mut Udf,
    dir: NodeId,
    path: &str,
    ancestors: &mut Ancestors,
    verbose: bool,
    tally: &mut Tally,
) {
    let items = match list(udf, dir) {
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
                match ancestors.enter(item.node(), &child) {
                    Ok(()) => {
                        walk(udf, item.node(), &child, ancestors, verbose, tally);
                        ancestors.leave();
                    }
                    Err(e) => tally.errors.push(e.to_string()),
                }
            }
            FileType::File => {
                tally.files += 1;
                let read = udf
                    .lookup(dir, item.name())
                    .map_err(Into::into)
                    .and_then(|node| {
                        let copied = copy_node(udf, node, &mut io::sink());
                        udf.forget(node, 1);
                        copied
                    });
                if let Err(e) = read {
                    tally.errors.push(format!("{child}: {e}"));
                }
            }
            _ => tally.files += 1,
        }
    }
}
