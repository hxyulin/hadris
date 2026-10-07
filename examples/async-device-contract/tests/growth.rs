#![cfg(feature = "alloc")]

use core::future::Future;
use core::task::{Context, Poll, Waker};
use hadris_experiment_async_device_contract::{BlockDevice, Cache};
use hadris_io::ErrorKind;
use hadris_storage::{BlockIndex, Partition};

fn complete<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

#[test]
fn borrowed_growing_device_works_through_cache_and_preserves_zero_filled_gaps() {
    let mut device = Vec::<u8>::new();
    assert_eq!(device.block_count(), 0);
    assert!(device.max_block_count() > 0);
    {
        let mut cache = Cache::new(&mut device);
        complete(cache.write_blocks(BlockIndex::new(2), &[9; 512])).unwrap();
        assert_eq!(cache.block_count(), 3);
        let mut gap = [7; 512];
        complete(cache.read_blocks(BlockIndex::new(1), &mut gap)).unwrap();
        assert_eq!(gap, [0; 512]);
        let mut data = [0; 512];
        complete(cache.read_blocks(BlockIndex::new(2), &mut data)).unwrap();
        assert_eq!(data, [9; 512]);
        complete(cache.flush()).unwrap();
    }
    assert_eq!(device.len(), 1536);
    assert_eq!(&device[..1024], &[0; 1024]);
    assert_eq!(&device[1024..], &[9; 512]);
}

#[test]
fn partition_writes_allow_backing_growth_but_keep_the_partition_bound() {
    let mut device = Vec::<u8>::new();
    {
        let mut part = Partition::new(&mut device, 512, 1024);
        complete(part.write_blocks(BlockIndex::new(1), &[3; 512])).unwrap();
        let mut data = [0; 512];
        complete(part.read_blocks(BlockIndex::new(1), &mut data)).unwrap();
        assert_eq!(data, [3; 512]);
        let err = complete(part.write_blocks(BlockIndex::new(2), &[4; 512])).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }
    assert_eq!(device.len(), 1536);
    assert_eq!(&device[..1024], &[0; 1024]);
    assert_eq!(&device[1024..], &[3; 512]);
}
