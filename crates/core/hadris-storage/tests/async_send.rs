#![cfg(all(feature = "async-send", feature = "alloc"))]

use hadris_io::Error;
use hadris_storage::async_send::{BlockDevice, ByteView, Cache};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

fn assert_send<T: Send>(value: T) -> T {
    value
}

fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

async fn copy_block<S: BlockDevice, T: BlockDevice>(
    src: &mut S,
    dst: &mut T,
) -> Result<(), Error<T::Error>> {
    let mut buf = [0u8; 512];
    src.read_blocks(BlockIndex::new(1), &mut buf).await.unwrap();
    dst.write_blocks(BlockIndex::new(0), &buf).await
}

#[test]
fn generic_device_futures_are_send() {
    let size = BlockSize::new(512).unwrap();
    let image: Vec<u8> = (0..2048u32).map(|i| (i / 512) as u8).collect();
    let mut src = Partition::new(MemDevice::new(&image[..], size), 512, 1024);
    let mut dst = Cache::new(Vec::new(), 4);
    block_on(assert_send(copy_block(&mut src, &mut dst))).unwrap();
    let dst = block_on(dst.finish()).unwrap();
    assert_eq!(dst[..512], [2u8; 512]);

    let mut view = ByteView::new(dst);
    let mut byte = [0u8];
    block_on(assert_send(view.read_at(3, &mut byte))).unwrap();
    assert_eq!(byte, [2]);
}
