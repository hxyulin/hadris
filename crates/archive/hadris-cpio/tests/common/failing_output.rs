use core::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hadris_cpio::FinishError;
use hadris_fs::{ErrorKind, PathError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Write,
    Flush,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Write => "injected write failure",
            Self::Flush => "injected flush failure",
        })
    }
}

impl core::error::Error for Fault {}

pub struct Output {
    pub bytes: Vec<u8>,
    pub drops: Arc<AtomicUsize>,
    pub flushes: usize,
    limit: Option<usize>,
    fail_flush: bool,
}

impl Output {
    pub fn new(limit: Option<usize>, fail_flush: bool) -> Self {
        Self {
            bytes: Vec::new(),
            drops: Arc::new(AtomicUsize::new(0)),
            flushes: 0,
            limit,
            fail_flush,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, Fault> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let remaining = self
            .limit
            .map_or(usize::MAX, |limit| limit.saturating_sub(self.bytes.len()));
        if remaining == 0 {
            return Err(Fault::Write);
        }
        let n = bytes.len().min(remaining).min(7);
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }

    fn flush_output(&mut self) -> Result<(), Fault> {
        self.flushes += 1;
        if self.fail_flush {
            Err(Fault::Flush)
        } else {
            Ok(())
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl hadris_io::ErrorType for Output {
    type Error = Fault;
}

impl hadris_io::sync::Write for Output {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, Fault> {
        self.write_bytes(bytes)
    }

    fn flush(&mut self) -> Result<(), Fault> {
        self.flush_output()
    }
}

#[cfg(feature = "async")]
async fn yield_once() {
    let mut yielded = false;
    core::future::poll_fn(|cx| {
        if yielded {
            core::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            core::task::Poll::Pending
        }
    })
    .await
}

#[cfg(feature = "async")]
impl hadris_io::async_::Write for Output {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, Fault> {
        yield_once().await;
        self.write_bytes(bytes)
    }

    async fn flush(&mut self) -> Result<(), Fault> {
        yield_once().await;
        self.flush_output()
    }
}

pub fn assert_cause(failure: &FinishError<Output>, cause: Fault, path: Option<&[u8]>) {
    assert_eq!(failure.error().kind(), ErrorKind::Io);
    assert_eq!(failure.error().path(), path);
    assert_eq!(failure.error().downcast_device::<Fault>(), Some(&cause));
    let source = core::error::Error::source(failure)
        .unwrap()
        .downcast_ref::<PathError>()
        .unwrap();
    assert!(core::ptr::eq(source, failure.error()));
    assert_eq!(failure.to_string(), "finishing the cpio archive failed");
    assert!(format!("{failure:?}").contains("FinishError"));
}
