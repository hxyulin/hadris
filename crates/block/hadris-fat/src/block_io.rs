//! One device block of buffering and byte-granular reads and writes over a
//! block device, shared by every driver of the mode.

use hadris_fs::{ErrorKind, FsResult};
use hadris_storage::BlockIndex;

use super::rawio;
use super::storage::BlockDevice;

#[cfg(feature = "alloc")]
pub(crate) use rawio::read_bytes;

/// The largest device block a driver can buffer.
pub(crate) const MAX_BLOCK_SIZE: usize = 4096;

/// Zeros that a fill writes from, many blocks per device call, so the fill
/// needs no buffer of its own.
static ZEROS: [u8; 64 * 1024] = [0; 64 * 1024];

/// One device block, the driver's only buffer.
pub(crate) type BlockBuf = hadris_fat_raw::io::BlockBuf<[u8; MAX_BLOCK_SIZE]>;

/// A buffer for device blocks of `size` bytes; [`ErrorKind::Unsupported`]
/// when they are larger than [`MAX_BLOCK_SIZE`].
pub(crate) fn new_block(size: usize) -> Result<BlockBuf, ErrorKind> {
    BlockBuf::new(size).ok_or(ErrorKind::Unsupported)
}

io_transform! {

/// Writes `len` bytes at byte `offset`: from `data`, or zeros when `data` is
/// `None`. Whole blocks of zeros go to the device up to 64 KiB per call.
pub(crate) async fn write_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut hadris_fat_raw::io::BlockBuf,
    offset: u64,
    data: Option<&[u8]>,
    len: usize,
) -> FsResult<(), D::Error> {
    match data {
        Some(data) => rawio::write_bytes(dev, block, offset, &data[..len]).await,
        None => write_zeros(dev, block, offset, len).await,
    }
}

async fn write_zeros<D: BlockDevice>(
    dev: &mut D,
    block: &mut hadris_fat_raw::io::BlockBuf,
    offset: u64,
    len: usize,
) -> FsResult<(), D::Error> {
    let size = block.block_size();
    if size > ZEROS.len() {
        return rawio::write_zeros(dev, block, offset, len).await;
    }
    let head = ((size - (offset % size as u64) as usize) % size).min(len);
    rawio::write_zeros(dev, block, offset, head).await?;
    let first = (offset + head as u64) / size as u64;
    let blocks = ((len - head) / size) as u64;
    if block.cached().is_some_and(|cached| cached.wrapping_sub(first) < blocks) {
        block.invalidate();
    }
    let per_call = (ZEROS.len() / size) as u64;
    let mut done = 0;
    while done < blocks {
        let n = per_call.min(blocks - done);
        dev.write_blocks(BlockIndex::new(first + done), &ZEROS[..n as usize * size])
            .await?;
        done += n;
    }
    let tail = (first + blocks) * size as u64;
    rawio::write_zeros(dev, block, tail, len - head - blocks as usize * size).await
}

}
