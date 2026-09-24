//! One device block of buffering and byte-granular reads and writes over a
//! block device, shared by every driver of the mode.

use hadris_fs::{Error, FsResult};
use hadris_storage::BlockIndex;

use super::storage::BlockDevice;

/// The largest device block a driver can buffer.
pub(crate) const MAX_BLOCK_SIZE: usize = 4096;

/// One device block, the driver's only buffer.
pub(crate) struct BlockBuf {
    pub(crate) data: [u8; MAX_BLOCK_SIZE],
    pub(crate) size: usize,
    pub(crate) cached: Option<u64>,
}

impl BlockBuf {
    pub(crate) fn new(size: usize) -> Self {
        Self {
            data: [0; MAX_BLOCK_SIZE],
            size,
            cached: None,
        }
    }
}

/// Clusters whose entries share a device block, as bits from `base`.
pub(crate) struct ClusterGroup {
    base: u32,
    bits: [u64; ClusterGroup::SPAN as usize / 64],
    pub(crate) count: u32,
}

impl ClusterGroup {
    /// Most clusters a group holds: the FAT16 entries of a 4096-byte block.
    pub(crate) const SPAN: u32 = 2048;

    pub(crate) fn new(base: u32) -> Self {
        Self {
            base,
            bits: [0; Self::SPAN as usize / 64],
            count: 0,
        }
    }

    /// Whether `cluster` is in the group's span.
    pub(crate) fn spans(&self, cluster: u32) -> bool {
        cluster.wrapping_sub(self.base) < Self::SPAN
    }

    pub(crate) fn has(&self, cluster: u32) -> bool {
        let bit = cluster.wrapping_sub(self.base);
        bit < Self::SPAN && self.bits[bit as usize / 64] & (1 << (bit % 64)) != 0
    }

    pub(crate) fn add(&mut self, cluster: u32) {
        let bit = cluster.wrapping_sub(self.base);
        if bit < Self::SPAN && !self.has(cluster) {
            self.bits[bit as usize / 64] |= 1 << (bit % 64);
            self.count += 1;
        }
    }

    pub(crate) fn descending(&self) -> impl Iterator<Item = u32> + '_ {
        (0..Self::SPAN)
            .rev()
            .filter(|&bit| self.bits[bit as usize / 64] & (1 << (bit % 64)) != 0)
            .map(|bit| self.base + bit)
    }

    pub(crate) fn lowest(&self) -> u32 {
        self.descending().last().unwrap_or(self.base)
    }
}

io_transform! {

pub(crate) async fn load<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, index: u64) -> FsResult<(), D::Error> {
    if block.cached == Some(index) {
        return Ok(());
    }
    block.cached = None;
    dev.read_blocks(BlockIndex::new(index), &mut block.data[..block.size])
        .await
        .map_err(Error::from_device)?;
    block.cached = Some(index);
    Ok(())
}

/// Reads `out.len()` bytes at byte `offset`. Whole blocks go straight into
/// `out`; partial blocks go through `block`.
pub(crate) async fn read_bytes<D: BlockDevice>(
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
                .await
                .map_err(Error::from_device)?;
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

/// Writes `len` bytes at byte `offset`: from `data`, or zeros when `data` is
/// `None`. Whole blocks of `data` go straight to the device, whole blocks of
/// zeros go from `block` as many at once as it holds, and partial blocks
/// are read, patched and written through `block`, which is left holding the
/// device's copy or nothing.
pub(crate) async fn write_bytes<D: BlockDevice>(
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
            let chunk = whole.min(MAX_BLOCK_SIZE / size * size);
            block.cached = None;
            block.data[..chunk].fill(0);
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
