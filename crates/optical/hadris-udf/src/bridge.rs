//! The ISO 9660 and UDF bridge image, planned without I/O.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use hadris_fs::{Content, FileType, Node, PathError, Report, Tree};
use hadris_iso::IsoOptions;

use crate::error::Detail;
use crate::options::UdfOptions;
use crate::plan::{Plan, SECTOR, lay_out, measure};

/// Blocks the UDF volume keeps after the last file, before its trailing
/// anchor, as 2.x did.
const SLACK: u64 = 100;
/// The trailing anchor and the 256 blocks after it.
const TAIL: u64 = 257;

/// Both halves of a bridge image.
pub(crate) struct BridgePlan {
    /// The ISO 9660 options, with the room the UDF metadata needs.
    pub(crate) iso: IsoOptions,
    pub(crate) iso_report: Report,
    /// The tree with every file pointing at its ISO 9660 extents.
    pub(crate) stored: Tree,
    pub(crate) udf: Plan,
    /// The ISO 9660 volume descriptors, the terminator included.
    pub(crate) descriptors: u32,
}

/// The ISO 9660 volume descriptors `opts` writes, the terminator included.
fn iso_descriptors(opts: &IsoOptions) -> u32 {
    2 + u32::from(opts.joliet()) + u32::from(opts.iso1999()) + u32::from(opts.el_torito().is_some())
}

/// The files of `tree` as the extents the ISO 9660 writer stores them in,
/// with the same directories, symlinks, special files, hard links and
/// attributes. Also returns the block after the last file.
fn stored(tree: &Tree, iso: &Report) -> Result<(Tree, u64), PathError> {
    let mut out = Tree::new();
    out.replace("", Node::dir().with_attrs(*tree.root().node().attrs()))?;
    let mut end = 0u64;
    let mut first: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut pending = vec![(tree.root(), Vec::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for (name, child) in dir.children() {
            let mut path = prefix.clone();
            path.push(b'/');
            path.extend_from_slice(name.as_bytes());
            if let Some(target) = first.get(&child.id()) {
                out.link(target, &path)?;
                continue;
            }
            let node = child.node();
            let new = match node.content() {
                Some(content) => {
                    let content = match iso.extents(&path) {
                        Some(extents) => {
                            for extent in extents {
                                end = end.max(extent.end().div_ceil(SECTOR as u64));
                            }
                            Content::stored(extents.to_vec())
                                .map_err(|error| error.with_path(&path))?
                        }
                        None if content.is_empty() => Content::empty(),
                        None => {
                            return Err(PathError::from(Detail::Content.corrupt::<Infallible>())
                                .with_path(path));
                        }
                    };
                    Node::file(content).with_attrs(*node.attrs())
                }
                None => node.clone(),
            };
            out.insert(&path, new)?;
            if node.file_type() == FileType::Dir {
                pending.push((child, path.clone()));
            }
            if child.links() > 1 {
                first.insert(child.id(), path);
            }
        }
    }
    Ok((out, end))
}

/// Plans a bridge image: the UDF metadata first, the ISO 9660 image after
/// it, then the UDF volume over the ISO 9660 file extents.
pub(crate) fn plan_both(
    tree: &Tree,
    iso: &IsoOptions,
    udf: &UdfOptions,
) -> Result<BridgePlan, PathError> {
    let descriptors = iso_descriptors(iso);
    let bridge = udf.clone().with_min_blocks(0);
    let first = lay_out(
        tree,
        &bridge,
        &measure(tree, true, false)?,
        false,
        Some(descriptors),
    )?;
    let iso = iso
        .clone()
        .with_min_blocks(iso.min_blocks().max(first.allocated_end));
    let iso_report = hadris_iso::plan(tree, &iso)?;
    let (stored, files_end) = stored(tree, &iso_report)?;
    let files_end = files_end.max(first.allocated_end);
    let total = (iso_report.size() / SECTOR as u64 + TAIL).max(files_end + SLACK + TAIL);
    let contents = measure(&stored, true, true)?;
    let udf = lay_out(
        &stored,
        &bridge.with_min_blocks(total),
        &contents,
        true,
        Some(descriptors),
    )?;
    Ok(BridgePlan {
        iso,
        iso_report,
        stored,
        udf,
        descriptors,
    })
}

impl BridgePlan {
    /// The report of the image: its size, the warnings of the ISO 9660
    /// writer then those of the UDF writer, and the file extents both
    /// trees share.
    pub(crate) fn report(&self) -> Report {
        let mut report = Report::new();
        report.set_size(self.udf.report.size().max(self.iso_report.size()));
        for warning in self
            .iso_report
            .warnings()
            .iter()
            .chain(self.udf.report.warnings())
        {
            report.push_warning(warning.clone());
        }
        for (path, extents) in self.iso_report.files() {
            for extent in extents {
                report.push_extent(path.as_bytes(), *extent);
            }
        }
        report
    }
}

/// Plans an ISO 9660 and UDF bridge image of `tree`, as DVD-Video and
/// `mkisofs -udf` write, without I/O, and returns the report
/// `write_bridge` returns.
///
/// Both file systems point at the same file data. `iso` and `udf` set each
/// volume as `hadris_iso::plan` and [`plan`](crate::plan) take them; the
/// writer raises [`IsoOptions::with_min_blocks`] to leave room for the UDF
/// metadata and sizes the UDF volume itself. The report has the image size,
/// the warnings of the ISO 9660 writer then those of the UDF writer, and
/// the extents of each file. Fails as either `plan` does.
pub fn plan_bridge(tree: &Tree, iso: &IsoOptions, udf: &UdfOptions) -> Result<Report, PathError> {
    Ok(plan_both(tree, iso, udf)?.report())
}
