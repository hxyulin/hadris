#![cfg(all(
    feature = "std",
    feature = "alloc",
    feature = "write",
    feature = "async"
))]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_cpio::r#async::{
    CpioArchiveWriter, CpioReader, CpioWriteOptions, FileNode, FileTree, Write,
};

#[derive(Default)]
struct AsyncVec(Vec<u8>);

impl Write for AsyncVec {
    type Error = hadris_io::ErrorKind;

    async fn write(&mut self, bytes: &[u8]) -> hadris_io::Result<usize, Self::Error> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> hadris_io::Result<(), Self::Error> {
        Ok(())
    }
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

#[test]
fn async_finish_recovers_target_and_roundtrips() {
    block_on(async {
        let mut tree = FileTree::new();
        tree.add(FileNode::file("hello", b"async cpio".to_vec(), 0o644));

        let target = CpioArchiveWriter::new(AsyncVec::default(), CpioWriteOptions::default())
            .finish(&tree)
            .await
            .unwrap();
        assert!(!target.0.is_empty());

        let mut reader = CpioReader::new(hadris_io::Cursor::new(target.0.as_slice()));
        let entry = reader.next_entry_alloc().await.unwrap().unwrap();
        assert_eq!(entry.name_str().unwrap(), "hello");
        assert_eq!(
            reader.read_entry_data_alloc(&entry).await.unwrap(),
            b"async cpio"
        );
    });
}
