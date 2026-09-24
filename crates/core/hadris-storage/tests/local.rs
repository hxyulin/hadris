#![cfg(feature = "async")]

use std::rc::Rc;

use hadris_io::{Error, ErrorType};
use hadris_storage::local::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, Partition};

fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

/// A device that is not `Send`, as one owned by a single-threaded
/// executor is.
struct Flash {
    blocks: Rc<[u8]>,
}

impl ErrorType for Flash {
    type Error = core::convert::Infallible;
}

impl BlockDevice for Flash {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.blocks.len() as u64 / 512
    }

    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>> {
        let at = first.get() as usize * 512;
        buf.copy_from_slice(&self.blocks[at..at + buf.len()]);
        Ok(())
    }
}

#[test]
fn partitions_of_local_devices() {
    let blocks: Rc<[u8]> = (0..2048u32).map(|i| (i / 512) as u8).collect();
    let mut partition = Partition::new(Flash { blocks }, 1024, 1024);
    assert_eq!(partition.block_count(), 2);
    assert_eq!(partition.disk_offset(), 1024);
    assert!(!partition.writable());
    let mut buf = [0u8; 512];
    block_on(partition.read_blocks(BlockIndex::new(1), &mut buf)).unwrap();
    assert_eq!(buf, [3; 512]);
    let err = block_on(partition.write_blocks(BlockIndex::new(0), &buf)).unwrap_err();
    assert_eq!(err.kind(), hadris_io::ErrorKind::ReadOnly);
}
