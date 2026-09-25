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

use std::path::Path;

use hadris_fs::host::{self, TreeOptions};
use hadris_fs::sync::FileSystem;
use hadris_fs::{
    Clock, DateTime, DirCursor, Metadata, NodeId, Report, Resolve, SystemClock, Tree, WarningKind,
};
use hadris_iso::sync::{IsoFs, write};
use hadris_iso::{IsoOptions, Namespace};
use hadris_storage::host::FileDevice;

use super::output::Output;

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

type View<'a> = IsoFs<&'a mut FileDevice>;

fn open(path: &Path) -> Result<FileDevice> {
    Ok(FileDevice::open(path)?)
}

/// The image on `dev`, read through the tree `namespace` names.
fn view(dev: &mut FileDevice, namespace: Namespace) -> Result<View<'_>> {
    IsoFs::mount_namespace(dev, hadris_fs::MountOptions::new(), namespace)
        .map_err(|err| err.into_parts().0.into())
}

/// The most capable tree of the image (Rock Ridge, then Joliet, then the
/// enhanced tree, then the primary tree) when `path` is in it, and otherwise
/// the primary tree, whose lookups ignore ASCII case, so ISO 9660 paths such
/// as `/README.TXT` work too.
fn view_for<'a>(iso: &'a mut FileDevice, path: &str) -> Result<View<'a>> {
    let mut preferred_view = view(iso, Namespace::Preferred)?;
    let preferred = match preferred_view.resolve(path.as_bytes(), Resolve::Lexical) {
        Ok(node) => {
            preferred_view.forget(node, 1);
            true
        }
        Err(_) => false,
    };
    let namespace = if preferred {
        Namespace::Preferred
    } else {
        Namespace::Primary
    };
    view(iso, namespace)
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
    let dir = view
        .resolve(path.as_bytes(), Resolve::Lexical)
        .map_err(|err| format!("Directory not found: {path}: {err}"))?;
    let mut entries = Vec::new();
    let mut cursor = DirCursor::START;
    let listed = loop {
        match view.readdir(dir, cursor) {
            Ok(Some(entry)) => {
                cursor = entry.next_cursor();
                entries.push(Entry {
                    name: String::from_utf8_lossy(entry.name().as_bytes()).into_owned(),
                    node: entry.node(),
                    meta: *entry.metadata(),
                });
            }
            Ok(None) => break Ok(entries),
            Err(err) => break Err(err.into()),
        }
    };
    view.forget(dir, 1);
    listed
}

/// The first logical block of a node's data.
fn first_block(view: &mut View<'_>, node: NodeId) -> Result<u64> {
    let mut first = None;
    let mut out = [hadris_fs::Extent::new(0, 0); 1];
    if view.extents(node, 0, &mut out)? == 1 {
        first = Some(out[0].offset());
    }
    Ok(first.unwrap_or(0) / u64::from(view.info().block_size()))
}

/// Reads the host directory `source` into a tree, reporting what it skipped.
fn read_source(source: &Path) -> Result<Tree> {
    let (tree, _) = host::read_tree(source, &TreeOptions::new())?;
    Ok(tree)
}

/// The time new images are dated with: `SOURCE_DATE_EPOCH` when set, the
/// system time otherwise.
fn build_time() -> Result<DateTime> {
    Ok(host::source_date_epoch()?.unwrap_or_else(|| SystemClock.now()))
}

/// Prints the writer's warnings. Dropped metadata, which every image without
/// Rock Ridge reports, only when `verbose`.
fn print_warnings(report: &Report, verbose: bool) {
    for warning in report.warnings() {
        if verbose || !matches!(warning.kind(), WarningKind::Dropped(_)) {
            eprintln!("warning: {warning}");
        }
    }
}

/// Writes `tree` to `output`, padded to at least 32 blocks. The file appears
/// only once the image is complete.
fn write_image(output: &Path, tree: &Tree, options: &IsoOptions, verbose: bool) -> Result<Report> {
    let (file, pending) = Output::create(output)
        .map_err(|err| format!("cannot create {}: {err}", output.display()))?;
    let mut dev = FileDevice::new(file)
        .map_err(|err| format!("cannot create {}: {err}", output.display()))?;
    let report = write(&mut dev, tree, options)?;
    let file = dev.into_inner();
    print_warnings(&report, verbose);
    let min_size = 32 * 2048;
    if pending.is_regular() && file.metadata()?.len() < min_size {
        file.set_len(min_size)?;
    }
    pending
        .commit(file)
        .map_err(|err| format!("cannot write {}: {err}", output.display()))?;
    Ok(report)
}

/// Normalize a path to use forward slashes (ISO 9660 standard).
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}
