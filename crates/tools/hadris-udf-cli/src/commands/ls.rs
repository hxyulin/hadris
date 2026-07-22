use std::fs::File;

use hadris_udf::UdfVolume;

use super::super::args::LsArgs;

use super::{Result, navigate_to_path};

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let udf = UdfVolume::open(file)?;

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
            println!("{}  {:>10}  {}", type_char, entry.size, entry.name());
        } else if entry.is_dir() {
            println!("{}/", entry.name());
        } else {
            println!("{}", entry.name());
        }
    }

    Ok(())
}
