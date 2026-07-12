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
