use alloc::vec;
use core::convert::Infallible;

use hadris_fs::{Content, ErrorKind, PathError, Report, Tree};
use hadris_storage::BlockIndex;

use super::fs::ContentReader;
use super::storage::BlockDevice;
use crate::error::{Detail, Error};
use crate::options::{BootInfo, IsoOptions};
use crate::plan::{self, Base, InfoTable, Plan, Region};
use crate::raw::{self, SECTOR_SIZE};

/// Bytes read from a content per request.
const CHUNK: usize = 64 * 1024;

io_transform! {

/// Writes `tree` as an ISO 9660 image on `out`, as `opts` says, from block
/// 0, and returns the report [`plan`](crate::plan) returns.
///
/// Blocks are written once each, in ascending order, then the device is
/// flushed. Its block size must divide 2048 ([`Detail::OutputBlockSize`]).
///
/// Fails before writing anything as [`plan`](crate::plan) does, with
/// [`ErrorKind::NoSpace`] when the device cannot hold [`Report::size`]
/// (a device that grows on write, such as a host file, needs no sizing),
/// and with [`ErrorKind::Unsupported`] naming the file whose content this
/// mode cannot read. An error from a file's content carries the file's path
/// ([`PathError::path`]).
pub async fn write<D: BlockDevice>(mut out: D, tree: &Tree, opts: &IsoOptions) -> Result<Report, PathError> {
    check_block_size(&out)?;
    let contents = plan::measure(tree, false)?;
    let plan = plan::lay_out(tree, opts, &contents, Base::image())?;
    check_output(&out, &plan)?;
    check_contents(tree, &plan)?;
    emit(&mut out, tree, &plan).await?;
    Ok(plan.report)
}

/// Checks that `out` can hold the planned image.
pub(crate) fn check_output<D: BlockDevice>(out: &D, plan: &Plan) -> Result<(), PathError> {
    let per_sector = (SECTOR_SIZE / out.block_size().get() as usize) as u64;
    if plan.total_blocks.saturating_mul(per_sector) > out.max_block_count() {
        return Err(PathError::new(ErrorKind::NoSpace, "the output device is smaller than the image"));
    }
    Ok(())
}

/// Checks that this mode can read every file the plan writes.
pub(crate) fn check_contents(tree: &Tree, plan: &Plan) -> Result<(), PathError> {
    for region in &plan.regions {
        match region {
            Region::File { path, .. } => {
                if let Some(content) = tree.get(path).and_then(|node| node.content()) {
                    ContentReader::check(content).map_err(|err| err.with_path(path))?;
                }
            }
            Region::Content { content, .. } => ContentReader::check(content)?,
            Region::Bytes { .. } => {}
        }
    }
    Ok(())
}

pub(crate) fn check_block_size<D: BlockDevice>(out: &D) -> Result<(), Error<D::Error>> {
    let block = out.block_size().get() as usize;
    if block > SECTOR_SIZE || SECTOR_SIZE % block != 0 {
        return Err(Detail::OutputBlockSize.error(ErrorKind::Unsupported));
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
pub(crate) async fn emit<D: BlockDevice>(out: &mut D, tree: &Tree, plan: &Plan) -> Result<(), PathError> {
    emit_from(out, tree, plan, None::<&mut D>).await
}

pub(crate) async fn emit_from<D: BlockDevice, S: BlockDevice>(
    out: &mut D,
    tree: &Tree,
    plan: &Plan,
    mut source: Option<&mut S>,
) -> Result<(), PathError> {
    let mut next = plan.regions.first().map_or(0, Region::block);
    if plan.fill_gaps {
        next = 0;
    }
    let mut buf = vec![0u8; CHUNK];
    for region in &plan.regions {
        if region.block() < next {
            return Err(Detail::Session.corrupt::<Infallible>().into());
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
                let changed = || PathError::from(Detail::Content.corrupt::<Infallible>()).with_path(path);
                let content = tree.get(path).and_then(|node| node.content()).ok_or_else(changed)?;
                if content.len() != *len {
                    return Err(changed());
                }
                file(out, content, *block, *info, &mut buf, source.as_deref_mut()).await.map_err(|err| err.with_path(path))?;
            }
            Region::Content { block, content, info } => {
                file(out, content, *block, *info, &mut buf, source.as_deref_mut()).await?;
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

enum SourceReader<'a, S> {
    Content(ContentReader<'a>),
    Stored { source: &'a mut S, content: &'a Content },
}

impl<'a, S: BlockDevice> SourceReader<'a, S> {
    async fn open(content: &'a Content, source: Option<&'a mut S>) -> Result<Self, PathError> {
        if content.stored_extents().is_some() && let Some(source) = source {
            return Ok(Self::Stored { source, content });
        }
        Ok(Self::Content(ContentReader::open(content).await?))
    }

    fn len(&self) -> u64 {
        match self {
            Self::Content(reader) => reader.len(),
            Self::Stored { content, .. } => content.len(),
        }
    }

    async fn read_exact_at(&mut self, mut offset: u64, mut buf: &mut [u8]) -> Result<(), PathError> {
        match self {
            Self::Content(reader) => reader.read_exact_at(offset, buf).await,
            Self::Stored { source, content } => {
                let len = source.block_count().saturating_mul(u64::from(source.block_size().get()));
                for extent in content.stored_extents().unwrap_or(&[]) {
                    if offset >= extent.len() {
                        offset -= extent.len();
                        continue;
                    }
                    let take = (extent.len() - offset).min(buf.len() as u64) as usize;
                    let start = extent.offset().checked_add(offset)
                        .ok_or_else(|| PathError::from(Detail::OutsideImage.corrupt::<Infallible>()))?;
                    super::image::read_bytes(*source, len, start, &mut buf[..take]).await?;
                    buf = &mut buf[take..];
                    if buf.is_empty() {
                        return Ok(());
                    }
                    offset = 0;
                }
                if buf.is_empty() {
                    Ok(())
                } else {
                    Err(Detail::Content.corrupt::<Infallible>().into())
                }
            }
        }
    }
}

/// The sum of the image's 32-bit words from byte 64, as a boot information
/// table records it.
async fn checksum<S: BlockDevice>(reader: &mut SourceReader<'_, S>, len: u64, buf: &mut [u8]) -> Result<u32, PathError> {
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

async fn file<D: BlockDevice, S: BlockDevice>(
    out: &mut D,
    content: &Content,
    block: u64,
    info: Option<InfoTable>,
    buf: &mut [u8],
    source: Option<&mut S>,
) -> Result<(), PathError> {
    let mut reader = SourceReader::open(content, source).await?;
    let len = reader.len();
    if len != content.len() {
        return Err(Detail::Content.corrupt::<Infallible>().into());
    }
    let table = match info {
        Some(info) => {
            let sum = checksum(&mut reader, len, buf).await?;
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
        reader.read_exact_at(offset, &mut buf[..take]).await?;
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
