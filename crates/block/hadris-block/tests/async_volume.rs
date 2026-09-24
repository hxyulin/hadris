#![cfg(feature = "async")]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_block::Detail;
use hadris_block::r#async::OpenVolume;
use hadris_block::detect::{BlockFormat, FatVariant};
use hadris_fat::{FatKind, FormatOptions};
use hadris_fs::MountError;
use hadris_fs::r#async::DriverExt;
use hadris_fs::{ErrorKind, OpenOptions};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

type Device = MemDevice<Vec<u8>>;

fn device(bytes: Vec<u8>) -> Device {
    MemDevice::new(bytes, BlockSize::new(512).unwrap())
}

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

fn formatted_fat12() -> Vec<u8> {
    let dev = device(vec![0_u8; 2 * 1024 * 1024]);
    let options = FormatOptions::new().with_kind(FatKind::Fat12);
    let fs = block_on(hadris_fat::r#async::format(dev, options)).unwrap();
    fs.into_inner().into_inner()
}

fn populated_gpt() -> hadris_block::part::Disk {
    use hadris_block::part::gpt::types;
    use hadris_block::part::{DiskLayout, Guid, PartitionSpec, Size};

    DiskLayout::gpt(Guid::from_bytes([0x31; 16]))
        .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::Blocks(4096)).with_start(40))
        .build(8192, BlockSize::new(512).unwrap())
        .unwrap()
}

fn populated_mbr() -> hadris_block::part::Disk {
    use hadris_block::part::{Disk, Mbr, MbrEntry, MbrType};

    let mut mbr = Mbr::new(8192, BlockSize::new(512).unwrap()).unwrap();
    mbr.add(MbrEntry::new(MbrType::FAT32, 2048, 4096)).unwrap();
    mbr.add(MbrEntry::new(MbrType::LINUX, 6144, 2048)).unwrap();
    Disk::new(mbr)
}

/// The first partition of a one-partition MBR for a disk of `blocks` blocks.
fn mbr_partition(blocks: u64, start: u64, len: u64) -> hadris_block::part::Partition {
    use hadris_block::part::{Disk, Mbr, MbrEntry, MbrType};

    let mut mbr = Mbr::new(blocks, BlockSize::new(512).unwrap()).unwrap();
    mbr.add(MbrEntry::new(MbrType::FAT12, start, len)).unwrap();
    Disk::new(mbr).partition(0).unwrap()
}

#[test]
fn async_partition_slices_enforce_their_bounds() {
    use hadris_block::part::r#async::open;
    use hadris_storage::r#async::BlockDevice;

    let bytes: Vec<u8> = (0..16 * 512).map(|index| (index / 512) as u8).collect();
    block_on(async {
        let mut disk = device(bytes);
        let entry = mbr_partition(16, 4, 8);
        let mut slice = open(&mut disk, &entry).unwrap();
        assert_eq!((slice.offset(), slice.block_count()), (4 * 512, 8));
        assert_eq!(slice.disk_offset(), 4 * 512);
        let mut block = [0_u8; 512];
        slice
            .read_blocks(BlockIndex::new(0), &mut block)
            .await
            .unwrap();
        assert_eq!(block, [4; 512]);
        slice
            .read_blocks(BlockIndex::new(7), &mut block)
            .await
            .unwrap();
        assert_eq!(block, [11; 512]);
        assert!(
            slice
                .read_blocks(BlockIndex::new(8), &mut block)
                .await
                .is_err()
        );

        let past = mbr_partition(100, 12, 8);
        assert!(open(&mut disk, &past).is_err());
        assert_eq!(disk.get_ref().len(), 16 * 512);
    });
}

#[test]
fn async_opens_detected_exfat() {
    block_on(async {
        let dev = device(vec![0_u8; 2 << 20]);
        let options = hadris_fat::exfat::FormatOptions::new();
        let dev = hadris_fat::exfat::r#async::format(dev, options)
            .await
            .unwrap()
            .into_inner();
        let mut volume = OpenVolume::open(dev).await.unwrap();
        assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::ExFat));
        volume.write_file("/a.txt", b"exfat").await.unwrap();
        assert_eq!(volume.read_to_vec("/A.TXT").await.unwrap(), b"exfat");
        volume.sync().await.unwrap();

        let mut image = vec![0_u8; 512];
        image[3..11].copy_from_slice(b"EXFAT   ");
        image[510..512].copy_from_slice(&[0x55, 0xaa]);
        let error = OpenVolume::open(device(image))
            .await
            .map(|_| ())
            .map_err(MountError::into_error)
            .unwrap_err();
        assert_eq!(error.detail().and_then(Detail::from_code), None);
    });
}

#[test]
fn async_detection_and_open_release_the_device() {
    let image = formatted_fat12();
    block_on(async {
        let mut dev = device(image);
        let detected = hadris_block::detect::r#async::detect(&mut dev)
            .await
            .unwrap();
        assert_eq!(
            detected,
            Some(hadris_block::detect::BlockFormat::Fat(FatVariant::Fat12))
        );

        let volume = OpenVolume::open(&mut dev).await.unwrap();
        assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::Fat12));
        assert!(volume.as_fat().is_some());
        let dev = volume.into_inner();
        assert_eq!(dev.get_ref().len(), 2 * 1024 * 1024);
    });
}

#[test]
fn async_open_reports_mismatch() {
    let image = formatted_fat12();
    block_on(async {
        let err = OpenVolume::open_detected(device(image), BlockFormat::Fat(FatVariant::Fat16))
            .await
            .err()
            .unwrap();
        assert!(matches!(
            err.error().detail().and_then(Detail::from_code),
            Some(Detail::FormatMismatch)
        ));
        let dev = err.into_device();
        assert_eq!(dev.get_ref().len(), 2 * 1024 * 1024);
    });
}

#[test]
fn async_detected_volume_that_fails_to_mount_gives_the_device_back() {
    let mut image = formatted_fat12();
    image.truncate(image.len() / 2);
    block_on(async {
        let mut dev = device(image.clone());
        assert_eq!(
            hadris_block::detect::r#async::detect(&mut dev)
                .await
                .unwrap(),
            Some(BlockFormat::Fat(FatVariant::Fat12))
        );
        let (error, dev) = OpenVolume::open(dev).await.err().unwrap().into_parts();
        let fat12 = BlockFormat::Fat(FatVariant::Fat12);
        assert_eq!(error.detail().and_then(Detail::from_code), None);
        assert_eq!(error.kind(), ErrorKind::Corrupt);
        assert_eq!(dev.get_ref(), &image);

        let err = OpenVolume::open_detected(dev, fat12).await.err().unwrap();
        assert_eq!(err.error().detail().and_then(Detail::from_code), None);
        assert_eq!(err.into_device().into_inner(), image);
    });
}

#[test]
fn async_fat_content_mutation_traversal_and_recovery() {
    let image = formatted_fat12();
    block_on(async {
        let volume = OpenVolume::open(device(image)).await.unwrap();
        let mut fs = volume.into_fat().ok().unwrap();

        let payload: Vec<u8> = (0..1537).map(|index| (index % 251) as u8).collect();
        fs.create_dir_all("/NESTED").await.unwrap();
        fs.write_file("/NESTED/PAYLOAD.BIN", &payload)
            .await
            .unwrap();
        assert_eq!(fs.read_to_vec("NESTED/PAYLOAD.BIN").await.unwrap(), payload);

        let mut file = fs
            .open("/NESTED/PAYLOAD.BIN", OpenOptions::write())
            .await
            .unwrap();
        file.set_len(513).await.unwrap();
        file.close().await.unwrap();
        assert_eq!(
            fs.read_to_vec("/NESTED/PAYLOAD.BIN").await.unwrap(),
            payload[..513]
        );

        assert!(fs.metadata("/NESTED").await.unwrap().file_type().is_dir());
        assert_eq!(
            fs.read_to_vec("/MISSING.BIN").await.unwrap_err().kind(),
            ErrorKind::NotFound
        );
        fs.sync().await.unwrap();

        let dev = fs.into_inner();
        let mut fs = hadris_fat::r#async::FatFs::open(dev).await.unwrap();
        assert_eq!(
            fs.read_to_vec("/NESTED/PAYLOAD.BIN").await.unwrap(),
            payload[..513]
        );
    });
}

#[test]
fn async_partition_table_gpt_write_detect_open_and_reject_malformed() {
    use hadris_block::detect::PartitionTableKind;
    use hadris_block::part::r#async::{read, write};
    use hadris_block::part::{GptCopy, PartitionTable};

    block_on(async {
        let table = populated_gpt();
        let mut disk = device(vec![0_u8; 8192 * 512]);
        write(&mut disk, &table).await.unwrap();
        assert_eq!(
            hadris_block::detect::r#async::detect(&mut disk)
                .await
                .unwrap(),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        );

        let opened = read(&mut disk).await.unwrap();
        assert_eq!(opened, table);
        assert_eq!(opened.partitions().count(), 1);

        let mut truncated = device(disk.get_ref()[..512].to_vec());
        assert_eq!(
            read(&mut truncated).await.unwrap_err().kind(),
            ErrorKind::Corrupt
        );

        let mut corrupt = disk.into_inner();
        corrupt[512..520].copy_from_slice(b"NOT GPT!");
        let recovered = read(&mut device(corrupt)).await.unwrap();
        let PartitionTable::Gpt(gpt) = recovered.table() else {
            panic!("expected a GPT");
        };
        assert_eq!(gpt.damaged_copy(), Some(GptCopy::Primary));
        assert_eq!(recovered.partitions().count(), 1);
    });
}

#[test]
fn async_partition_table_mbr_write_detect_open_and_reject_malformed() {
    use hadris_block::detect::PartitionTableKind;
    use hadris_block::part::r#async::{read, write};

    block_on(async {
        let mut disk = device(vec![0_u8; 8192 * 512]);
        write(&mut disk, &populated_mbr()).await.unwrap();
        assert_eq!(
            hadris_block::detect::r#async::detect(&mut disk)
                .await
                .unwrap(),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Mbr))
        );

        let opened = read(&mut disk).await.unwrap();
        assert_eq!(opened, populated_mbr());
        let partitions: Vec<_> = opened.partitions().map(|p| (p.start(), p.len())).collect();
        assert_eq!(partitions, [(2048, 4096), (6144, 2048)]);

        assert_eq!(
            read(&mut device(vec![0_u8; 64])).await.unwrap_err().kind(),
            ErrorKind::InvalidInput
        );

        let mut invalid = vec![0_u8; 512];
        invalid[510..].copy_from_slice(&[0x12, 0x34]);
        assert_eq!(
            read(&mut device(invalid)).await.unwrap_err().kind(),
            ErrorKind::NotRecognized
        );
    });
}

#[test]
fn async_partition_table_opens_fat_through_a_gpt_view() {
    use hadris_block::part::r#async::{open, read, write};

    let mut bytes = vec![0_u8; 8192 * 512];
    let start = 40 * 512;
    let end = start + 4096 * 512;
    bytes[start..end].copy_from_slice(&{
        let dev = device(vec![0_u8; end - start]);
        let options = FormatOptions::new().with_kind(FatKind::Fat12);
        let fs = block_on(hadris_fat::r#async::format(dev, options)).unwrap();
        fs.into_inner().into_inner()
    });

    block_on(async {
        let mut disk = device(bytes);
        write(&mut disk, &populated_gpt()).await.unwrap();
        let table = read(&mut disk).await.unwrap();
        let entry = table.partition(0).unwrap();
        let partition = open(&mut disk, &entry).unwrap();
        let volume = OpenVolume::open(partition).await.unwrap();
        assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::Fat12));
        let mut fs = volume.into_fat().ok().unwrap();
        assert!(fs.read_dir("/").await.unwrap().next_entry().await.is_none());
    });
}

#[test]
fn async_partition_table_hybrid_write_open_roundtrip() {
    use hadris_block::part::r#async::{read, write};
    use hadris_block::part::{
        Disk, Hybrid, HybridMbr, MbrType, PartitionFlags, PartitionTable, TableKind,
    };

    let PartitionTable::Gpt(gpt) = populated_gpt().into_table() else {
        unreachable!();
    };
    let mut config = HybridMbr::new().with_protective_slot(3);
    config
        .add_mirrored(0, MbrType::EFI_SYSTEM, PartitionFlags::BOOTABLE)
        .unwrap();
    let table = Disk::new(Hybrid::new(gpt, &config).unwrap());

    block_on(async {
        let mut disk = device(vec![0_u8; 8192 * 512]);
        write(&mut disk, &table).await.unwrap();
        let opened = read(&mut disk).await.unwrap();
        assert_eq!(opened.table().kind(), TableKind::Hybrid);
        assert_eq!(opened, table);
    });
}

#[test]
fn async_unknown_block_input_is_category_typed() {
    block_on(async {
        let mut dev = device(vec![0xA5_u8; 4096]);
        assert_eq!(
            hadris_block::detect::r#async::detect(&mut dev)
                .await
                .unwrap(),
            None
        );
        let (error, dev) = OpenVolume::open(dev).await.err().unwrap().into_parts();
        assert_eq!(error.kind(), ErrorKind::NotRecognized);
        assert_eq!(dev.get_ref().len(), 4096);
    });
}

#[path = "../../hadris-ntfs/tests/support/image.rs"]
mod ntfs_image;

#[test]
fn async_opens_ntfs_and_passes_the_contract() {
    block_on(async {
        let mut volume = OpenVolume::open(device(ntfs_image::base_image()))
            .await
            .unwrap();
        assert_eq!(volume.format(), BlockFormat::Ntfs);
        assert_eq!(
            volume.read_to_vec("/HELLO.TXT").await.unwrap(),
            b"hello ntfs"
        );
        hadris_fs::r#async::contract::check_read_only(&mut volume)
            .await
            .unwrap();
    });
}
