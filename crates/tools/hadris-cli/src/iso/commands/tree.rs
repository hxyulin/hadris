use hadris_fs::sync::FileSystem;
use hadris_fs::{NodeId, Resolve};

use super::super::args::TreeArgs;
use super::{Entry, Result, View, join, list_node, open, view_for};
use crate::common::Ancestors;

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let mut iso = open(&args.input, &args.locate)?;
    let mut view = view_for(&mut iso, &args.path)?;

    println!("{}", args.path);
    if args.depth == Some(0) {
        return Ok(());
    }
    let top = view
        .resolve(args.path.as_bytes(), Resolve::Lexical)
        .map_err(|err| format!("Directory not found: {}: {err}", args.path))?;
    let entries = list_node(&mut view, top);
    view.forget(top, 1);
    let entries = entries?;
    let mut printer = Printer {
        view: &mut view,
        ancestors: Ancestors::new(top),
        max_depth: args.depth.unwrap_or(usize::MAX),
    };
    printer.print(entries, &args.path, "", 1)
}

struct Printer<'v, 'a> {
    view: &'v mut View<'a>,
    ancestors: Ancestors,
    max_depth: usize,
}

impl Printer<'_, '_> {
    fn print(&mut self, entries: Vec<Entry>, path: &str, prefix: &str, depth: usize) -> Result<()> {
        let count = entries.len();
        for (idx, entry) in entries.into_iter().enumerate() {
            let is_last_entry = idx + 1 == count;
            let connector = if is_last_entry {
                "\u{2514}\u{2500}\u{2500} "
            } else {
                "\u{251c}\u{2500}\u{2500} "
            };
            let is_dir = entry.meta.file_type().is_dir();
            let suffix = if is_dir { "/" } else { "" };
            println!("{prefix}{connector}{}{suffix}", entry.name);

            if is_dir && depth < self.max_depth {
                let new_prefix = if is_last_entry {
                    format!("{prefix}    ")
                } else {
                    format!("{prefix}\u{2502}   ")
                };
                let child = join(path, &entry.name);
                self.descend(entry.node, &child, &new_prefix, depth + 1)?;
            }
        }
        Ok(())
    }

    fn descend(&mut self, node: NodeId, path: &str, prefix: &str, depth: usize) -> Result<()> {
        self.ancestors.enter(node, path)?;
        let printed = list_node(self.view, node)
            .map_err(|err| format!("{path}: {err}").into())
            .and_then(|entries| self.print(entries, path, prefix, depth));
        self.ancestors.leave();
        printed
    }
}
