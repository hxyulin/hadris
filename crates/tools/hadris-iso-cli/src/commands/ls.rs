use hadris_fs::FileType;

use super::super::args::LsArgs;

use super::{Result, first_block, list_dir, open, preferred_view};

fn type_char(file_type: FileType) -> char {
    match file_type {
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        FileType::CharDevice => 'c',
        FileType::BlockDevice => 'b',
        FileType::Fifo => 'p',
        FileType::Socket => 's',
        _ => '-',
    }
}

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let mut iso = open(&args.input)?;
    let mut view = preferred_view(&mut iso)?;
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
