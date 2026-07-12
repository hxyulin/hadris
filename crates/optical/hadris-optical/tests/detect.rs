#![cfg(feature = "detect")]

const SECTOR_SIZE: usize = 2048;

fn image_with(ids: &[(usize, &[u8; 5])]) -> Vec<u8> {
    let mut image = vec![0_u8; 32 * SECTOR_SIZE];
    for (sector, id) in ids {
        let offset = sector * SECTOR_SIZE;
        image[offset + 1..offset + 6].copy_from_slice(*id);
        image[offset + 6] = 1;
    }
    image
}

#[cfg(feature = "sync")]
#[test]
fn sync_probe_distinguishes_iso_udf_and_bridge_and_restores_position() {
    use hadris_optical::detect::{UdfVrs, sync::detect};
    use std::io::{Seek, SeekFrom};

    let cases = [
        (image_with(&[(16, b"CD001")]), true, None),
        (
            image_with(&[(16, b"BEA01"), (17, b"NSR02"), (18, b"TEA01")]),
            false,
            Some(UdfVrs::Nsr02),
        ),
        (
            image_with(&[
                (16, b"CD001"),
                (17, b"BEA01"),
                (18, b"NSR03"),
                (19, b"TEA01"),
            ]),
            true,
            Some(UdfVrs::Nsr03),
        ),
    ];

    for (image, iso, udf) in cases {
        let mut source = std::io::Cursor::new(image);
        source.seek(SeekFrom::Start(37)).unwrap();
        let formats = detect(&mut source).unwrap().unwrap();
        assert_eq!(formats.has_iso9660(), iso);
        assert_eq!(formats.udf(), udf);
        assert_eq!(formats.is_bridge(), iso && udf.is_some());
        assert_eq!(source.stream_position().unwrap(), 37);
    }
}

#[cfg(all(feature = "sync", feature = "cd"))]
#[test]
fn detects_images_created_by_optical_writer() {
    let cases = [
        (
            hadris_optical::cd::CdOptions::default().iso_only(),
            true,
            false,
        ),
        (
            hadris_optical::cd::CdOptions::default().udf_only(),
            false,
            true,
        ),
        (hadris_optical::cd::CdOptions::default(), true, true),
    ];
    for (options, iso, udf) in cases {
        let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
        hadris_optical::cd::CdWriter::new(hadris_io::sync::Borrowed::new(&mut image), options)
            .write(hadris_optical::cd::FileTree::new())
            .unwrap();

        let formats = hadris_optical::detect::sync::detect(&mut image)
            .unwrap()
            .unwrap();
        assert_eq!(formats.has_iso9660(), iso);
        assert_eq!(formats.udf().is_some(), udf);
        assert_eq!(formats.is_bridge(), iso && udf);
    }
}

#[cfg(feature = "async")]
mod asynchronous {
    use core::future::Future;
    use core::task::{Context, Poll};
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    use hadris_io::SeekFrom;
    use hadris_io::r#async::Seek;
    use hadris_optical::detect::{UdfVrs, r#async::detect};

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
    fn async_probe_reports_bridge_and_restores_position() {
        let image = super::image_with(&[
            (16, b"CD001"),
            (17, b"BEA01"),
            (18, b"NSR03"),
            (19, b"TEA01"),
        ]);
        block_on(async {
            let mut source = hadris_io::Cursor::new(&image);
            source.seek(SeekFrom::Start(41)).await.unwrap();
            let formats = detect(&mut source).await.unwrap().unwrap();
            assert!(formats.has_iso9660());
            assert_eq!(formats.udf(), Some(UdfVrs::Nsr03));
            assert!(formats.is_bridge());
            assert_eq!(source.stream_position().await.unwrap(), 41);
        });
    }
}
