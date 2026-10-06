use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use hadris::fs::{Content, MountOptions, Node, Tree};
use hadris::storage::{BlockIndex, BlockSize, MemDevice};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

#[derive(Default)]
struct Capture {
    next_id: AtomicU64,
    entered: AtomicUsize,
    spans: Mutex<Vec<Span>>,
}

#[derive(Debug)]
struct Span {
    name: &'static str,
    module: &'static str,
    target: &'static str,
    has_source: bool,
    fields: Vec<(String, String)>,
}

struct Fields(Vec<(String, String)>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }
}

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target().starts_with("hadris::")
    }
    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let meta = attrs.metadata();
        let mut fields = Fields(Vec::new());
        attrs.record(&mut fields);
        self.spans.lock().unwrap().push(Span {
            name: meta.name(),
            target: meta.target(),
            module: meta.module_path().unwrap_or_default(),
            has_source: meta.file().is_some() && meta.line().is_some(),
            fields: fields.0,
        });
        Id::from_u64(self.next_id.fetch_add(1, Ordering::Relaxed) + 1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, _: &Event<'_>) {}
    fn enter(&self, _: &Id) {
        self.entered.fetch_add(1, Ordering::Relaxed);
    }
    fn exit(&self, _: &Id) {
        self.entered.fetch_sub(1, Ordering::Relaxed);
    }
}

fn assert_span(capture: &Capture, name: &str, module: &str, field: Option<(&str, &str)>) {
    let spans = capture.spans.lock().unwrap();
    assert!(
        spans.iter().any(|span| {
            span.name == name
                && span.module.contains(module)
                && span.has_source
                && field.is_none_or(|(key, value)| {
                    span.fields.iter().any(|(k, v)| k == key && v == value)
                })
        }),
        "missing {module}::{name} span: {spans:?}"
    );
    assert!(spans.iter().all(|span| {
        span.fields.iter().all(|(name, _)| {
            ![
                "buf",
                "data",
                "dev",
                "self",
                "file",
                "password",
                "crypto_user",
            ]
            .contains(&name.as_str())
        })
    }));
}

fn device(bytes: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(bytes, BlockSize::new(2048).unwrap())
}

fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("sample.txt", Node::file(Content::bytes("private payload")))
        .unwrap();
    tree
}

#[test]
fn umbrella_tracing_reaches_every_format_without_recording_secrets() {
    use hadris::fs::sync::FileSystem;
    let capture = Arc::new(Capture::default());
    tracing::subscriber::with_default(capture.clone(), || {
        let mut iso = device(vec![0; 2 * 1024 * 1024]);
        hadris::iso::sync::write(&mut iso, &tree(), &hadris::iso::IsoOptions::new()).unwrap();
        let mut fs = hadris::iso::sync::IsoFs::mount(iso, MountOptions::new()).unwrap();
        let node = fs
            .lookup(fs.root(), hadris::fs::Name::new("sample.txt"))
            .unwrap();
        assert_eq!(fs.read(node, 0, &mut [0; 64]).unwrap(), 15);
        let mut udf = device(vec![0; 2 * 1024 * 1024]);
        hadris::udf::sync::write(&mut udf, &tree(), &hadris::udf::UdfOptions::new()).unwrap();
        let mut fs = hadris::udf::sync::UdfFs::mount(udf, MountOptions::new()).unwrap();
        fs.stat(fs.root()).unwrap();
        let mut out = hadris::io::StdIo::new(Vec::new());
        hadris::cpio::sync::write(&mut out, &tree(), &hadris::cpio::CpioOptions::new()).unwrap();
        let archive = out.into_inner();
        let mut reader = hadris::cpio::sync::CpioReader::new(hadris::io::Cursor::new(&archive));
        assert!(reader.next_entry().unwrap().is_some());
        assert!(hadris::part::sync::read(&mut device(vec![0; 32768])).is_err());
        assert!(
            hadris::ntfs::sync::NtfsFs::mount(device(vec![0; 32768]), MountOptions::new()).is_err()
        );
        assert!(
            hadris::apfs::sync::ApfsFs::mount_with_password(
                device(vec![0; 32768]),
                MountOptions::new(),
                b"private password",
                None
            )
            .is_err()
        );
    });
    for format in ["iso", "udf", "cpio", "part", "ntfs", "apfs"] {
        let spans = capture.spans.lock().unwrap();
        assert!(
            spans
                .iter()
                .any(|s| s.target == format!("hadris::{format}")),
            "missing {format}: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .all(|s| s.has_source && !format!("{:?}", s.fields).contains("private"))
        );
    }
    assert_span(&capture, "read", "sync::image", Some(("bytes", "64")));
    assert_span(&capture, "mount_with_password", "sync::fs", None);
    assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
}

struct PendingDevice;

impl hadris::io::ErrorType for PendingDevice {
    type Error = core::convert::Infallible;
}

impl hadris::storage::r#async::BlockDevice for PendingDevice {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(2048).unwrap()
    }
    fn block_count(&self) -> u64 {
        1024
    }
    async fn read_blocks(
        &mut self,
        _: BlockIndex,
        _: &mut [u8],
    ) -> Result<(), hadris::Error<Self::Error>> {
        core::future::pending().await
    }
}

#[test]
fn async_spans_leave_the_subscriber_between_polls_and_on_cancellation() {
    let capture = Arc::new(Capture::default());
    tracing::subscriber::with_default(capture.clone(), || {
        let mut future = Box::pin(hadris::iso::r#async::IsoFs::mount(
            PendingDevice,
            MountOptions::new(),
        ));
        fn assert_send<T: Send>(_: &T) {}
        assert_send(&future);
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..2 {
            assert!(matches!(future.as_mut().poll(&mut cx), Poll::Pending));
            assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
        }
        drop(future);
        assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
    });
    assert_span(&capture, "mount", "async::image", None);
}
