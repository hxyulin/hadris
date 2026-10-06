#![cfg(all(feature = "async", feature = "sync", feature = "alloc"))]

use core::future::Future;
use core::task::{Context, Poll, Waker};
use hadris_io::ErrorKind;
use hadris_storage::async_::{BlockDevice, Local};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

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
fn send_and_local_partition_writes_preserve_boundaries() {
    let sector = BlockSize::new(512).unwrap();
    let mut disk = MemDevice::new(vec![0xA5; 4 * 512], sector);
    block_on(async {
        let mut send = Partition::new(&mut disk, 512, 2 * 512);
        assert!(BlockDevice::writable(&send));
        assert_eq!(BlockDevice::disk_offset(&send), 512);
        BlockDevice::write_blocks(&mut send, BlockIndex::new(0), &[1; 512])
            .await
            .unwrap();
        BlockDevice::flush(&mut send).await.unwrap();
    });
    block_on(async {
        let mut local = Local::new(Partition::new(&mut disk, 512, 2 * 512));
        assert_eq!(local.block_count(), 2);
        assert_eq!(local.max_block_count(), 2);
        assert_eq!(local.disk_offset(), 512);
        let mut data = [0; 512];
        local
            .read_blocks(BlockIndex::new(0), &mut data)
            .await
            .unwrap();
        assert_eq!(data, [1; 512]);
        local
            .write_blocks(BlockIndex::new(1), &[2; 512])
            .await
            .unwrap();
        local.flush().await.unwrap();
        let error = local
            .write_blocks(BlockIndex::new(2), &[3; 512])
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert_eq!(local.into_inner().len(), 2 * 512);
    });
    assert_eq!(&disk.get_ref()[..512], &[0xA5; 512]);
    assert_eq!(&disk.get_ref()[512..1024], &[1; 512]);
    assert_eq!(&disk.get_ref()[1024..1536], &[2; 512]);
    assert_eq!(&disk.get_ref()[1536..], &[0xA5; 512]);
}

#[test]
fn local_adapter_preserves_growth_and_read_only_errors() {
    block_on(async {
        let mut growable = Local::new(Vec::<u8>::new());
        assert!(growable.max_block_count() > growable.block_count());
        growable
            .write_blocks(BlockIndex::new(1), &[7; 512])
            .await
            .unwrap();
        assert_eq!(growable.block_count(), 2);
        assert_eq!(&growable.into_inner()[..512], &[0; 512]);
        let data = [0; 512];
        let mut read_only = Local::new(MemDevice::new(&data[..], BlockSize::new(512).unwrap()));
        assert!(!read_only.writable());
        let error = read_only
            .write_blocks(BlockIndex::new(0), &data)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ReadOnly);
    });
}
