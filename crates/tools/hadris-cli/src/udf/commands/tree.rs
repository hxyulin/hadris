use hadris_fs::sync::FileSystem;
use hadris_fs::{DirEntry, Resolve};

use super::super::args::TreeArgs;
use super::{Result, Udf, join, list, open};
use crate::common::Ancestors;

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let mut udf = open(&args.input, &args.locate)?;
    println!("{}", args.path);
    if args.depth == Some(0) {
        return Ok(());
    }
    let top = udf.resolve(args.path.as_bytes(), Resolve::Lexical)?;
    let items = list(&mut udf, top);
    udf.forget(top, 1);
    let mut ancestors = Ancestors::new(top);
    print_tree(
        &mut udf,
        &mut ancestors,
        items?,
        &args.path,
        "",
        1,
        args.depth,
    )
}

/// Prints `items`, the entries of the directory at `path` and `depth`
/// levels below the top, and the directories below them.
fn print_tree(
    udf: &mut Udf,
    ancestors: &mut Ancestors,
    items: Vec<DirEntry>,
    path: &str,
    prefix: &str,
    depth: usize,
    max_depth: Option<usize>,
) -> Result<()> {
    let count = items.len();
    for (i, item) in items.into_iter().enumerate() {
        let is_last = i + 1 == count;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let name = String::from_utf8_lossy(item.name().as_bytes()).into_owned();
        let is_dir = item.file_type().is_dir();
        println!("{prefix}{connector}{name}{}", if is_dir { "/" } else { "" });
        if !is_dir || max_depth.is_some_and(|max| depth >= max) {
            continue;
        }
        let extension = if is_last { "    " } else { "\u{2502}   " };
        let child_prefix = format!("{prefix}{extension}");
        let child = join(path, &name);
        ancestors.enter(item.node(), &child)?;
        let printed = match list(udf, item.node()) {
            Ok(children) => print_tree(
                udf,
                ancestors,
                children,
                &child,
                &child_prefix,
                depth + 1,
                max_depth,
            ),
            Err(e) => {
                println!("{child_prefix}{connector}<error: {e}>");
                Ok(())
            }
        };
        ancestors.leave();
        printed?;
    }
    Ok(())
}
