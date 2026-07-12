#![cfg(feature = "async")]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_block::r#async::OpenVolume;
use hadris_block::detect::FatVariant;
use hadris_io::SeekFrom;
use hadris_io::r#async::{Read, Seek};
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
