#![cfg(all(feature = "open", feature = "async", feature = "sync", feature = "cd"))]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

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

#[test]
fn asynchronously_opens_and_recovers_an_iso_source() {
    let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    hadris_optical::cd::CdWriter::new(
        hadris_io::sync::Borrowed::new(&mut image),
        hadris_optical::cd::CdOptions::default().iso_only(),
    )
    .finish(hadris_optical::cd::FileTree::new())
    .unwrap();
    let bytes = image.into_inner();

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Iso9660,
        )
        .await
        .unwrap();
        assert_eq!(opened.format(), hadris_optical::OpticalFormat::Iso9660);
        let _ = opened.into_inner();
    });
}

#[test]
fn asynchronously_opens_and_recovers_a_udf_source() {
    use hadris_optical::udf::sync::write::{SimpleDir, UdfWriteOptions, UdfWriter};

    let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    UdfWriter::format(
        hadris_io::sync::Borrowed::new(&mut image),
        &SimpleDir::root(),
        UdfWriteOptions::default(),
    )
    .unwrap();
    let bytes = image.into_inner();

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Udf,
        )
        .await
        .unwrap();
        assert_eq!(opened.format(), hadris_optical::OpticalFormat::Udf);
        assert!(opened.as_udf().is_some());
        let _ = opened.into_inner();
    });
}
