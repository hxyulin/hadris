use std::fs;

use hadris_fs::sync::{FileSystem, extract_to_host};
use hadris_fs::{DirCursor, NodeId, Resolve};

use super::super::args::ExtractArgs;
use super::{Result, Udf, open};

/// Extract files from a UDF image. The root is merged into the output
/// directory; any other path lands at `<output>/<name>`.
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    let from = args.path.as_deref().unwrap_or("/");
    let destination = match stored_name(&mut udf, from)? {
        None => args.output.clone(),
        Some(name) => {
            fs::create_dir_all(&args.output)?;
            args.output.join(name)
        }
    };
    if args.verbose {
        println!("Extracting {from} to {}", destination.display());
    }
    extract_to_host(&mut udf, from, &destination)
        .map_err(|err| format!("Failed to extract {from}: {err}"))?;
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
