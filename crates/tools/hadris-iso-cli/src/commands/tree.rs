use super::super::args::TreeArgs;

use super::{Result, View, join, list_dir, open, preferred_view};

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let mut iso = open(&args.input)?;
    let mut view = preferred_view(&mut iso)?;

    println!("{}", args.path);
    let max_depth = args.depth.unwrap_or(usize::MAX);
    print_tree_recursive(&mut view, &args.path, "", 0, max_depth)
}

fn print_tree_recursive(
    view: &mut View<'_>,
    path: &str,
    prefix: &str,
    depth: usize,
    max_depth: usize,
) -> Result<()> {
    if depth >= max_depth {
        return Ok(());
    }

    let entries = list_dir(view, path)?;
    for (idx, entry) in entries.iter().enumerate() {
        let is_last_entry = idx == entries.len() - 1;
        let connector = if is_last_entry {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let is_dir = entry.meta.file_type().is_dir();
        let suffix = if is_dir { "/" } else { "" };
        println!("{prefix}{connector}{}{suffix}", entry.name);

        if is_dir {
            let new_prefix = if is_last_entry {
                format!("{prefix}    ")
            } else {
                format!("{prefix}\u{2502}   ")
            };
            print_tree_recursive(
                view,
                &join(path, &entry.name),
                &new_prefix,
                depth + 1,
                max_depth,
            )?;
        }
    }

    Ok(())
}
