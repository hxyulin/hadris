#![cfg(all(feature = "std", feature = "async"))]

use core::future::Future;
use core::task::{Context, Poll, Waker};

use hadris_io::ErrorKind;
use hadris_storage::BlockIndex;
use hadris_storage::async_::BlockDevice;
use hadris_storage::host::FileDevice;

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

#[test]
fn file_device_implements_the_async_contract() {
    let path =
        std::env::temp_dir().join(format!("hadris-storage-host-async-{}", std::process::id()));
    std::fs::write(&path, [0u8; 1024]).unwrap();
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut dev = FileDevice::new(file).unwrap();
    block_on(async {
        assert_eq!(BlockDevice::block_count(&dev), 2);
        dev.write_blocks(BlockIndex::new(2), &[7; 512])
            .await
            .unwrap();
        dev.flush().await.unwrap();
        assert_eq!(BlockDevice::block_count(&dev), 3);
        let mut buf = [0; 512];
        dev.read_blocks(BlockIndex::new(2), &mut buf).await.unwrap();
        assert_eq!(buf, [7; 512]);
    });
    let mut read_only = FileDevice::open(&path).unwrap();
    block_on(async {
        assert!(!BlockDevice::writable(&read_only));
        let err = read_only
            .write_blocks(BlockIndex::new(0), &[1; 512])
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ReadOnly);
    });
    std::fs::remove_file(&path).unwrap();
}
