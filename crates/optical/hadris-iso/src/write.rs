use alloc::collections::BTreeMap;
use alloc::vec;
use core::convert::Infallible;

use hadris_fs::tree::{NodeKind, Tree};
use hadris_fs::{Clock, ErrorKind};
use hadris_storage::BlockIndex;

use super::fs::ContentReader;
use super::storage::BlockDevice;
use crate::error::{Detail, Error};
use crate::options::{BootInfo, IsoOptions};
use crate::plan::{self, Base, ContentInfo, InfoTable, Plan, Region};
use crate::raw::{self, SECTOR_SIZE};
use crate::report::Report;

/// Bytes read from a content per request.
const CHUNK: usize = 64 * 1024;

fn never<E>(err: Error<Infallible>) -> Error<E> {
    err.map_device(|never| match never {})
}

io_transform! {

/// Measures every file of `tree`. Stored content is accepted only when
/// `stored` is set.
pub(crate) async fn measure(tree: &Tree, stored: bool) -> Result<BTreeMap<usize, ContentInfo>, Error<Infallible>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![tree.root()];
    while let Some(dir) = pending.pop() {
        for (_, child) in dir.children() {
            match child.kind() {
                NodeKind::Dir => pending.push(child),
                NodeKind::File(content) if !out.contains_key(&child.id()) => {
                    let info = match content.stored_extents() {
                        Some(extents) if stored => ContentInfo {
                            len: extents.iter().map(|extent| extent.len()).sum(),
                            stored: Some(extents.to_vec()),
                        },
                        Some(_) => return Err(Error::new(ErrorKind::Unsupported, Detail::StoredContent)),
                        None => {
                            let reader = ContentReader::open(content).await.map_err(Error::content)?;
                            ContentInfo { len: reader.len(), stored: None }
                        }
                    };
                    out.insert(child.id(), info);
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The [`Report`] [`write()`] would return, without writing: the image size,
/// where each file goes, and the warnings.
///
/// Size an output device with [`Report::size_bytes`]. Host files are
/// opened and measured; nothing else is read.
pub async fn plan<C: Clock>(tree: &Tree, opts: &IsoOptions<C>) -> Result<Report, Error<Infallible>> {
    let contents = measure(tree, false).await?;
    Ok(plan::plan(tree, opts, &contents, Base::image())?.report)
}

/// Writes `tree` as an ISO 9660 image on `out`, as `opts` says, from block
/// 0, and returns the [`Report`].
///
/// Blocks are written once each, in ascending order, then the device is
/// flushed. The device must hold [`Report::size_bytes`] (see [`plan`]); a
/// device that grows on write, such as a host file, needs no sizing. Its
/// block size must divide 2048 ([`Detail::OutputBlockSize`]).
///
/// Fails before writing anything when the options do not fit the tree:
/// [`ErrorKind::InvalidInput`] for a missing boot image, a diskette image
/// of the wrong size, a load size of zero, an identifier that does not
/// fit, a relocation clash or a tree too deep without Rock Ridge;
/// [`ErrorKind::LimitExceeded`] for MBR boot code over 446 bytes;
/// [`ErrorKind::FileTooLarge`] for a file of 4 GiB or more below Level 3.
/// A content that cannot be read fails with [`Detail::Content`].
pub async fn write<D: BlockDevice, C: Clock>(mut out: D, tree: &Tree, opts: &IsoOptions<C>) -> Result<Report, Error<D::Error>> {
    check_block_size(&out)?;
    let contents = measure(tree, false).await.map_err(never)?;
    let plan = plan::plan(tree, opts, &contents, Base::image()).map_err(never)?;
    emit(&mut out, tree, &plan).await?;
    Ok(plan.report)
}

pub(crate) fn check_block_size<D: BlockDevice>(out: &D) -> Result<(), Error<D::Error>> {
    let block = out.block_size().get() as usize;
    if block > SECTOR_SIZE || SECTOR_SIZE % block != 0 {
        return Err(Error::new(ErrorKind::Unsupported, Detail::OutputBlockSize));
    }
    Ok(())
}

async fn write_sectors<D: BlockDevice>(out: &mut D, sector: u64, data: &[u8]) -> Result<(), Error<D::Error>> {
    let per_sector = (SECTOR_SIZE / out.block_size().get() as usize) as u64;
    out.write_blocks(BlockIndex::new(sector * per_sector), data).await?;
    Ok(())
}

/// Writes the regions of `plan` in order, zero-filling the gaps when the
/// plan asks for it, and flushes.
pub(crate) async fn emit<D: BlockDevice>(out: &mut D, tree: &Tree, plan: &Plan) -> Result<(), Error<D::Error>> {
    let mut next = plan.regions.first().map_or(0, Region::block);
    if plan.fill_gaps {
        next = 0;
    }
    let mut buf = vec![0u8; CHUNK];
    for region in &plan.regions {
        if region.block() < next {
            return Err(Error::corrupt(Detail::Session));
        }
        if plan.fill_gaps {
            zero(out, next, region.block(), &mut buf).await?;
        }
        match region {
            Region::Bytes { block, data } => {
                let mut padded;
                let data = if data.len() % SECTOR_SIZE == 0 {
                    &data[..]
                } else {
                    padded = data.clone();
                    padded.resize(data.len().div_ceil(SECTOR_SIZE) * SECTOR_SIZE, 0);
                    &padded[..]
                };
                write_sectors(out, *block, data).await?;
            }
            Region::File { block, path, len, info } => {
                file(out, tree, *block, path, *len, *info, &mut buf).await?;
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

async fn zero<D: BlockDevice>(out: &mut D, from: u64, to: u64, buf: &mut [u8]) -> Result<(), Error<D::Error>> {
    buf.fill(0);
    let mut sector = from;
    while sector < to {
        let count = (to - sector).min((buf.len() / SECTOR_SIZE) as u64);
        write_sectors(out, sector, &buf[..count as usize * SECTOR_SIZE]).await?;
        sector += count;
    }
    Ok(())
}

/// The sum of the image's 32-bit words from byte 64, as a boot information
/// table records it.
async fn checksum(reader: &mut ContentReader<'_>, len: u64, buf: &mut [u8]) -> Result<u32, hadris_fs::PathError> {
    let words_end = 64 + (len - 64) / 4 * 4;
    let mut sum = 0u32;
    let mut offset = 64;
    while offset < words_end {
        let take = (words_end - offset).min(buf.len() as u64) as usize;
        reader.read_exact_at(offset, &mut buf[..take]).await?;
        for word in buf[..take].chunks_exact(4) {
            sum = sum.wrapping_add(u32::from_le_bytes([word[0], word[1], word[2], word[3]]));
        }
        offset += take as u64;
    }
    Ok(sum)
}

async fn file<D: BlockDevice>(
    out: &mut D,
    tree: &Tree,
    block: u64,
    path: &str,
    len: u64,
    info: Option<InfoTable>,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    let Some(NodeKind::File(content)) = tree.get(path).map(|node| node.kind()) else {
        return Err(Error::corrupt(Detail::Content));
    };
    let mut reader = ContentReader::open(content).await.map_err(Error::content)?;
    if reader.len() != len {
        return Err(Error::content(ErrorKind::Corrupt.into()));
    }
    let table = match info {
        Some(info) => {
            let sum = checksum(&mut reader, len, buf).await.map_err(Error::content)?;
            let table = raw::Grub2BootInfoTable {
                pvd_lba: raw::U32Le::new(raw::DESCRIPTOR_START),
                file_lba: raw::U32Le::new(info.block),
                file_len: raw::U32Le::new(info.len),
                checksum: raw::U32Le::new(sum),
                reserved: [0; 40],
            };
            let size = match info.kind {
                BootInfo::Grub2 => 56,
                _ => 16,
            };
            let mut bytes = [0u8; 56];
            bytes.copy_from_slice(bytemuck::bytes_of(&table));
            Some((bytes, size))
        }
        None => None,
    };
    let mut offset = 0u64;
    let mut sector = block;
    while offset < len {
        let take = (len - offset).min(buf.len() as u64) as usize;
        reader.read_exact_at(offset, &mut buf[..take]).await.map_err(Error::content)?;
        if let Some((bytes, size)) = &table
            && offset == 0
        {
            let end = (8 + size).min(take);
            buf[8..end].copy_from_slice(&bytes[..end - 8]);
        }
        let padded = take.div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
        buf[take..padded].fill(0);
        write_sectors(out, sector, &buf[..padded]).await?;
        sector += (padded / SECTOR_SIZE) as u64;
        offset += take as u64;
    }
    Ok(())
}

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_errors_convert_to_any_device() {
        let err: Error<u8> = never(Error::invalid(Detail::BootImage));
        assert_eq!(err.detail(), Some(Detail::BootImage));
    }
}
