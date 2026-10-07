use core::task::{Context, Poll};
use hadris_io::{Error, ErrorType};
use hadris_storage::{BlockIndex, BlockSize};

use crate::BlockDevice;

/// Prototype write-through cache for one 512-byte block.
/// Other block sizes and multi-block transfers pass through to the device.
pub struct Cache<D> {
    device: D,
    buffer: [u8; 512],
    valid: Option<BlockIndex>,
}

impl<D> Cache<D> {
    /// Creates an empty cache.
    pub fn new(device: D) -> Self {
        Self {
            device,
            buffer: [0; 512],
            valid: None,
        }
    }

    /// Returns the underlying device, discarding cached data.
    pub fn into_inner(self) -> D {
        self.device
    }
}

impl<D: ErrorType> ErrorType for Cache<D> {
    type Error = D::Error;
}

impl<D: BlockDevice> BlockDevice for Cache<D> {
    type State = D::State;

    fn block_size(&self) -> BlockSize {
        self.device.block_size()
    }
    fn block_count(&self) -> u64 {
        self.device.block_count()
    }
    fn max_block_count(&self) -> u64 {
        self.device.max_block_count()
    }
    fn disk_offset(&self) -> u64 {
        self.device.disk_offset()
    }
    fn writable(&self) -> bool {
        self.device.writable()
    }

    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if self.block_size().get() != 512 || buf.len() != 512 {
            return self.device.poll_read_blocks(state, cx, first, buf);
        }
        if self.valid == Some(first) {
            buf.copy_from_slice(&self.buffer);
            return Poll::Ready(Ok(()));
        }
        self.valid = None;
        match self
            .device
            .poll_read_blocks(state, cx, first, &mut self.buffer)
        {
            Poll::Ready(Ok(())) => {
                self.valid = Some(first);
                buf.copy_from_slice(&self.buffer);
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }

    fn poll_write_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        self.valid = None;
        self.device.poll_write_blocks(state, cx, first, buf)
    }

    fn poll_flush(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        self.device.poll_flush(state, cx)
    }

    fn cancel(&mut self, state: &mut Self::State) {
        self.device.cancel(state);
        self.valid = None;
    }
}
