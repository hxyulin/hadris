#![cfg(feature = "async-send")]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_block::async_send::OpenVolume;
use hadris_block::detect::{BlockFormat, FatVariant, PartitionTableKind};
use hadris_block::part::{MbrPartition, MbrPartitionType};
use hadris_block::partition::async_send::mbr_partition;
use hadris_block::{Error, OpenError};
use hadris_fat::{FatKind, FormatOptions};
use hadris_fs::async_send::DriverExt;
use hadris_storage::async_send::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

type Device = MemDevice<Vec<u8>>;

const VOLUME_LEN: usize = 2 * 1024 * 1024;

fn device(bytes: Vec<u8>) -> Device {
    MemDevice::new(bytes, BlockSize::new(512).unwrap())
}

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Polls `future` to completion, requiring it to be `Send`.
fn block_on<F: Future + Send>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn formatted_fat12<D: BlockDevice>(dev: D) -> D {
    let options = FormatOptions::new().with_kind(FatKind::Fat12);
    block_on(hadris_fat::async_send::format(dev, options))
        .unwrap()
        .into_inner()
}

/// A disk with an MBR whose first partition holds a FAT12 volume.
fn partitioned_disk() -> (Device, MbrPartition) {
    let entry = MbrPartition::new(MbrPartitionType::Fat12, 1, (VOLUME_LEN / 512) as u32);
    let mut bytes = vec![0_u8; VOLUME_LEN + 512];
    bytes[446..462].copy_from_slice(bytemuck::bytes_of(&entry));
    bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
    let mut disk = device(bytes);
    formatted_fat12(mbr_partition(&mut disk, &entry).unwrap());
    (disk, entry)
}

#[test]
fn opens_fat_through_an_mbr_partition() {
    let (mut disk, entry) = partitioned_disk();
    block_on(async {
        assert_eq!(
            hadris_block::detect::async_send::detect(&mut disk)
                .await
                .unwrap(),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Mbr))
        );
        let err = OpenVolume::open(&mut disk).await.err().unwrap();
        assert!(matches!(
            err.error(),
            Error::PartitionedDisk(PartitionTableKind::Mbr)
        ));

        let partition = mbr_partition(&mut disk, &entry).unwrap();
        let volume = OpenVolume::open(partition).await.unwrap();
        assert_eq!(volume.format(), FatVariant::Fat12);
        let mut fs = volume.into_fat().ok().unwrap();
        fs.write_file("/HELLO.TXT", b"hello").await.unwrap();
        assert_eq!(fs.read_to_vec("/HELLO.TXT").await.unwrap(), b"hello");
        fs.sync().await.unwrap();
        assert_eq!(fs.into_inner().first(), BlockIndex::new(1));
    });
}

#[test]
fn failures_give_the_device_back() {
    block_on(async {
        let dev = formatted_fat12(device(vec![0_u8; VOLUME_LEN]));
        let (error, dev) = OpenVolume::open_detected(dev, FatVariant::Fat32)
            .await
            .err()
            .unwrap()
            .into_parts();
        assert!(matches!(
            error,
            Error::DetectedFormatMismatch {
                detected: FatVariant::Fat32,
                opened: FatVariant::Fat12,
            }
        ));
        assert_eq!(dev.get_ref().len(), VOLUME_LEN);

        let error: Error<_> = OpenVolume::open(device(vec![0_u8; 1024]))
            .await
            .map_err(OpenError::into_error)
            .err()
            .unwrap();
        assert!(matches!(error, Error::UnknownFormat));

        let past = MbrPartition::new(MbrPartitionType::Fat12, 4096, 16);
        let dev = mbr_partition(dev, &past).err().unwrap();
        assert_eq!(dev.get_ref().len(), VOLUME_LEN);

        let mut image = dev.into_inner();
        image.truncate(VOLUME_LEN / 2);
        let (error, dev) = OpenVolume::open(device(image.clone()))
            .await
            .err()
            .unwrap()
            .into_parts();
        assert!(matches!(error, Error::Fat(_)));
        assert_eq!(dev.into_inner(), image);
    });
}
