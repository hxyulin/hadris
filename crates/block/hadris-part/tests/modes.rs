//! The `sync`, `r#async` and `async_send` modes read, scan, write and open
//! the same disks the same way.

use core::future::Future;
use core::ops::ControlFlow;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_part::gpt::types;
use hadris_part::{
    Alignment, Disk, DiskLayout, Guid, MbrType, Partition, PartitionSpec, Size, TableKind,
};
use hadris_storage::{BlockSize, MemDevice};

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
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

fn assert_send<T: Send>(value: T) -> T {
    value
}

const B512: BlockSize = BlockSize::new(512).unwrap();

fn layouts() -> Vec<DiskLayout> {
    let guid = Guid::from_bytes([0x3C; 16]);
    let mut mbr = DiskLayout::mbr().with_alignment(Alignment::Blocks(8));
    for kind in [
        MbrType::FAT32_LBA,
        MbrType::LINUX,
        MbrType::LINUX_SWAP,
        MbrType::NTFS,
        MbrType::LINUX,
    ] {
        mbr = mbr.partition(PartitionSpec::new(kind, Size::Blocks(100)));
    }
    vec![
        mbr,
        DiskLayout::gpt(guid)
            .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::KiB(512)).with_name("EFI"))
            .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining)),
        DiskLayout::hybrid(guid)
            .partition(
                PartitionSpec::new(types::EFI_SYSTEM, Size::KiB(512))
                    .with_mirror(MbrType::EFI_SYSTEM),
            )
            .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining)),
    ]
}

fn scan_sync(dev: &mut MemDevice<Vec<u8>>) -> (TableKind, Vec<Partition>) {
    let mut found = Vec::new();
    let kind = hadris_part::sync::scan(dev, |p| {
        found.push(p);
        ControlFlow::Continue(())
    })
    .unwrap();
    (kind, found)
}

#[test]
fn every_mode_reads_scans_writes_and_opens_alike() {
    for layout in layouts() {
        let mut sync_dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
        let disk: Disk = hadris_part::sync::create(&mut sync_dev, &layout).unwrap();
        let (kind, listed) = scan_sync(&mut sync_dev);
        assert_eq!(kind, disk.table().kind());
        assert_eq!(listed, disk.partitions().collect::<Vec<_>>());

        block_on(async {
            let mut dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
            let created = hadris_part::r#async::create(&mut dev, &layout)
                .await
                .unwrap();
            assert_eq!(created, disk);
            assert_eq!(dev.get_ref(), sync_dev.get_ref());
            assert_eq!(hadris_part::r#async::read(&mut dev).await.unwrap(), disk);
            let mut found = Vec::new();
            let scanned = hadris_part::r#async::scan(&mut dev, |p| {
                found.push(p);
                ControlFlow::Continue(())
            })
            .await
            .unwrap();
            assert_eq!((scanned, found), (kind, listed.clone()));
            let slice = hadris_part::r#async::open(&mut dev, &listed[0]).unwrap();
            assert_eq!(
                hadris_storage::r#async::BlockDevice::block_count(&slice),
                listed[0].len()
            );
        });

        let mut dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
        let write = assert_send(hadris_part::async_send::write(&mut dev, &disk));
        block_on(write).unwrap();
        assert_eq!(dev.get_ref(), sync_dev.get_ref());
        let read = assert_send(hadris_part::async_send::read(&mut dev));
        assert_eq!(block_on(read).unwrap(), disk);
        let mut count = 0;
        let scan = assert_send(hadris_part::async_send::scan(&mut dev, |_| {
            count += 1;
            ControlFlow::Continue(())
        }));
        assert_eq!(block_on(scan).unwrap(), kind);
        assert_eq!(count, listed.len());
        let slice = hadris_part::async_send::open(&mut dev, &listed[1]).unwrap();
        assert_eq!(
            hadris_storage::async_send::BlockDevice::block_count(&slice),
            listed[1].len()
        );
    }
}

#[test]
fn every_mode_falls_back_to_the_backup_gpt() {
    let layout = &layouts()[1];
    let mut dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
    let disk = hadris_part::sync::create(&mut dev, layout).unwrap();
    dev.get_mut()[512 + 8] ^= 0xFF;
    let recovered = hadris_part::sync::read(&mut dev).unwrap();
    assert_eq!(
        recovered.partitions().collect::<Vec<_>>(),
        disk.partitions().collect::<Vec<_>>()
    );
    let from_async = block_on(hadris_part::r#async::read(&mut dev)).unwrap();
    let from_send = block_on(hadris_part::async_send::read(&mut dev)).unwrap();
    assert_eq!(from_async, recovered);
    assert_eq!(from_send, recovered);
}
