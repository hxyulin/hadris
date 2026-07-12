use std::fs::File;
use std::io::{BufReader, Read, Seek};

use hadris_iso::directory::{DirectoryRef, FileFlags};
use hadris_iso::read::IsoImage;

use crate::args::TreeArgs;

use super::{Result, clean_name, navigate_to_path};

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    println!("{}", args.path);

    let target = navigate_to_path(&iso, &args.path)?;
    let max_depth = args.depth.unwrap_or(usize::MAX);

    print_tree_recursive(&iso, target, "", 0, max_depth)?;

    Ok(())
}

fn print_tree_recursive<R: Read + Seek>(
    iso: &IsoImage<R>,
    dir_ref: DirectoryRef,
    prefix: &str,
    depth: usize,
    max_depth: usize,
) -> Result<()> {
    if depth >= max_depth {
        return Ok(());
    }

    let dir = iso.open_dir(dir_ref);
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    for (idx, entry) in entries.iter().enumerate() {
        let is_last_entry = idx == entries.len() - 1;
        let connector = if is_last_entry {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let display_name = clean_name(entry.name());

        let flags = FileFlags::from_bits_truncate(entry.header().flags);
        let suffix = if flags.contains(FileFlags::DIRECTORY) {
            "/"
        } else {
            ""
        };

        println!("{}{}{}{}", prefix, connector, display_name, suffix);

        if flags.contains(FileFlags::DIRECTORY)
            && let Ok(child_ref) = entry.as_dir_ref(iso)
        {
            let new_prefix = if is_last_entry {
                format!("{}    ", prefix)
            } else {
                format!("{}\u{2502}   ", prefix)
            };
            print_tree_recursive(iso, child_ref, &new_prefix, depth + 1, max_depth)?;
        }
    }

    Ok(())
}
