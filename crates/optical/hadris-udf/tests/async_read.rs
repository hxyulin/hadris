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

use hadris_udf::r#async::UdfVolume;
use hadris_udf::sync::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

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
fn async_leaf_opens_traverses_and_reads_nested_file() {
    let payload = b"direct async UDF leaf coverage";
    let mut root = SimpleDir::root();
    let mut docs = SimpleDir::new("DOCS");
    docs.add_file(SimpleFile::new("README.TXT", payload.to_vec()));
    root.add_dir(docs);

    let image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    let output = UdfWriter::create(image, &root, UdfWriteOptions::default()).unwrap();
    let bytes = output.target.into_inner();

    block_on(async {
        let volume = UdfVolume::open(hadris_io::Cursor::new(bytes.as_slice()))
            .await
            .unwrap();
        assert!(!volume.info().volume_id.is_empty());

        let root = volume.root_dir().await.unwrap();
        let docs = root.find("DOCS").unwrap();
        let nested = volume.read_directory(&docs.icb).await.unwrap();
        let readme = nested.find("README.TXT").unwrap();
        assert_eq!(volume.read_file(readme).await.unwrap(), payload);
        assert!(volume.read_file(docs).await.is_err());
    });
}
