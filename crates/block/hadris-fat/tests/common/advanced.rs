use hadris_io::{Error, ErrorType};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

/// A device whose physical blocks are larger than its logical ones, as a
/// 512e disk's are.
pub struct Advanced {
    pub inner: MemDevice<Vec<u8>>,
    pub physical: BlockSize,
}

impl Advanced {
    pub fn new(bytes: usize, logical: u32, physical: u32) -> Self {
        Self {
            inner: MemDevice::new(vec![0; bytes], BlockSize::new(logical).unwrap()),
            physical: BlockSize::new(physical).unwrap(),
        }
    }
}

impl ErrorType for Advanced {
    type Error = core::convert::Infallible;
}

impl BlockDevice for Advanced {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn physical_block_size(&self) -> BlockSize {
        self.physical
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.inner.read_blocks(first, buf)
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        self.inner.write_blocks(first, buf)
    }
}
