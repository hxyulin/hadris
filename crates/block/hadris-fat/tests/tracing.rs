#[path = "common/cancel.rs"]
mod cancel;
#[path = "common/fatfs.rs"]
mod common;

use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use hadris_fat::embedded::{MountToken, Options};
use hadris_fs::{MountOptions, Name, OpenOptions, SetAttr};
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
        span.fields
            .iter()
            .all(|(name, _)| !["buf", "data", "dev", "self", "file"].contains(&name.as_str()))
    }));
}

#[test]
fn sync_spans_include_function_metadata_and_sizes_without_buffer_contents() {
    use hadris_fs::sync::FileSystem;
    let case = common::CASES[0];
    let image = common::blank(case);
    let capture = Arc::new(Capture::default());
    tracing::subscriber::with_default(capture.clone(), || {
        let mut fs = hadris_fat::sync::FatFs::mount(
            common::device(case, image.clone()),
            MountOptions::new(),
        )
        .unwrap();
        let root = fs.root();
        let file = fs
            .create(root, Name::new("DATA.BIN"), &SetAttr::new())
            .unwrap();
        fs.write(file, 0, &[7; 1024]).unwrap();
        fs.sync().unwrap();
        fs.forget(file, 1);
        let image = fs.unmount().unwrap().into_inner();
        let mut token = MountToken::new();
        let mut fs = hadris_fat::embedded::sync::Fat::<_, 4>::mount_with(
            common::device(case, image),
            &mut token,
            Options::new(),
        )
        .unwrap();
        let file = fs
            .open(fs.root(), "DATA.BIN", OpenOptions::new().read())
            .unwrap();
        assert_eq!(fs.read(&file, &mut [0; 512]).unwrap(), 512);
        fs.close(file).unwrap();
    });
    assert_span(&capture, "mount", "sync::fatfs", None);
    assert_span(&capture, "write", "sync::fatfs", Some(("bytes", "1024")));
    assert_span(
        &capture,
        "allocate_chain",
        "sync::fatfs",
        Some(("count", "2")),
    );
    assert_span(&capture, "read", "embedded::sync", Some(("bytes", "512")));
    assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
}

#[test]
fn async_spans_exit_between_polls_and_when_a_pending_future_is_dropped() {
    use hadris_fs::r#async::FileSystem;
    let case = common::CASES[0];
    let dev = cancel::YieldDev(common::device(case, common::blank(case)));
    let capture = Arc::new(Capture::default());
    tracing::subscriber::with_default(capture.clone(), || {
        common::block_on(async {
            let mut fs = hadris_fat::r#async::FatFs::mount(dev, MountOptions::new())
                .await
                .unwrap();
            let file = fs
                .create(fs.root(), Name::new("DATA.BIN"), &SetAttr::new())
                .await
                .unwrap();
            {
                let mut pending = Box::pin(fs.write(file, 0, &[7; 1024]));
                fn assert_send<T: Send>(_: &T) {}
                assert_send(&pending);
                let mut cx = Context::from_waker(Waker::noop());
                assert!(matches!(pending.as_mut().poll(&mut cx), Poll::Pending));
                assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
            }
            assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
            fs.sync().await.unwrap();
            fs.forget(file, 1);
            let mut dev = fs.unmount().await.unwrap();
            let report = hadris_fat::r#async::check(&mut dev, &mut [0; 16384], |_| {})
                .await
                .unwrap();
            assert!(report.is_clean());
        });
    });
    assert_span(&capture, "write", "async::fatfs", Some(("bytes", "1024")));
    assert_eq!(capture.entered.load(Ordering::Relaxed), 0);
}
