use core::convert::Infallible;

use hadris_fs::Clock;
use hadris_fs::tree::Tree;
use hadris_iso::IsoOptions;
use hadris_udf::{Bridge, UdfOptions};

use super::storage::BlockDevice;
use crate::error::Error;
use crate::options::CdOptions;
use crate::report::Report;

/// Blocks the UDF volume keeps after the last file, before its trailing
/// anchor, as 2.x did.
const SLACK: u64 = 100;
/// The trailing anchor and the 256 blocks after it.
const TAIL: u64 = 257;

/// The ISO 9660 volume descriptors `opts` writes, the terminator included.
fn iso_descriptors<C: Clock>(opts: &IsoOptions<C>) -> u32 {
    2 + u32::from(opts.joliet().is_some())
        + u32::from(opts.has_enhanced_tree())
        + u32::from(opts.el_torito().is_some())
}

fn bridge<C: Clock + Clone>(opts: &CdOptions<C>) -> UdfOptions<C> {
    opts.udf()
        .clone()
        .with_bridge(Bridge::new(iso_descriptors(opts.iso())))
        .with_min_blocks(0)
}

fn never<E>(err: Error<Infallible>) -> Error<E> {
    err.map_device(|never| match never {})
}

/// The length of the image: the ISO 9660 image, the files and the UDF
/// structures before them, then the trailing anchor.
fn total(iso: &hadris_iso::Report, files_end: u64) -> u64 {
    (iso.total_blocks() + TAIL).max(files_end + SLACK + TAIL)
}

io_transform! {

/// The [`Report`] [`write()`] would return, without writing: the image
/// size, where each file goes, and the warnings of both writers.
///
/// Size an output device with [`Report::size_bytes`]. Host files are
/// opened and measured; nothing else is read.
pub async fn plan<C: Clock + Clone>(tree: &Tree, opts: &CdOptions<C>) -> Result<Report, Error<Infallible>> {
    let udf_opts = bridge(opts);
    let udf = super::udf::plan(tree, &udf_opts).await?;
    let iso_opts = opts.iso().clone().with_min_blocks(opts.iso().min_blocks().max(udf.allocated_end()));
    let iso = super::iso::plan(tree, &iso_opts).await?;
    let (stored, files_end) = crate::tree::stored::<Infallible>(tree, &iso)?;
    let files_end = files_end.max(udf.allocated_end());
    let udf = super::udf::plan(&stored, &udf_opts.with_min_blocks(total(&iso, files_end))).await?;
    Ok(Report::new(iso, udf))
}

/// Writes `tree` as a hybrid ISO 9660 and UDF image on `out`, as `opts`
/// says, and returns the [`Report`].
///
/// The ISO 9660 image is written first, from block 0 in ascending order,
/// with its directories after the UDF metadata; then the UDF structures,
/// pointing at the ISO 9660 file extents, and the last block of the image.
/// The device must hold [`Report::size_bytes`] (see [`plan`]); a device
/// that grows on write, such as a host file, needs no sizing. Errors of
/// either writer keep their kind and detail ([`Detail`](crate::Detail)).
pub async fn write<D: BlockDevice, C: Clock + Clone>(mut out: D, tree: &Tree, opts: &CdOptions<C>) -> Result<Report, Error<D::Error>> {
    let udf_opts = bridge(opts);
    let udf = super::udf::plan(tree, &udf_opts).await.map_err(|err| never(err.into()))?;
    let iso_opts = opts.iso().clone().with_min_blocks(opts.iso().min_blocks().max(udf.allocated_end()));
    let iso = super::iso::write(&mut out, tree, &iso_opts).await?;
    let (stored, files_end) = crate::tree::stored(tree, &iso)?;
    let files_end = files_end.max(udf.allocated_end());
    let udf = super::udf::write(&mut out, &stored, &udf_opts.with_min_blocks(total(&iso, files_end))).await?;
    Ok(Report::new(iso, udf))
}

}
