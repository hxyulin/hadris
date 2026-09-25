use super::super::args::TreeArgs;
use super::{Result, Udf, entries, join, open};

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    println!("{}", args.path);
    print_tree(&mut udf, &args.path, "", 0, args.depth)
}

fn print_tree(
    udf: &mut Udf,
    path: &str,
    prefix: &str,
    depth: usize,
    max_depth: Option<usize>,
) -> Result<()> {
    if max_depth.is_some_and(|max| depth >= max) {
        return Ok(());
    }
    let items = entries(udf, path)?;
    for (i, item) in items.iter().enumerate() {
        let is_last = i + 1 == items.len();
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let name = String::from_utf8_lossy(item.name().as_bytes()).into_owned();
        let is_dir = item.file_type().is_dir();
        println!("{prefix}{connector}{name}{}", if is_dir { "/" } else { "" });
        if is_dir {
            let extension = if is_last { "    " } else { "\u{2502}   " };
            let child_prefix = format!("{prefix}{extension}");
            if let Err(e) = print_tree(udf, &join(path, &name), &child_prefix, depth + 1, max_depth)
            {
                println!("{child_prefix}{connector}<error: {e}>");
            }
        }
    }
    Ok(())
}
