#![cfg(feature = "async")]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_block::r#async::OpenVolume;
use hadris_block::detect::FatVariant;
use hadris_io::SeekFrom;
use hadris_io::r#async::{Read, Seek, Write};
use hadris_storage::PartitionView;

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
    use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
    let mut image = vec![0_u8; 2 * 1024 * 1024];
    let options = FatFormatOptions::new(image.len() as u64).fat_type(FatTypeSelection::Fat12);
    let volume = FatVolumeFormatter::format(std::io::Cursor::new(&mut image[..]), options).unwrap();
    drop(volume);
    image
}

fn populated_gpt() -> hadris_block::part::DiskPartitionScheme {
    use hadris_block::part::{DiskPartitionScheme, GptPartitionEntry, Guid};

    let mut scheme = DiskPartitionScheme::new_gpt(8192, 512);
    let DiskPartitionScheme::Gpt { gpt, .. } = &mut scheme else {
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
    type Error = hadris_io::ErrorKind;

    async fn read(&mut self, buffer: &mut [u8]) -> hadris_io::Result<usize, Self::Error> {
        let start = usize::try_from(self.position)
            .map_err(|_| hadris_io::Error::from_kind(hadris_io::ErrorKind::InvalidInput))?;
        let available = self.bytes.len().saturating_sub(start);
        let len = available.min(buffer.len());
        buffer[..len].copy_from_slice(&self.bytes[start..start + len]);
        self.position += len as u64;
        Ok(len)
    }
}

impl Write for AsyncCursor {
    type Error = hadris_io::ErrorKind;

    async fn write(&mut self, buffer: &[u8]) -> hadris_io::Result<usize, Self::Error> {
        let start = usize::try_from(self.position)
            .map_err(|_| hadris_io::Error::from_kind(hadris_io::ErrorKind::InvalidInput))?;
        let end = start
            .checked_add(buffer.len())
            .ok_or_else(|| hadris_io::Error::from_kind(hadris_io::ErrorKind::InvalidInput))?;
        if end > self.bytes.len() {
            return Err(hadris_io::Error::from_kind(hadris_io::ErrorKind::WriteZero));
        }
        self.bytes[start..end].copy_from_slice(buffer);
        self.position = end as u64;
        Ok(buffer.len())
    }

    async fn flush(&mut self) -> hadris_io::Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for AsyncCursor {
    type Error = hadris_io::ErrorKind;

    async fn seek(&mut self, position: SeekFrom) -> hadris_io::Result<u64, Self::Error> {
        let next = match position {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => self.bytes.len() as i128 + i128::from(offset),
        };
        if !(0..=self.bytes.len() as i128).contains(&next) {
            return Err(hadris_io::Error::from_kind(
                hadris_io::ErrorKind::InvalidInput,
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
fn async_detection_and_open_restore_and_release_source() {
    let image = formatted_fat12();
    block_on(async {
        let mut source = hadris_io::Cursor::new(&image);
        source.seek(SeekFrom::Start(23)).await.unwrap();
        let detected = hadris_block::detect::r#async::detect(&mut source, 512)
            .await
            .unwrap();
        assert_eq!(
            detected,
            Some(hadris_block::detect::BlockFormat::Fat(FatVariant::Fat12))
        );
        assert_eq!(source.stream_position().await.unwrap(), 23);

        let volume = OpenVolume::open(&mut source, 512).await.unwrap();
        assert_eq!(volume.format(), FatVariant::Fat12);
        assert!(volume.as_fat().is_some());
        let source = volume.into_inner();
        assert!(source.position() > 0);
    });
}

#[test]
fn async_open_reports_mismatch_without_consuming_source() {
    let image = formatted_fat12();
    block_on(async {
        let mut source = hadris_io::Cursor::new(&image);
        assert!(matches!(
            OpenVolume::open_detected(&mut source, FatVariant::Fat16).await,
            Err(hadris_block::Error::DetectedFormatMismatch { .. })
        ));
        source.seek(SeekFrom::Start(11)).await.unwrap();
        assert_eq!(source.stream_position().await.unwrap(), 11);
    });
}

#[test]
fn async_fat_content_mutation_traversal_and_recovery() {
    use hadris_fat::r#async::FatVolumeWriteExt;

    let image = formatted_fat12();
    block_on(async {
        let mut source = AsyncCursor::new(image);
        let volume = OpenVolume::open(&mut source, 512).await.unwrap();
        let fs = volume.as_fat().unwrap();
        let root = fs.root_dir();
        let nested = fs.create_dir(&root, "NESTED").await.unwrap();
        let entry = fs.create_file(&nested, "PAYLOAD.BIN").await.unwrap();

        let payload: Vec<u8> = (0..1537).map(|index| (index % 251) as u8).collect();
        let mut writer = fs.write_file(&entry).unwrap();
        assert_eq!(writer.write(&payload).await.unwrap(), payload.len());
        writer.finish().await.unwrap();

        let mut reader = fs.open_file_path("NESTED/PAYLOAD.BIN").await.unwrap();
        assert_eq!(reader.read_to_vec().await.unwrap(), payload);

        let entry = fs.open_path("NESTED/PAYLOAD.BIN").await.unwrap();
        fs.truncate(&entry, 513).await.unwrap();
        let mut reader = fs.open_file_path("NESTED/PAYLOAD.BIN").await.unwrap();
        assert_eq!(reader.read_to_vec().await.unwrap(), payload[..513]);

        assert!(fs.create_file(&nested, "PAYLOAD.BIN").await.is_err());
        assert!(fs.open_dir_path("NESTED").await.is_ok());

        let source = volume.into_inner();
        assert_eq!(source.bytes.len(), 2 * 1024 * 1024);
        source.seek(SeekFrom::Start(0)).await.unwrap();
        assert_eq!(source.stream_position().await.unwrap(), 0);
    });
}

#[test]
fn async_partition_table_gpt_write_detect_open_and_reject_malformed() {
    use hadris_block::part::PartitionSchemeType;
    use hadris_block::part::r#async::scheme_io::DiskPartitionSchemeWriteExt;

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
            Err(hadris_block::part::PartitionError::Io(error))
                if error.kind() == hadris_io::ErrorKind::UnexpectedEof
        ));

        let mut corrupt = disk.bytes;
        corrupt[512..520].copy_from_slice(b"NOT GPT!");
        assert!(matches!(
            hadris_block::part::r#async::partition_table::open(
                &mut AsyncCursor::new(corrupt),
                512,
            ).await,
            Err(hadris_block::part::PartitionError::InvalidGptSignature { .. })
        ));
    });
}

#[test]
fn async_partition_table_opens_fat_through_a_gpt_view() {
    use hadris_block::part::r#async::scheme_io::DiskPartitionSchemeWriteExt;

    let mut bytes = vec![0_u8; 8192 * 512];
    let start = 40 * 512;
    let end = start + 4096 * 512;
    let options = hadris_fat::format::FatFormatOptions::new((end - start) as u64)
        .fat_type(hadris_fat::format::FatTypeSelection::Fat12);
    drop(
        hadris_fat::format::FatVolumeFormatter::format(
            std::io::Cursor::new(&mut bytes[start..end]),
            options,
        )
        .unwrap(),
    );

    block_on(async {
        let mut disk = AsyncCursor::new(bytes);
        populated_gpt().write_to(&mut disk).await.unwrap();
        let table = hadris_block::part::r#async::partition_table::open(&mut disk, 512)
            .await
            .unwrap();
        let entry = match &table {
            hadris_block::part::DiskPartitionScheme::Gpt { gpt, .. } => &gpt.entries[0],
            _ => unreachable!(),
        };
        let mut partition =
            hadris_block::partition::gpt_partition_view(&mut disk, entry, 512).unwrap();
        let volume = OpenVolume::open(&mut partition, 512).await.unwrap();
        assert_eq!(volume.format(), FatVariant::Fat12);
        let _root = volume.as_fat().unwrap().root_dir();
    });
}

#[test]
fn async_partition_table_hybrid_write_open_roundtrip() {
    use hadris_block::part::r#async::scheme_io::DiskPartitionSchemeWriteExt;
    use hadris_block::part::hybrid::HybridMbrBuilder;
    use hadris_block::part::{DiskPartitionScheme, MbrPartitionType, PartitionSchemeType};

    let DiskPartitionScheme::Gpt { gpt, .. } = populated_gpt() else {
        unreachable!();
    };
    let hybrid_mbr = HybridMbrBuilder::new(8192)
        .protective_slot(3)
        .mirror_partition(0, MbrPartitionType::EfiSystemPartition, true)
        .build(&gpt.entries)
        .unwrap();
    let scheme = DiskPartitionScheme::Hybrid { hybrid_mbr, gpt };

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
fn async_unknown_block_input_is_non_destructive_and_category_typed() {
    block_on(async {
        let mut source = hadris_io::Cursor::new(&[0xA5_u8; 4096]);
        source.seek(SeekFrom::Start(37)).await.unwrap();
        assert_eq!(
            hadris_block::detect::r#async::detect(&mut source, 512)
                .await
                .unwrap(),
            None
        );
        assert_eq!(source.stream_position().await.unwrap(), 37);
        assert!(matches!(
            OpenVolume::open(&mut source, 512).await,
            Err(hadris_block::Error::UnknownFormat)
        ));
    });
}
