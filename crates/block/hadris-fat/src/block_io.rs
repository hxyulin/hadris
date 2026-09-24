//! One device block of buffering and byte-granular reads and writes over a
//! block device, shared by every driver of the mode.

use hadris_fs::{ErrorKind, FsResult};

use super::rawio;
use super::storage::BlockDevice;

pub(crate) use rawio::read_bytes;

/// The largest device block a driver can buffer.
pub(crate) const MAX_BLOCK_SIZE: usize = 4096;

/// One device block, the driver's only buffer.
pub(crate) type BlockBuf = hadris_fat_raw::io::BlockBuf<[u8; MAX_BLOCK_SIZE]>;

/// A buffer for device blocks of `size` bytes; [`ErrorKind::Unsupported`]
/// when they are larger than [`MAX_BLOCK_SIZE`].
pub(crate) fn new_block(size: usize) -> Result<BlockBuf, ErrorKind> {
    BlockBuf::new(size).ok_or(ErrorKind::Unsupported)
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
