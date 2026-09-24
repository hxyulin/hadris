//! One device block of buffering and byte-granular reads and writes over a
//! block device, shared by every driver of the mode.

use hadris_fs::{ErrorKind, FsResult};

use super::rawio;
use super::storage::BlockDevice;

pub(crate) use rawio::{load, read_bytes, store};

/// The largest device block a driver can buffer.
pub(crate) const MAX_BLOCK_SIZE: usize = 4096;

/// One device block, the driver's only buffer.
pub(crate) type BlockBuf = hadris_fat_raw::io::BlockBuf<[u8; MAX_BLOCK_SIZE]>;

/// A buffer for device blocks of `size` bytes; [`ErrorKind::Unsupported`]
/// when they are larger than [`MAX_BLOCK_SIZE`].
pub(crate) fn new_block(size: usize) -> Result<BlockBuf, ErrorKind> {
    BlockBuf::new(size).ok_or(ErrorKind::Unsupported)
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

/// Writes `len` bytes at byte `offset`: from `data`, or zeros when `data` is
/// `None`.
pub(crate) async fn write_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut hadris_fat_raw::io::BlockBuf,
    offset: u64,
    data: Option<&[u8]>,
    len: usize,
) -> FsResult<(), D::Error> {
    match data {
        Some(data) => rawio::write_bytes(dev, block, offset, &data[..len]).await,
        None => rawio::write_zeros(dev, block, offset, len).await,
    }
}

}
