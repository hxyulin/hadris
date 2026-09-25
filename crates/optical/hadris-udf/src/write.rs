use alloc::vec;
use core::convert::Infallible;

use hadris_fs::{ErrorKind, PathError, Report, Tree};
use hadris_iso::IsoOptions;
use hadris_storage::BlockIndex;

use super::fs::ContentReader;
use super::storage::BlockDevice;
use crate::bridge;
use crate::error::{Detail, Error};
use crate::options::UdfOptions;
use crate::plan::{self, Plan, Region, SECTOR};

/// Bytes read from a content per request.
const CHUNK: usize = 64 * 1024;

fn check_block_size<D: BlockDevice>(out: &D) -> Result<(), PathError> {
    let block = out.block_size().get() as usize;
    if block > SECTOR || SECTOR % block != 0 {
        return Err(Detail::OutputBlockSize
            .error::<Infallible>(ErrorKind::Unsupported)
            .into());
    }
    Ok(())
}

/// Checks that `out` can hold `blocks` 2048-byte blocks.
fn check_output<D: BlockDevice>(out: &D, blocks: u64) -> Result<(), PathError> {
    let per_sector = (SECTOR / out.block_size().get() as usize) as u64;
    if blocks.saturating_mul(per_sector) > out.max_block_count() {
        return Err(PathError::new(
            ErrorKind::NoSpace,
            "the output device is smaller than the volume",
        ));
    }
    Ok(())
}

/// Checks that this mode can read every file the plan writes.
fn check_contents(tree: &Tree, plan: &Plan) -> Result<(), PathError> {
    for region in &plan.regions {
        if let Region::File { path, .. } = region
            && let Some(content) = tree.get(path).and_then(|node| node.content())
        {
            ContentReader::check(content).map_err(|err| err.with_path(path))?;
        }
    }
    Ok(())
}

io_transform! {

/// Writes `tree` as a UDF volume on `out`, as `opts` says, from block 0,
/// and returns the report [`plan`](crate::plan) returns.
///
/// Every block is written once, in ascending order, gaps as zeros; then the
/// device is flushed. Its block size must divide 2048
/// ([`Detail::OutputBlockSize`]).
///
/// Fails before writing anything as [`plan`](crate::plan) does, with
/// [`ErrorKind::NoSpace`] when the device cannot hold [`Report::size`] (a
/// device that grows on write, such as a host file, needs no sizing), and
/// with [`ErrorKind::Unsupported`] naming the file whose content this mode
/// cannot read. An error from a file's content carries the file's path
/// ([`PathError::path`]).
pub async fn write<D: BlockDevice>(mut out: D, tree: &Tree, opts: &UdfOptions) -> Result<Report, PathError> {
    check_block_size(&out)?;
    let contents = plan::measure(tree, false, true)?;
    let plan = plan::lay_out(tree, opts, &contents, true, None)?;
    check_output(&out, plan.total_blocks)?;
    check_contents(tree, &plan)?;
    emit(&mut out, tree, &plan).await?;
    Ok(plan.report)
}

/// Writes `tree` as an ISO 9660 and UDF bridge image on `out` and returns
/// the report [`plan_bridge`](crate::plan_bridge) returns.
///
/// The ISO 9660 image is written first, from block 0 in ascending order,
/// with its directories after the UDF metadata; then the UDF structures,
/// pointing at the ISO 9660 file extents, and the last block of the image.
/// Last, the volume space size of each ISO 9660 volume descriptor is set to
/// the whole image, UDF structures included (ECMA-119 8.4.8). Nothing is
/// read back from the device except those descriptors.
///
/// Fails before writing anything as [`plan_bridge`](crate::plan_bridge)
/// does, and like [`write()`] for the device. Errors of either writer keep
/// their kind, detail code and path.
pub async fn write_bridge<D: BlockDevice>(mut out: D, tree: &Tree, iso: &IsoOptions, udf: &UdfOptions) -> Result<Report, PathError> {
    check_block_size(&out)?;
    let plan = bridge::plan_both(tree, iso, udf)?;
    check_output(&out, plan.udf.total_blocks)?;
    super::iso::write(&mut out, tree, &plan.iso).await?;
    emit(&mut out, &plan.stored, &plan.udf).await?;
    cover(&mut out, plan.descriptors, plan.udf.total_blocks).await?;
    Ok(plan.report())
}

/// Sets the volume space size of the first `descriptors` ISO 9660 volume
/// descriptors to `blocks`, the whole image, so the ISO 9660 volume also
/// covers the UDF structures after its own end.
async fn cover<D: BlockDevice>(out: &mut D, descriptors: u32, blocks: u64) -> Result<(), Error<D::Error>> {
    let blocks = u32::try_from(blocks).unwrap_or(u32::MAX);
    let per_sector = SECTOR as u64 / u64::from(out.block_size().get());
    let mut sector = [0u8; SECTOR];
    for index in 16..16 + u64::from(descriptors) {
        let first = BlockIndex::new(index * per_sector);
        out.read_blocks(first, &mut sector).await?;
        if !matches!(sector[0], 1 | 2) || &sector[1..6] != b"CD001" {
            continue;
        }
        sector[80..84].copy_from_slice(&blocks.to_le_bytes());
        sector[84..88].copy_from_slice(&blocks.to_be_bytes());
        out.write_blocks(first, &sector).await?;
    }
    out.flush().await
}

async fn write_sectors<D: BlockDevice>(out: &mut D, sector: u64, data: &[u8]) -> Result<(), Error<D::Error>> {
    let per_sector = (SECTOR / out.block_size().get() as usize) as u64;
    out.write_blocks(BlockIndex::new(sector * per_sector), data).await?;
    Ok(())
}

async fn zero<D: BlockDevice>(out: &mut D, from: u64, to: u64, buf: &mut [u8]) -> Result<(), Error<D::Error>> {
    buf.fill(0);
    let mut sector = from;
    while sector < to {
        let count = (to - sector).min((buf.len() / SECTOR) as u64);
        write_sectors(out, sector, &buf[..count as usize * SECTOR]).await?;
        sector += count;
    }
    Ok(())
}

/// Writes the regions of `plan` in order, zero-filling the gaps when the
/// plan asks for it, and flushes.
async fn emit<D: BlockDevice>(out: &mut D, tree: &Tree, plan: &Plan) -> Result<(), PathError> {
    let mut next = 0;
    let mut buf = vec![0u8; CHUNK];
    for region in &plan.regions {
        if plan.fill_gaps {
            zero(out, next, region.block(), &mut buf).await?;
        }
        match region {
            Region::Bytes { block, data } => {
                if data.len() % SECTOR == 0 {
                    write_sectors(out, *block, data).await?;
                } else {
                    let mut padded = data.clone();
                    padded.resize(data.len().div_ceil(SECTOR) * SECTOR, 0);
                    write_sectors(out, *block, &padded).await?;
                }
            }
            Region::File { block, path, len } => {
                file(out, tree, *block, path, *len, &mut buf).await?;
            }
        }
        next = region.block() + region.blocks();
    }
    if plan.fill_gaps {
        zero(out, next, plan.total_blocks, &mut buf).await?;
    }
    out.flush().await?;
    Ok(())
}

async fn file<D: BlockDevice>(
    out: &mut D,
    tree: &Tree,
    block: u64,
    path: &str,
    len: u64,
    buf: &mut [u8],
) -> Result<(), PathError> {
    let changed = || PathError::from(Detail::Content.corrupt::<Infallible>()).with_path(path);
    let Some(content) = tree.get(path).and_then(|node| node.content()) else {
        return Err(changed());
    };
    let mut reader = ContentReader::open(content).await.map_err(|err| err.with_path(path))?;
    if reader.len() != len {
        return Err(changed());
    }
    let mut offset = 0u64;
    let mut sector = block;
    while offset < len {
        let take = (len - offset).min(buf.len() as u64) as usize;
        reader.read_exact_at(offset, &mut buf[..take]).await.map_err(|err| err.with_path(path))?;
        let padded = take.div_ceil(SECTOR) * SECTOR;
        buf[take..padded].fill(0);
        write_sectors(out, sector, &buf[..padded]).await?;
        sector += (padded / SECTOR) as u64;
        offset += take as u64;
    }
    Ok(())
}

}
