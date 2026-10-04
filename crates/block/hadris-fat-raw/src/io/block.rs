use hadris_fs::FsResult;
use hadris_storage::BlockIndex;

use super::storage::BlockDevice;
use crate::io::BlockBuf;

io_transform! {

/// Reads device block `index` into `block`, unless it already holds it.
pub async fn load<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, index: u64) -> FsResult<(), D::Error> {
    if block.cached == Some(index) {
        return Ok(());
    }
    block.cached = None;
    let size = block.size;
    dev.read_blocks(BlockIndex::new(index), &mut block.data[..size])
        .await?;
    block.cached = Some(index);
    Ok(())
}

/// Writes the contents of `block` as device block `index`, after which the
/// buffer holds that block.
pub async fn store<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, index: u64) -> FsResult<(), D::Error> {
    block.cached = None;
    let size = block.size;
    dev.write_blocks(BlockIndex::new(index), &block.data[..size]).await?;
    block.cached = Some(index);
    Ok(())
}

/// Reads `out.len()` bytes at byte `offset`. Whole blocks go straight into
/// `out`; partial blocks go through `block`.
pub async fn read_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    offset: u64,
    out: &mut [u8],
) -> FsResult<(), D::Error> {
    if out.is_empty() {
        return Ok(());
    }
    let size = block.size;
    let mut index = offset / size as u64;
    let mut at = (offset % size as u64) as usize;
    let mut out = out;
    while !out.is_empty() {
        let blocks = if at == 0 { out.len() / size } else { 0 };
        if blocks > 0 {
            let first = BlockIndex::new(index);
            index = index.wrapping_add(blocks as u64);
            let (chunk, tail) = out.split_at_mut(blocks * size);
            out = tail;
            dev.read_blocks(first, chunk).await?;
            continue;
        }
        load(dev, block, index).await?;
        let n = (size - at).min(out.len());
        let (chunk, tail) = out.split_at_mut(n);
        chunk.copy_from_slice(&block.data[at..at + n]);
        out = tail;
        index += 1;
        at = 0;
    }
    Ok(())
}

/// Writes `data` at byte `offset`. Whole blocks go straight to the device;
/// partial blocks are read, patched and written through `block`, which is
/// left holding the device's copy or nothing.
pub async fn write_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    offset: u64,
    data: &[u8],
) -> FsResult<(), D::Error> {
    put(dev, block, offset, Some(data), data.len()).await
}

/// Writes `len` zero bytes at byte `offset`. Whole blocks go from `block`
/// as many at once as it holds.
pub async fn write_zeros<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    offset: u64,
    len: usize,
) -> FsResult<(), D::Error> {
    put(dev, block, offset, None, len).await
}

pub(super) async fn put<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    offset: u64,
    data: Option<&[u8]>,
    len: usize,
) -> FsResult<(), D::Error> {
    if len == 0 {
        return Ok(());
    }
    let size = block.size;
    let mut index = offset / size as u64;
    let mut at = (offset % size as u64) as usize;
    let mut data = data;
    let mut remaining = len;
    while remaining > 0 {
        let blocks = if at == 0 { remaining / size } else { 0 };
        if blocks > 0 {
            let blocks = if data.is_none() { blocks.min(block.capacity() / size) } else { blocks };
            let chunk = blocks * size;
            let bytes = match data {
                Some(bytes) => {
                    if block.cached.is_some_and(|cached| cached >= index && cached - index < blocks as u64) {
                        block.cached = None;
                    }
                    data = Some(&bytes[chunk..]);
                    &bytes[..chunk]
                }
                None => {
                    block.scratch(chunk).fill(0);
                    &block.data[..chunk]
                }
            };
            let first = BlockIndex::new(index);
            remaining -= chunk;
            index = index.wrapping_add(blocks as u64);
            dev.write_blocks(first, bytes).await?;
            continue;
        }
        let n = (size - at).min(remaining);
        load(dev, block, index).await?;
        block.cached = None;
        match data {
            Some(bytes) => {
                block.data[at..at + n].copy_from_slice(&bytes[..n]);
                data = Some(&bytes[n..]);
            }
            None => block.data[at..at + n].fill(0),
        }
        dev.write_blocks(BlockIndex::new(index), &block.data[..size]).await?;
        block.cached = Some(index);
        remaining -= n;
        index += 1;
        at = 0;
    }
    Ok(())
}

}
