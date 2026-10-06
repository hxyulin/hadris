mod common;

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
        let n = bytes.len().min(7);
        self.0.extend_from_slice(&bytes[..n]);
        core::future::ready(Ok(n))
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
            names.push((entry.path_str().unwrap().to_string(), data.len()));
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

#[test]
fn custom_buffer_segments_and_offsets_are_send() {
    let mut output = StdIo::new(Vec::new());
    let mut tree = Tree::new();
    tree.insert("a", Node::file(Content::bytes(b"abc")))
        .unwrap();
    hadris_cpio::sync::write(&mut output, &tree, &CpioOptions::default()).unwrap();
    let first = output.into_inner();
    let mut bytes = first.clone();
    bytes.extend([0; 13]);
    bytes.extend(&first);
    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    block_on(assert_send(async {
        let mut buffer = [0; 32];
        let mut reader = hadris_cpio::r#async::CpioReader::with_buffer(
            Cursor::new(&bytes),
            &mut buffer[..],
            hadris_cpio::ReaderOptions::new(),
        );
        assert_eq!(reader.next_entry().await.unwrap().unwrap().offset(), 0);
        assert!(reader.next_entry().await.unwrap().is_none());
        assert!(reader.next_segment().await.unwrap());
        let entry = reader.next_entry().await.unwrap().unwrap();
        assert_eq!(entry.path_str().unwrap(), "a");
        assert_eq!(entry.offset(), first.len() as u64 + 13);
        assert!(reader.next_entry().await.unwrap().is_none());
        assert!(!reader.next_segment().await.unwrap());
    }));
}

#[test]
fn async_hard_link_owners_follow_equivalent_tree_paths() {
    let mut linked = common::newc_entry(b"dir//a", 0o100644, b"old", None);
    linked[38..46].copy_from_slice(b"00000002");
    let mut final_link = common::newc_entry(b"b", 0o100644, b"group", None);
    final_link[38..46].copy_from_slice(b"00000002");
    let bytes = [
        linked,
        common::newc_entry(b"dir/a/", 0o100644, b"replacement", None),
        final_link,
        common::trailer(),
    ]
    .concat();
    let tree = block_on(hadris_cpio::r#async::read_tree(
        &mut hadris_cpio::r#async::CpioReader::new(Cursor::new(&bytes)),
    ))
    .unwrap();
    assert_eq!(
        tree.get("dir/a").unwrap().content().unwrap().as_bytes(),
        Some(&b"replacement"[..])
    );
    assert_eq!(
        tree.get("b").unwrap().content().unwrap().as_bytes(),
        Some(&b"group"[..])
    );
    assert_eq!(tree.entry("dir/a").unwrap().links(), 1);
}

#[test]
fn async_borrowed_payloads_handle_short_writes() {
    let mut tree = Tree::new();
    tree.insert("data", Node::file(Content::bytes(vec![37; 131_073])))
        .unwrap();
    for format in [Format::Newc, Format::Crc, Format::Odc] {
        let expected = common::archive(&tree, format);
        let mut out = Sink::default();
        block_on(hadris_cpio::r#async::write(
            &mut out,
            &tree,
            &CpioOptions::new().with_format(format),
        ))
        .unwrap();
        assert_eq!(out.0, expected);
    }
}

#[test]
fn async_odc_hard_links_store_each_payload() {
    let options = CpioOptions::new().with_format(Format::Odc);
    let node = Node::file(Content::bytes(vec![37; 70_001]));
    let mut out = Sink::default();
    let mut writer = hadris_cpio::r#async::Writer::new(&mut out, &options);
    block_on(writer.append_hard_links(&["a", "b", "c"], &node)).unwrap();
    let (_, report) = block_on(writer.finish()).unwrap();
    for entry in common::read_all(&out.0).unwrap() {
        assert_eq!(entry.data, vec![37; 70_001]);
        assert_eq!(report.extents(entry.name).unwrap()[0].len(), 70_001);
    }
    assert_ne!(report.extents("a"), report.extents("b"));
}
