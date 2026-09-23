#![allow(dead_code, unused_macros)]

use std::fmt;

/// A device error, as a kernel driver would define it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    Timeout { lba: u64 },
}

impl fmt::Display for MemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { lba } => write!(f, "timeout at LBA {lba}"),
        }
    }
}

impl core::error::Error for MemError {}

/// A tree used by several tests: files, a nested directory and symlinks.
pub fn fixture<F>(
    new: impl FnOnce() -> F,
    add: impl Fn(&mut F, &str, &str, hadris_fs::NewNode<'_>, &[u8]),
) -> F {
    use hadris_fs::NewNode;
    let mut fs = new();
    add(&mut fs, "/", "etc", NewNode::Dir, b"");
    add(&mut fs, "/etc", "conf", NewNode::File, b"key=value");
    add(&mut fs, "/etc", "up", NewNode::Symlink(b"../etc"), b"");
    add(&mut fs, "/", "link", NewNode::Symlink(b"etc"), b"");
    add(&mut fs, "/", "abs", NewNode::Symlink(b"/etc/conf"), b"");
    add(&mut fs, "/", "loop", NewNode::Symlink(b"loop"), b"");
    add(
        &mut fs,
        "/",
        "long",
        NewNode::Symlink(b"etc/././././././././././././conf"),
        b"",
    );
    add(&mut fs, "/", "a.txt", NewNode::File, b"root a");
    fs
}

pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

#[path = "mode_sync.rs"]
pub mod sync;

#[path = "mode_asynch.rs"]
pub mod asynch;

#[path = "mode_send.rs"]
pub mod send;
