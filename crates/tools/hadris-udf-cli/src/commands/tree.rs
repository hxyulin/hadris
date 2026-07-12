use std::fs::File;

use hadris_udf::{UdfDir, UdfFs};

use crate::args::TreeArgs;

use super::{Result, navigate_to_path};

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let udf = UdfFs::open(file)?;

    println!("{}", args.path);

    let root = navigate_to_path(&udf, &args.path)?;
    let max_depth = args.depth;

    print_tree(&udf, &root, "", 0, max_depth)?;

    Ok(())
}

fn print_tree(
    udf: &UdfFs<File>,
    dir: &UdfDir,
    prefix: &str,
    depth: usize,
    max_depth: Option<usize>,
) -> Result<()> {
    if let Some(max) = max_depth {
        if depth >= max {
            return Ok(());
        }
    }

    let entries: Vec<_> = dir.entries().collect();

    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == entries.len() - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let suffix = if entry.is_dir() { "/" } else { "" };
        println!("{}{}{}{}", prefix, connector, entry.name(), suffix);

        if entry.is_dir() {
            let extension = if is_last { "    " } else { "\u{2502}   " };
            let new_prefix = format!("{}{}", prefix, extension);
            let icb = entry.icb;
            match udf.read_directory(&icb) {
                Ok(subdir) => print_tree(udf, &subdir, &new_prefix, depth + 1, max_depth)?,
                Err(e) => println!("{}{}<error: {}>", new_prefix, connector, e),
            }
        }
    }

    Ok(())
}
