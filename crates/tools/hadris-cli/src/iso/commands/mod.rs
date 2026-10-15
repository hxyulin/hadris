//! Command implementations for `hadris iso`.

mod cat;
mod create;
mod extract;
mod info;
mod ls;
mod mkisofs;
mod tree;
mod verify;

pub use cat::cat;
pub use create::{create, iso_options};
pub use extract::extract;
pub use info::info;
pub use ls::ls;
pub use mkisofs::mkisofs;
pub use tree::tree;
pub use verify::verify;

use anyhow::{Context, Result};
use hadris_fs::sync::FileSystem;
use hadris_fs::{Metadata, NodeId, Report, Resolve, Tree};
use hadris_iso::sync::{IsoFs, write};
use hadris_iso::{IsoOptions, Namespace};

use crate::common::{Device, Target, print_warnings};
use crate::vfs::entries;

type View<'a> = IsoFs<&'a mut Device>;

/// The image on `dev`, read through the tree `namespace` names.
fn view(dev: &mut Device, namespace: Namespace) -> Result<View<'_>> {
    IsoFs::mount_namespace(dev, hadris_fs::MountOptions::new(), namespace)
        .map_err(|err| err.into_parts().0.into())
}

/// The most capable tree of the image (Rock Ridge, then Joliet, then the
/// enhanced tree, then the primary tree) when `path` is in it, and otherwise
/// the primary tree, whose lookups ignore ASCII case, so ISO 9660 paths such
/// as `/README.TXT` work too.
fn view_for<'a>(iso: &'a mut Device, path: &str) -> Result<View<'a>> {
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
        .with_context(|| format!("Directory not found: {path}"))?;
    let listed = list_node(view, dir);
    view.forget(dir, 1);
    listed
}

/// The entries of the directory `dir`, in directory order.
fn list_node(view: &mut View<'_>, dir: NodeId) -> Result<Vec<Entry>> {
    Ok(entries(view, dir)?
        .into_iter()
        .map(|entry| Entry {
            name: String::from_utf8_lossy(entry.name().as_bytes()).into_owned(),
            node: entry.node(),
            meta: *entry.metadata(),
        })
        .collect())
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

/// Writes `tree` to `output`, padded to at least 32 blocks. The file appears
/// only once the image is complete.
fn write_image(
    target: &Target,
    tree: &Tree,
    options: &IsoOptions,
    verbose: bool,
) -> Result<Report> {
    target.write_image(32 * 2048, |dev| {
        let report = write(dev, tree, options)?;
        print_warnings(&report, verbose);
        Ok(report)
    })
}

/// Normalize a path to use forward slashes (ISO 9660 standard).
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}
