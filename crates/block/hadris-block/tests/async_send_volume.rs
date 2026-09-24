#![cfg(feature = "async-send")]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_block::Detail;
use hadris_block::async_send::OpenVolume;
use hadris_block::detect::{BlockFormat, FatVariant, PartitionTableKind};
use hadris_block::part::async_send::open;
use hadris_block::part::{Disk, Mbr, MbrEntry, MbrType, Partition};
use hadris_fat::{FatKind, FormatOptions};
use hadris_fs::async_send::DriverExt;
use hadris_fs::{Error, ErrorKind, MountError};
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

/// A partition of `len` blocks from `start` in an MBR for a disk of
/// `blocks` blocks.
fn mbr_partition(blocks: u64, start: u64, len: u64) -> (Disk, Partition) {
    let mut mbr = Mbr::new(blocks, BlockSize::new(512).unwrap()).unwrap();
    mbr.add(MbrEntry::new(MbrType::FAT12, start, len)).unwrap();
    let disk = Disk::new(mbr);
    let entry = disk.partition(0).unwrap();
    (disk, entry)
}

/// A disk with an MBR whose first partition holds a FAT12 volume.
fn partitioned_disk() -> (Device, Partition) {
    let blocks = (VOLUME_LEN / 512) as u64;
    let (table, entry) = mbr_partition(blocks + 1, 1, blocks);
    let mut disk = device(vec![0_u8; VOLUME_LEN + 512]);
    block_on(hadris_block::part::async_send::write(&mut disk, &table)).unwrap();
    formatted_fat12(open(&mut disk, &entry).unwrap());
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
        assert_eq!(
            err.error().detail().and_then(Detail::from_code),
            Some(Detail::PartitionedDisk)
        );

        let partition = open(&mut disk, &entry).unwrap();
        let volume = OpenVolume::open(partition).await.unwrap();
        assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::Fat12));
        let mut fs = volume.into_fat().ok().unwrap();
        fs.write_file("/HELLO.TXT", b"hello").await.unwrap();
        assert_eq!(fs.read_to_vec("/HELLO.TXT").await.unwrap(), b"hello");
        fs.sync().await.unwrap();
        assert_eq!(fs.into_inner().first(), BlockIndex::new(1));
    });
}

#[test]
fn opens_exfat() {
    block_on(async {
        let dev = device(vec![0_u8; VOLUME_LEN]);
        let options = hadris_fat::exfat::FormatOptions::new();
        let dev = hadris_fat::exfat::async_send::format(dev, options)
            .await
            .unwrap()
            .into_inner();
        let volume = OpenVolume::open(dev).await.unwrap();
        assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::ExFat));
        let mut fs = volume.into_exfat().ok().unwrap();
        fs.write_file("/a.txt", b"exfat").await.unwrap();
        assert_eq!(fs.read_to_vec("/a.txt").await.unwrap(), b"exfat");
        fs.sync().await.unwrap();
    });
}

#[test]
fn failures_give_the_device_back() {
    block_on(async {
        let dev = formatted_fat12(device(vec![0_u8; VOLUME_LEN]));
        let (error, dev) = OpenVolume::open_detected(dev, BlockFormat::Fat(FatVariant::Fat32))
            .await
            .err()
            .unwrap()
            .into_parts();
        assert!(matches!(
            error.detail().and_then(Detail::from_code),
            Some(Detail::FormatMismatch)
        ));
        assert_eq!(dev.get_ref().len(), VOLUME_LEN);

        let error: Error<_> = OpenVolume::open(device(vec![0_u8; 1024]))
            .await
            .map_err(MountError::into_error)
            .err()
            .unwrap();
        assert_eq!(error.kind(), ErrorKind::NotRecognized);

        let (_, past) = mbr_partition(8192, 4096, 16);
        let mut dev = dev;
        assert!(open(&mut dev, &past).is_err());
        assert_eq!(dev.get_ref().len(), VOLUME_LEN);

        let mut image = dev.into_inner();
        image.truncate(VOLUME_LEN / 2);
        let (error, dev) = OpenVolume::open(device(image.clone()))
            .await
            .err()
            .unwrap()
            .into_parts();
        assert_eq!(error.detail().and_then(Detail::from_code), None);
        assert_eq!(dev.into_inner(), image);
    });
}
