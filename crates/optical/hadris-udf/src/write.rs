use alloc::collections::BTreeMap;
use alloc::vec;
use core::convert::Infallible;

use hadris_fs::tree::{NodeKind, Tree};
use hadris_fs::{Clock, ErrorKind};
use hadris_storage::BlockIndex;

use super::fs::ContentReader;
use super::storage::BlockDevice;
use crate::error::{Detail, Error};
use crate::options::UdfOptions;
use crate::plan::{self, ContentInfo, Plan, Region, SECTOR};
use crate::report::Report;

/// Bytes read from a content per request.
const CHUNK: usize = 64 * 1024;

fn never<E>(err: Error<Infallible>) -> Error<E> {
    err.map_device(|never| match never {})
}

io_transform! {

/// Measures every file of `tree`: the length of content to write, or the
/// extents of stored content in a bridge volume. With `strict`, a bridge
/// file whose content is neither stored nor empty fails; without, it is
/// left out.
async fn measure(tree: &Tree, bridge: bool, strict: bool) -> Result<BTreeMap<usize, ContentInfo>, Error<Infallible>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![tree.root()];
    while let Some(dir) = pending.pop() {
        for (_, child) in dir.children() {
            match child.kind() {
                NodeKind::Dir => pending.push(child),
                NodeKind::File(content) if !out.contains_key(&child.id()) => {
                    let info = match content.stored_extents() {
                        Some(extents) if bridge => ContentInfo {
                            len: extents.iter().map(|extent| extent.len()).sum(),
                            stored: Some(extents.to_vec()),
                        },
                        None if bridge && content.len() == Some(0) => ContentInfo { len: 0, stored: None },
                        None if bridge && !strict => continue,
                        None if !bridge => {
                            let reader = ContentReader::open(content).await.map_err(Error::content)?;
                            ContentInfo { len: reader.len(), stored: None }
                        }
                        _ => return Err(Error::new(ErrorKind::Unsupported, Detail::StoredContent)),
                    };
                    out.insert(child.id(), info);
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The [`Report`] [`write()`] would return, without writing: the volume
/// size, where each file goes, and the warnings.
///
/// Size an output device with [`Report::size_bytes`]. Host files are
/// opened and measured; nothing else is read. For a bridge volume, files
/// whose content is not stored yet are planned without data, so the
/// ISO 9660 structures can be placed at [`Report::allocated_end`] before
/// the stored extents are known.
pub async fn plan<C: Clock>(tree: &Tree, opts: &UdfOptions<C>) -> Result<Report, Error<Infallible>> {
    let bridge = opts.bridge().is_some();
    let contents = measure(tree, bridge, false).await?;
    Ok(plan::plan(tree, opts, &contents, !bridge)?.report)
}

/// Writes `tree` as a UDF volume on `out`, as `opts` says, and returns the
/// [`Report`].
///
/// A standalone volume is written from block 0, every block once, in
/// ascending order, gaps as zeros; then the device is flushed. The device
/// must hold [`Report::size_bytes`] (see [`plan`]); a device that grows on
/// write, such as a host file, needs no sizing. Its block size must divide
/// 2048 ([`Detail::OutputBlockSize`]).
///
/// A bridge volume ([`UdfOptions::with_bridge`]) writes only its own
/// structures and points at the stored extents of each file, which must be
/// whole blocks after [`Report::allocated_end`]
/// ([`Detail::StoredContent`]).
///
/// Fails before writing anything with [`ErrorKind::InvalidInput`] for a
/// volume identifier over 126 bytes or a time outside the years 1 to 9999,
/// [`ErrorKind::NameTooLong`] for a name over 254 bytes of OSTA Compressed
/// Unicode, and [`ErrorKind::FileTooLarge`] for a file of more than 234
/// GiB. A content that cannot be read fails with [`Detail::Content`].
pub async fn write<D: BlockDevice, C: Clock>(mut out: D, tree: &Tree, opts: &UdfOptions<C>) -> Result<Report, Error<D::Error>> {
    let block = out.block_size().get() as usize;
    if block > SECTOR || SECTOR % block != 0 {
        return Err(Error::new(ErrorKind::Unsupported, Detail::OutputBlockSize));
    }
    let bridge = opts.bridge().is_some();
    let contents = measure(tree, bridge, true).await.map_err(never)?;
    let plan = plan::plan(tree, opts, &contents, true).map_err(never)?;
    emit(&mut out, tree, &plan).await?;
    Ok(plan.report)
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
async fn emit<D: BlockDevice>(out: &mut D, tree: &Tree, plan: &Plan) -> Result<(), Error<D::Error>> {
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
) -> Result<(), Error<D::Error>> {
    let Some(NodeKind::File(content)) = tree.get(path).map(|node| node.kind()) else {
        return Err(Error::corrupt(Detail::Content));
    };
    let mut reader = ContentReader::open(content).await.map_err(Error::content)?;
    if reader.len() != len {
        return Err(Error::content(ErrorKind::Corrupt.into()));
    }
    let mut offset = 0u64;
    let mut sector = block;
    while offset < len {
        let take = (len - offset).min(buf.len() as u64) as usize;
        reader.read_exact_at(offset, &mut buf[..take]).await.map_err(Error::content)?;
        let padded = take.div_ceil(SECTOR) * SECTOR;
        buf[take..padded].fill(0);
        write_sectors(out, sector, &buf[..padded]).await?;
        sector += (padded / SECTOR) as u64;
        offset += take as u64;
    }
    Ok(())
}

}
