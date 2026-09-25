use std::fs;

use hadris_fs::sync::{FileSystem, Volume, read_tree};
use hadris_fs::{DirCursor, NodeId, Resolve, host};

use super::super::args::ExtractArgs;
use super::{Result, Udf, open};

/// Extract files from a UDF image. The root is merged into the output
/// directory; any other path lands at `<output>/<name>`.
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    let from = args.path.as_deref().unwrap_or("/");
    let name = stored_name(&mut udf, from)?;
    let node = udf.resolve(from.as_bytes(), Resolve::Lexical)?;
    let is_dir = udf.stat(node).map(|meta| meta.file_type().is_dir());
    udf.forget(node, 1);
    let (destination, target) = match (name, is_dir?) {
        (None, _) => (args.output.clone(), args.output.clone()),
        (Some(name), true) => (args.output.join(&name), args.output.join(name)),
        (Some(name), false) => (args.output.join(name), args.output.clone()),
    };
    if args.verbose {
        println!("Extracting {from} to {}", destination.display());
    }
    let vol = Volume::new(udf);
    let tree = read_tree(&vol, from).map_err(|err| format!("Failed to extract {from}: {err}"))?;
    fs::create_dir_all(&target)?;
    let report = host::write_tree(&target, &tree)
        .map_err(|err| format!("Failed to extract {from}: {err}"))?;
    for warning in report.warnings() {
        eprintln!("warning: {warning}");
    }
    println!("Extracted {from} to {}", destination.display());
    Ok(())
}

/// The name `path` is listed under in its directory, or `None` for the
/// root. Fails on names that are not one plain host path component.
fn stored_name(udf: &mut Udf, path: &str) -> Result<Option<String>> {
    let node = udf
        .resolve(path.as_bytes(), Resolve::Lexical)
        .map_err(|err| format!("Not found: {path}: {err}"))?;
    if node == udf.root() {
        udf.forget(node, 1);
        return Ok(None);
    }
    let found = find_name(udf, path, node);
    udf.forget(node, 1);
    let name = found?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(format!(
            "Refusing to extract {path}: its name {name:?} is not a plain file name"
        )
        .into());
    }
    Ok(Some(name))
}

/// Scans the parent of `path` for the entry listed with id `node`.
fn find_name(udf: &mut Udf, path: &str, node: NodeId) -> Result<String> {
    let parent = udf.resolve(format!("{path}/..").as_bytes(), Resolve::Lexical)?;
    let mut cursor = DirCursor::START;
    let found = loop {
        match udf.readdir(parent, cursor) {
            Ok(Some(entry)) if entry.node() == node => {
                break Ok(String::from_utf8_lossy(entry.name().as_bytes()).into_owned());
            }
            Ok(Some(entry)) => cursor = entry.next_cursor(),
            Ok(None) => break Err(format!("{path} is not listed in its directory").into()),
            Err(err) => break Err(err.into()),
        }
    };
    udf.forget(parent, 1);
    found
}
