//! The ISO 9660 and UDF bridge image, planned without I/O.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use hadris_fs::{FileType, PathError, Report, Tree};
use hadris_iso::{IsoOptions, PreparedIso};

use crate::error::Detail;
use crate::options::UdfOptions;
use crate::plan::{self, ContentInfo, Plan, SECTOR};

/// Blocks the UDF volume keeps after the last file, before its trailing
/// anchor, as 2.x did.
const SLACK: u64 = 100;
/// The trailing anchor and the 256 blocks after it.
const TAIL: u64 = 257;

/// Both halves of a bridge image.
pub(crate) struct BridgePlan<'tree> {
    pub(crate) iso: PreparedIso<'tree>,
    pub(crate) udf: Plan,
}

/// The ISO 9660 volume descriptors `opts` writes, the terminator included.
fn iso_descriptors(opts: &IsoOptions) -> u32 {
    2 + u32::from(opts.joliet()) + u32::from(opts.iso1999()) + u32::from(opts.el_torito().is_some())
}

fn stored_contents(tree: &Tree, iso: &Report) -> Result<BTreeMap<usize, ContentInfo>, PathError> {
    let mut out = BTreeMap::new();
    let mut pending = vec![(tree.root(), Vec::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for (name, child) in dir.children() {
            let mut path = prefix.clone();
            path.push(b'/');
            path.extend_from_slice(name.as_bytes());
            let node = child.node();
            if node.file_type() == FileType::Dir {
                pending.push((child, path));
                continue;
            }
            let Some(content) = node.content() else {
                continue;
            };
            if out.contains_key(&child.id()) {
                continue;
            }
            let info = match iso.extents(&path) {
                Some(extents) => ContentInfo {
                    len: content.len(),
                    stored: Some(extents.to_vec()),
                },
                None if content.is_empty() => ContentInfo {
                    len: 0,
                    stored: None,
                },
                None => {
                    return Err(
                        PathError::from(Detail::Content.corrupt::<Infallible>()).with_path(path)
                    );
                }
            };
            out.insert(child.id(), info);
        }
    }
    Ok(out)
}

/// Plans a bridge image: the UDF metadata first, the ISO 9660 image after
/// it, then the UDF volume over the ISO 9660 file extents.
pub(crate) fn plan_both<'tree>(
    tree: &'tree Tree,
    iso: &IsoOptions,
    udf: &UdfOptions,
) -> Result<BridgePlan<'tree>, PathError> {
    let descriptors = iso_descriptors(iso);
    let bridge = udf.clone().with_min_blocks(0);
    let first = plan::lay_out(
        tree,
        &bridge,
        &plan::measure(tree, true, false)?,
        false,
        Some(descriptors),
    )?;
    let iso = iso
        .clone()
        .with_min_blocks(iso.min_blocks().max(first.allocated_end));
    let natural = hadris_iso::plan(tree, &iso)?;
    let files_end = natural
        .files()
        .flat_map(|(_, extents)| extents)
        .map(|extent| extent.end().div_ceil(SECTOR as u64))
        .fold(first.allocated_end, u64::max);
    let total = (natural.size() / SECTOR as u64 + TAIL)
        .max(files_end + SLACK + TAIL)
        .max(iso.min_image_blocks());
    let iso = iso.with_min_image_blocks(total);
    let iso = PreparedIso::new(tree, &iso)?;
    let contents = stored_contents(tree, iso.report())?;
    let udf = plan::lay_out(
        tree,
        &bridge.with_min_blocks(total),
        &contents,
        true,
        Some(descriptors),
    )?;
    Ok(BridgePlan { iso, udf })
}

impl BridgePlan<'_> {
    /// The report of the image: its size, the warnings of the ISO 9660
    /// writer then those of the UDF writer, and the file extents both
    /// trees share.
    pub(crate) fn report(&self) -> Report {
        let mut report = Report::new();
        report.set_size(self.udf.report.size().max(self.iso.report().size()));
        for warning in self
            .iso
            .report()
            .warnings()
            .iter()
            .chain(self.udf.report.warnings())
        {
            report.push_warning(warning.clone());
        }
        for (path, extents) in self.iso.report().files() {
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
/// metadata, sizes the UDF volume itself and raises
/// [`IsoOptions::with_min_image_blocks`] to the whole image. The report has the image size,
/// the warnings of the ISO 9660 writer then those of the UDF writer, and
/// the extents of each file.
///
/// # Errors
///
/// Propagates validation and size-limit errors from either format's planner.
/// Stored content must use whole, aligned blocks after the UDF metadata;
/// invalid extents return [`Detail::StoredContent`] with kind
/// [`ErrorKind::InvalidInput`](hadris_fs::ErrorKind::InvalidInput).
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(target = "hadris::udf", level = "trace", skip_all)
)]
pub fn plan_bridge(tree: &Tree, iso: &IsoOptions, udf: &UdfOptions) -> Result<Report, PathError> {
    Ok(plan_both(tree, iso, udf)?.report())
}
