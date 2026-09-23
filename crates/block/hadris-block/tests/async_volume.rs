#![cfg(feature = "async")]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_block::Error;
use hadris_block::r#async::OpenVolume;
use hadris_block::detect::{BlockFormat, FatVariant};
use hadris_fat::{FatKind, FormatOptions};
use hadris_fs::r#async::DriverExt;
use hadris_fs::{ErrorKind, OpenOptions};
use hadris_io::SeekFrom;
use hadris_io::legacy::r#async::{Read, Seek, Write};
use hadris_storage::PartitionView;
use hadris_storage::{BlockSize, MemDevice};

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

fn populated_gpt() -> hadris_block::part::PartitionTable {
    use hadris_block::part::{GptPartitionEntry, Guid, PartitionTable};

    let mut scheme = PartitionTable::new_gpt(8192, 512);
    let PartitionTable::Gpt { gpt, .. } = &mut scheme else {
        unreachable!();
    };
    gpt.add_partition(GptPartitionEntry::new(
        Guid::EFI_SYSTEM,
        Guid::from_bytes([0x31; 16]),
        40,
        4135,
    ))
    .unwrap();
    scheme
}

fn populated_mbr() -> hadris_block::part::PartitionTable {
    use hadris_block::part::{MasterBootRecord, MbrPartition, MbrPartitionType, PartitionTable};

    let mut mbr = MasterBootRecord::default();
    mbr.with_partition_table(|table| {
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 4096);
        table[1] = MbrPartition::new(MbrPartitionType::LinuxNative, 6144, 2048);
    });
    PartitionTable::Mbr(mbr)
}

struct AsyncCursor {
    bytes: Vec<u8>,
    position: u64,
}

impl AsyncCursor {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, position: 0 }
    }
}

impl Read for AsyncCursor {
    async fn read(&mut self, buffer: &mut [u8]) -> hadris_io::legacy::Result<usize> {
        let start = usize::try_from(self.position).map_err(|_| {
            hadris_io::legacy::Error::from_kind(hadris_io::legacy::ErrorKind::InvalidInput)
        })?;
        let available = self.bytes.len().saturating_sub(start);
        let len = available.min(buffer.len());
        buffer[..len].copy_from_slice(&self.bytes[start..start + len]);
        self.position += len as u64;
        Ok(len)
    }
}

impl Write for AsyncCursor {
    async fn write(&mut self, buffer: &[u8]) -> hadris_io::legacy::Result<usize> {
        let start = usize::try_from(self.position).map_err(|_| {
            hadris_io::legacy::Error::from_kind(hadris_io::legacy::ErrorKind::InvalidInput)
        })?;
        let end = start.checked_add(buffer.len()).ok_or_else(|| {
            hadris_io::legacy::Error::from_kind(hadris_io::legacy::ErrorKind::InvalidInput)
        })?;
        if end > self.bytes.len() {
            return Err(hadris_io::legacy::Error::from_kind(
                hadris_io::legacy::ErrorKind::WriteZero,
            ));
        }
        self.bytes[start..end].copy_from_slice(buffer);
        self.position = end as u64;
        Ok(buffer.len())
    }

    async fn flush(&mut self) -> hadris_io::legacy::Result<()> {
        Ok(())
    }
}

impl Seek for AsyncCursor {
    async fn seek(&mut self, position: SeekFrom) -> hadris_io::legacy::Result<u64> {
        let next = match position {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => self.bytes.len() as i128 + i128::from(offset),
        };
        if !(0..=self.bytes.len() as i128).contains(&next) {
            return Err(hadris_io::legacy::Error::from_kind(
                hadris_io::legacy::ErrorKind::InvalidInput,
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}

#[test]
fn async_partition_view_enforces_relative_bounds() {
    block_on(async {
        let bytes = [0_u8, 1, 2, 3, 4, 5, 6, 7];
        let mut source = hadris_io::Cursor::new(&bytes);
        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();
        let mut buffer = [0_u8; 8];
        assert_eq!(view.read(&mut buffer).await.unwrap(), 4);
        assert_eq!(&buffer[..4], &[2, 3, 4, 5]);
        assert_eq!(view.read(&mut buffer).await.unwrap(), 0);
        assert!(view.seek(SeekFrom::Start(5)).await.is_err());
        assert_eq!(view.seek(SeekFrom::End(-1)).await.unwrap(), 3);
    });
}

#[test]
fn async_detects_exfat_but_rejects_unified_opening() {
    block_on(async {
        let mut image = vec![0_u8; 512];
        image[3..11].copy_from_slice(b"EXFAT   ");
        image[510..512].copy_from_slice(&[0x55, 0xaa]);

        assert!(matches!(
            OpenVolume::open(device(image)).await,
            Err(Error::UnsupportedFormat(BlockFormat::Fat(
                FatVariant::ExFat
            )))
        ));
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
        assert_eq!(volume.format(), FatVariant::Fat12);
        assert!(volume.as_fat().is_some());
        let dev = volume.into_inner();
        assert_eq!(dev.get_ref().len(), 2 * 1024 * 1024);
    });
}

#[test]
fn async_open_reports_mismatch() {
    let image = formatted_fat12();
    block_on(async {
        assert!(matches!(
            OpenVolume::open_detected(device(image), FatVariant::Fat16).await,
            Err(hadris_block::Error::DetectedFormatMismatch { .. })
        ));
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
    use hadris_block::part::PartitionSchemeType;
    use hadris_block::part::r#async::scheme_io::PartitionTableWriteExt;

    block_on(async {
        let scheme = populated_gpt();
        let mut disk = AsyncCursor::new(vec![0_u8; 8192 * 512]);
        scheme.write_to(&mut disk).await.unwrap();

        disk.seek(SeekFrom::Start(91)).await.unwrap();
        assert_eq!(
            hadris_block::part::r#async::partition_table::detect(&mut disk)
                .await
                .unwrap(),
            PartitionSchemeType::Gpt
        );
        assert_eq!(disk.stream_position().await.unwrap(), 91);

        let opened = hadris_block::part::r#async::partition_table::open(&mut disk, 512)
            .await
            .unwrap();
        opened.validate().unwrap();
        assert_eq!(opened.partitions().len(), 1);

        let mut truncated = AsyncCursor::new(disk.bytes[..512].to_vec());
        assert!(matches!(
            hadris_block::part::r#async::partition_table::open(&mut truncated, 512).await,
            Err(hadris_block::part::Error::Io(error))
                if error.kind() == hadris_io::legacy::ErrorKind::UnexpectedEof
        ));

        let mut corrupt = disk.bytes;
        corrupt[512..520].copy_from_slice(b"NOT GPT!");
        assert!(matches!(
            hadris_block::part::r#async::partition_table::open(
                &mut AsyncCursor::new(corrupt),
                512,
            ).await,
            Err(hadris_block::part::Error::InvalidGptSignature { .. })
        ));
    });
}

#[test]
fn async_partition_table_mbr_write_detect_open_and_reject_malformed() {
    use hadris_block::part::r#async::scheme_io::PartitionTableWriteExt;
    use hadris_block::part::{Error, PartitionSchemeType};

    block_on(async {
        let mut disk = AsyncCursor::new(vec![0_u8; 8192 * 512]);
        populated_mbr().write_to(&mut disk).await.unwrap();

        disk.seek(SeekFrom::Start(47)).await.unwrap();
        assert_eq!(
            hadris_block::part::r#async::partition_table::detect(&mut disk)
                .await
                .unwrap(),
            PartitionSchemeType::Mbr
        );
        assert_eq!(disk.stream_position().await.unwrap(), 47);

        let opened = hadris_block::part::r#async::partition_table::open(&mut disk, 512)
            .await
            .unwrap();
        assert_eq!(opened.scheme_type(), PartitionSchemeType::Mbr);
        opened.validate().unwrap();
        let partitions = opened.partitions();
        assert_eq!(partitions.len(), 2);
        assert_eq!(
            (partitions[0].start_lba, partitions[0].size_sectors),
            (2048, 4096)
        );
        assert_eq!(
            (partitions[1].start_lba, partitions[1].size_sectors),
            (6144, 2048)
        );

        assert!(matches!(
            hadris_block::part::r#async::partition_table::open(
                &mut AsyncCursor::new(vec![0_u8; 64]),
                512,
            )
            .await,
            Err(Error::Io(error))
                if error.kind() == hadris_io::legacy::ErrorKind::UnexpectedEof
        ));

        let mut invalid = vec![0_u8; 512];
        invalid[510..].copy_from_slice(&[0x12, 0x34]);
        assert!(matches!(
            hadris_block::part::r#async::partition_table::open(
                &mut AsyncCursor::new(invalid),
                512,
            )
            .await,
            Err(Error::InvalidMbrSignature { found: [0x12, 0x34] })
        ));
    });
}

#[test]
fn async_partition_table_opens_fat_through_a_gpt_view() {
    use hadris_block::part::r#async::scheme_io::PartitionTableWriteExt;

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
        let mut disk = AsyncCursor::new(bytes);
        populated_gpt().write_to(&mut disk).await.unwrap();
        let table = hadris_block::part::r#async::partition_table::open(&mut disk, 512)
            .await
            .unwrap();
        let entry = match &table {
            hadris_block::part::PartitionTable::Gpt { gpt, .. } => &gpt.entries[0],
            _ => unreachable!(),
        };
        let mut disk = device(disk.bytes);
        let partition = hadris_block::partition::r#async::gpt_partition(&mut disk, entry).unwrap();
        let volume = OpenVolume::open(partition).await.unwrap();
        assert_eq!(volume.format(), FatVariant::Fat12);
        let mut fs = volume.into_fat().ok().unwrap();
        assert!(fs.read_dir("/").await.unwrap().next_entry().await.is_none());
    });
}

#[test]
fn async_partition_table_hybrid_write_open_roundtrip() {
    use hadris_block::part::r#async::scheme_io::PartitionTableWriteExt;
    use hadris_block::part::hybrid::HybridMbrBuilder;
    use hadris_block::part::{MbrPartitionType, PartitionSchemeType, PartitionTable};

    let PartitionTable::Gpt { gpt, .. } = populated_gpt() else {
        unreachable!();
    };
    let hybrid_mbr = HybridMbrBuilder::new(8192)
        .protective_slot(3)
        .mirror_partition(0, MbrPartitionType::EfiSystemPartition, true)
        .build(&gpt.entries)
        .unwrap();
    let scheme = PartitionTable::Hybrid { hybrid_mbr, gpt };

    block_on(async {
        let mut disk = AsyncCursor::new(vec![0_u8; 8192 * 512]);
        scheme.write_to(&mut disk).await.unwrap();
        let opened = hadris_block::part::r#async::partition_table::open(&mut disk, 512)
            .await
            .unwrap();
        assert_eq!(opened.scheme_type(), PartitionSchemeType::Hybrid);
        opened.validate().unwrap();
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
        assert!(matches!(
            OpenVolume::open(dev).await,
            Err(hadris_block::Error::UnknownFormat)
        ));
    });
}
