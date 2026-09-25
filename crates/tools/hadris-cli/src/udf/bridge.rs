//! `hadris udf bridge` and `hadris udf compare`: ISO 9660 and UDF bridge
//! images.

use std::collections::BTreeMap;
use std::hash::Hasher;
use std::path::Path;

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, FileType, MountOptions, NodeId, OpenMode, Resolve, WarningKind};
use hadris_iso::Namespace;
use hadris_iso::sync::IsoFs;
use hadris_storage::host::FileDevice;
use hadris_udf::sync::UdfFs;
use hadris_udf::{UdfId, UdfOptions};

use super::args::BridgeArgs;
use crate::common::{build_time, read_source};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Writes a bridge image of the host directory `args.source`: ISO 9660
/// with the `iso create` options, and UDF over the same file data.
pub fn create(args: BridgeArgs) -> Result<()> {
    let output = &args.target.output;
    let tree = read_source(&args.source)?;
    let time = build_time()?;
    let iso = crate::iso::iso_options(&args.iso, time);
    let udf = UdfOptions::default()
        .with_id(UdfId::Volume, &args.iso.volume_name)
        .with_revision(args.revision.0)
        .with_time(time);

    if args.dry_run {
        let report = hadris_udf::plan_bridge(&tree, &iso, &udf)?;
        println!(
            "Estimated size: {} bytes ({} sectors)",
            report.size(),
            report.size() / 2048
        );
        return Ok(());
    }

    let (file, pending) = args.target.create()?;
    let mut dev = FileDevice::new(file)
        .map_err(|err| format!("cannot create {}: {err}", output.display()))?;
    let report = hadris_udf::sync::write_bridge(&mut dev, &tree, &iso, &udf)?;
    pending
        .commit(dev.into_inner())
        .map_err(|err| format!("cannot write {}: {err}", output.display()))?;
    for warning in report.warnings() {
        if args.verbose || !matches!(warning.kind(), WarningKind::Dropped(_)) {
            eprintln!("warning: {warning}");
        }
    }
    println!("Created: {}", output.display());
    Ok(())
}

/// A directory, or a file's length and a hash of its contents.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Directory,
    File(u64, u64),
}

/// Compares the ISO 9660 and UDF trees of the bridge image at `path`.
pub fn compare(path: &Path) -> Result<()> {
    let mut iso = FileDevice::open(path)?;
    IsoFs::mount(&mut iso, MountOptions::new())
        .map_err(|error| format!("ISO namespace is not readable: {}", error.error()))?;
    let mut udf = UdfFs::mount(FileDevice::open(path)?, MountOptions::new())
        .map_err(|error| format!("UDF namespace is not readable: {error}"))?;

    let mut view = IsoFs::mount(&mut iso, MountOptions::new()).map_err(|err| err.into_parts().0)?;
    let namespace = view.namespace();
    let names_match = namespace != Namespace::Primary;
    let mut iso_nodes = BTreeMap::new();
    let root = view.root();
    collect(&mut view, root, "", &mut iso_nodes)?;
    let mut udf_nodes = BTreeMap::new();
    let root = udf.root();
    collect(&mut udf, root, "", &mut udf_nodes)?;
    if namespace == Namespace::RockRidge {
        for name in relocation_dirs(&mut iso, &iso_nodes, &udf_nodes)? {
            iso_nodes.remove(&name);
        }
    }

    if !names_match {
        println!("  ISO tree has only ISO 9660 identifiers; comparing contents, not names");
        let mut iso_contents: Vec<_> = iso_nodes.into_values().collect();
        let mut udf_contents: Vec<_> = udf_nodes.into_values().collect();
        iso_contents.sort();
        udf_contents.sort();
        if iso_contents != udf_contents {
            return Err("ISO and UDF namespaces differ".into());
        }
        println!(
            "Verified: {} ({} shared entries)",
            path.display(),
            iso_contents.len()
        );
        return Ok(());
    }

    if iso_nodes != udf_nodes {
        for key in iso_nodes.keys().chain(udf_nodes.keys()) {
            if iso_nodes.get(key) != udf_nodes.get(key) {
                eprintln!("  mismatch: {key}");
            }
        }
        return Err("ISO and UDF namespaces differ".into());
    }
    println!(
        "Verified: {} ({} shared entries)",
        path.display(),
        iso_nodes.len()
    );
    Ok(())
}

/// Root directories of the Rock Ridge tree that hold relocated deep
/// directories: absent from UDF, empty in the Rock Ridge view because their
/// children are shown in their real place, and not empty in the primary tree.
fn relocation_dirs(
    iso: &mut FileDevice,
    iso_nodes: &BTreeMap<String, Node>,
    udf_nodes: &BTreeMap<String, Node>,
) -> Result<Vec<String>> {
    let candidates: Vec<&String> = iso_nodes
        .iter()
        .filter(|(path, node)| {
            **node == Node::Directory
                && !path.contains('/')
                && !udf_nodes.contains_key(*path)
                && !iso_nodes.keys().any(|other| {
                    other
                        .strip_prefix(path.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
        })
        .map(|(path, _)| path)
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut primary = IsoFs::mount_namespace(iso, MountOptions::new(), Namespace::Primary)
        .map_err(|err| err.into_parts().0)?;
    let mut found = Vec::new();
    for path in candidates {
        let dir = primary.resolve(format!("/{path}").as_bytes(), Resolve::Lexical)?;
        let listed = primary.readdir(dir, DirCursor::START);
        primary.forget(dir, 1);
        if listed?.is_some() {
            found.push(path.clone());
        }
    }
    Ok(found)
}

fn collect<F: FileSystem>(
    fs: &mut F,
    dir: NodeId,
    prefix: &str,
    nodes: &mut BTreeMap<String, Node>,
) -> Result<()> {
    let mut cursor = DirCursor::START;
    while let Some(entry) = fs.readdir(dir, cursor)? {
        cursor = entry.next_cursor();
        let path = join(prefix, &String::from_utf8_lossy(entry.name().as_bytes()));
        let node = fs.lookup(dir, entry.name())?;
        let result = match entry.file_type() {
            FileType::Dir => {
                nodes.insert(path.clone(), Node::Directory);
                collect(fs, node, &path, nodes)
            }
            FileType::File => hash_file(fs, node).map(|(len, hash)| {
                nodes.insert(path, Node::File(len, hash));
            }),
            _ => Ok(()),
        };
        fs.forget(node, 1);
        result?;
    }
    Ok(())
}

/// The length and hash of the contents of the pinned file `node`.
fn hash_file<F: FileSystem>(fs: &mut F, node: NodeId) -> Result<(u64, u64)> {
    fs.open(node, OpenMode::Read)?;
    let mut hasher = std::hash::DefaultHasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    let read = loop {
        match fs.read(node, len, &mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                hasher.write(&buf[..n]);
                len += n as u64;
            }
            Err(err) => break Err(err),
        }
    };
    let closed = fs.close(node);
    read?;
    closed?;
    Ok((len, hasher.finish()))
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}
