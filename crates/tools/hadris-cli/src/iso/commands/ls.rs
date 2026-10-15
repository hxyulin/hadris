use anyhow::Result;

use super::super::args::LsArgs;

use super::{first_block, list_dir, view_for};
use crate::common::type_char;

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let mut iso = args.locate.open(&args.input)?;
    let mut view = view_for(&mut iso, &args.path)?;
    let entries = list_dir(&mut view, &args.path)?;

    if args.all {
        for name in [".", ".."] {
            if args.long {
                println!("d  {:>10}  {:>8}  {name}", "", "");
            } else {
                println!("{name}/");
            }
        }
    }

    for entry in entries {
        let file_type = entry.meta.file_type();
        if args.long {
            let block = first_block(&mut view, entry.node)?;
            println!(
                "{}  {:>10}  {block:>8}  {}",
                type_char(file_type),
                entry.meta.len(),
                entry.name
            );
        } else if file_type.is_dir() {
            println!("{}/", entry.name);
        } else {
            println!("{}", entry.name);
        }
    }

    Ok(())
}
