use std::fs::File;

use hadris_udf::UdfFs;

use crate::args::LsArgs;

use super::{Result, navigate_to_path};

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let udf = UdfFs::open(file)?;

    let dir = navigate_to_path(&udf, &args.path)?;

    let entries: Vec<_> = if args.all {
        dir.all_entries().collect()
    } else {
        dir.entries().collect()
    };

    for entry in &entries {
        if entry.is_hidden() && !args.all {
            continue;
        }

        if args.long {
            let type_char = if entry.is_dir() { 'd' } else { '-' };
            // size is currently a placeholder (0) in the UDF library
            let size_str = if entry.size == 0 && entry.is_file() {
                "N/A".to_string()
            } else {
                entry.size.to_string()
            };
            println!("{}  {:>10}  {}", type_char, size_str, entry.name());
        } else if entry.is_dir() {
            println!("{}/", entry.name());
        } else {
            println!("{}", entry.name());
        }
    }

    Ok(())
}
