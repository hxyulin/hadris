use core::convert::Infallible;
use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_cpio::{CpioOptions, Format};
use hadris_fs::{Content, Node, Tree};
use hadris_io::{Cursor, StdIo};

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = core::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[derive(Default)]
struct Sink(Vec<u8>);

impl hadris_io::ErrorType for Sink {
    type Error = Infallible;
}

impl hadris_io::r#async::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> impl Future<Output = Result<usize, Infallible>> + Send {
        self.0.extend_from_slice(bytes);
        core::future::ready(Ok(bytes.len()))
    }

    fn flush(&mut self) -> impl Future<Output = Result<(), Infallible>> + Send {
        core::future::ready(Ok(()))
    }
}

fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("bin/busybox", Node::file(Content::bytes(vec![1u8; 5000])))
        .unwrap();
    tree.insert("bin/sh", Node::symlink("busybox")).unwrap();
    tree.link("bin/busybox", "linuxrc").unwrap();
    tree
}

#[test]
fn every_mode_writes_the_same_bytes() {
    let options = CpioOptions::default().with_format(Format::Crc);
    let mut sync = StdIo::new(Vec::new());
    hadris_cpio::sync::write(&mut sync, &tree(), &options).unwrap();
    let sync = sync.into_inner();

    let mut send = Sink::default();
    let tree = tree();
    let future = hadris_cpio::r#async::write(&mut send, &tree, &options);
    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    block_on(assert_send(future)).unwrap();
    assert_eq!(send.0, sync);

    let back = block_on(assert_send(hadris_cpio::r#async::read_tree(
        &mut hadris_cpio::r#async::CpioReader::new(Cursor::new(&sync)),
    )))
    .unwrap();
    assert_eq!(back.entry("linuxrc").unwrap().links(), 2);
}

#[test]
fn the_async_reader_reads_what_was_written() {
    let mut sync = StdIo::new(Vec::new());
    hadris_cpio::sync::write(&mut sync, &tree(), &CpioOptions::default()).unwrap();
    let bytes = sync.into_inner();
    block_on(async {
        use hadris_io::r#async::Read;
        let mut reader = hadris_cpio::r#async::CpioReader::new(Cursor::new(&bytes));
        let mut names = Vec::new();
        while let Some(mut entry) = reader.next_entry().await.unwrap() {
            let mut data = vec![0u8; entry.len() as usize];
            entry.read_exact(&mut data).await.unwrap();
            names.push((entry.name_str().unwrap().to_string(), data.len()));
        }
        assert_eq!(
            names,
            [
                ("bin".to_string(), 0),
                ("bin/busybox".to_string(), 0),
                ("bin/sh".to_string(), 7),
                ("linuxrc".to_string(), 5000)
            ]
        );
    });
}
