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
    let size = block.size;
    let mut done = 0;
    while done < out.len() {
        let pos = offset + done as u64;
        let index = pos / size as u64;
        let at = (pos % size as u64) as usize;
        let whole = (out.len() - done) / size * size;
        if at == 0 && whole > 0 {
            dev.read_blocks(BlockIndex::new(index), &mut out[done..done + whole])
                .await?;
            done += whole;
            continue;
        }
        load(dev, block, index).await?;
        let n = (size - at).min(out.len() - done);
        out[done..done + n].copy_from_slice(&block.data[at..at + n]);
        done += n;
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
    let size = block.size;
    let mut done = 0;
    while done < len {
        let pos = offset + done as u64;
        let index = pos / size as u64;
        let at = (pos % size as u64) as usize;
        let whole = (len - done) / size * size;
        if data.is_none() && at == 0 && whole > 0 {
            let chunk = whole.min(block.capacity() / size * size);
            block.scratch(chunk).fill(0);
            dev.write_blocks(BlockIndex::new(index), &block.data[..chunk]).await?;
            done += chunk;
            continue;
        }
        if let Some(data) = data
            && at == 0
            && whole > 0
        {
            let blocks = index..index + (whole / size) as u64;
            if block.cached.is_some_and(|cached| blocks.contains(&cached)) {
                block.cached = None;
            }
            dev.write_blocks(BlockIndex::new(index), &data[done..done + whole]).await?;
            done += whole;
            continue;
        }
        let n = (size - at).min(len - done);
        if n < size {
            load(dev, block, index).await?;
        }
        block.cached = None;
        match data {
            Some(data) => block.data[at..at + n].copy_from_slice(&data[done..done + n]),
            None => block.data[at..at + n].fill(0),
        }
        dev.write_blocks(BlockIndex::new(index), &block.data[..size]).await?;
        block.cached = Some(index);
        done += n;
    }
    Ok(())
}

}
