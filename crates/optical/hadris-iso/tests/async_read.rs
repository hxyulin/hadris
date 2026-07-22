#![cfg(all(
    feature = "std",
    feature = "sync",
    feature = "async",
    feature = "write"
))]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_iso::r#async::read::IsoImage;
use hadris_iso::sync::read::PathSeparator;
use hadris_iso::sync::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::sync::write::{InputEntry, InputTree, IsoImageWriter};

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
fn async_leaf_reads_descriptors_traverses_and_reads_file() {
    let payload = b"direct async ISO leaf coverage";
    let files = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![InputEntry::directory(
            "DOCS",
            vec![InputEntry::file("README.TXT", payload)],
        )],
    );
    let options = IsoFormatOptions {
        volume_name: "ASYNC_TEST".to_owned(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::default(),
        strict_charset: false,
    };
    let mut image = std::io::Cursor::new(vec![0_u8; 2 * 1024 * 1024]);
    IsoImageWriter::create(&mut image, files, options).unwrap();
    let bytes = image.into_inner();

    block_on(async {
        let image = IsoImage::open(hadris_io::Cursor::new(bytes.as_slice()))
            .await
            .unwrap();
        assert_eq!(
            image
                .read_pvd()
                .await
                .unwrap()
                .volume_identifier
                .try_to_str()
                .unwrap(),
            "ASYNC_TEST"
        );

        let mut descriptors = image.read_volume_descriptors();
        assert!(descriptors.next_descriptor().await.unwrap().is_some());

        let entry = image.find_path("/DOCS//README.TXT").await.unwrap().unwrap();
        assert_eq!(image.read_file(&entry).await.unwrap(), payload);

        let root = image.open_dir(image.root_dir().dir_ref());
        assert!(
            root.read_entries()
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.is_directory())
        );
    });
}
