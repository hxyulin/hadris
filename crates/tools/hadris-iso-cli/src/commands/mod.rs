//! Command implementations for hadris-iso CLI

mod cat;
mod create;
mod extract;
mod info;
mod ls;
mod mkisofs;
mod tree;
mod verify;

pub use cat::cat;
pub use create::create;
pub use extract::extract;
pub use info::info;
pub use ls::ls;
pub use mkisofs::mkisofs;
pub use tree::tree;
pub use verify::verify;

use std::fs::File;
use std::path::Path;

use hadris_fs::sync::DriverExt;
use hadris_fs::tree::{FromFsOptions, Tree, WarningKind};
use hadris_fs::{Metadata, NodeId, SystemClock};
use hadris_iso::sync::{IsoImage, IsoView, write};
use hadris_iso::{IsoOptions, Namespace, Report};

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

type View<'a> = IsoView<&'a mut File>;

fn open(path: &Path) -> Result<IsoImage<File>> {
    Ok(IsoImage::open(File::open(path)?)?)
}

/// The most capable tree of the image: Rock Ridge, then Joliet, then the
/// enhanced tree, then the primary tree.
fn preferred_view(iso: &mut IsoImage<File>) -> Result<View<'_>> {
    Ok(iso.view(Namespace::Preferred)?)
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// One directory entry: its name, node and metadata.
struct Entry {
    name: String,
    node: NodeId,
    meta: Metadata,
}

/// The entries of the directory at `path`, in directory order.
fn list_dir(view: &mut View<'_>, path: &str) -> Result<Vec<Entry>> {
    let mut items = Vec::new();
    for item in view
        .read_dir(path)
        .map_err(|err| format!("Directory not found: {path}: {err}"))?
    {
        let item = item?;
        items.push((
            String::from_utf8_lossy(item.name_bytes()).into_owned(),
            item.entry().node(),
        ));
    }
    items
        .into_iter()
        .map(|(name, node)| {
            let meta = view.node_metadata(node)?;
            Ok(Entry { name, node, meta })
        })
        .collect()
}

/// The first logical block of a node's data.
fn first_block(view: &mut View<'_>, node: NodeId) -> Result<u64> {
    let mut first = None;
    view.extents(node, |extent| {
        first.get_or_insert(extent.offset());
    })?;
    Ok(first.unwrap_or(0) / u64::from(view.block_size()))
}

/// Reads the host directory `source` into a tree, reporting what it skipped.
fn read_source(source: &Path) -> Result<Tree> {
    let tree = Tree::from_fs(source, FromFsOptions::new())?;
    for warning in tree.warnings() {
        eprintln!("warning: {warning}");
    }
    Ok(tree)
}

/// Prints the writer's warnings. Dropped metadata, which every image without
/// Rock Ridge reports, only when `verbose`.
fn print_warnings(report: &Report, verbose: bool) {
    for warning in report.warnings() {
        if verbose || warning.kind() != WarningKind::IgnoredMetadata {
            eprintln!("warning: {warning}");
        }
    }
}

/// Writes `tree` to the file `output`, padded to at least 32 blocks.
fn write_image(
    output: &Path,
    tree: &Tree,
    options: &IsoOptions<SystemClock>,
    verbose: bool,
) -> Result<Report> {
    let mut file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    let report = write(&mut file, tree, options)?;
    print_warnings(&report, verbose);
    let min_size = 32 * 2048;
    if file.metadata()?.len() < min_size {
        file.set_len(min_size)?;
    }
    Ok(report)
}

/// Normalize a path to use forward slashes (ISO 9660 standard).
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}
