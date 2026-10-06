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
    add: impl Fn(&mut F, &str, &str, hadris_fs::FileType, &[u8]),
) -> F {
    use hadris_fs::FileType::{Dir, File, Symlink};
    let mut fs = new();
    add(&mut fs, "/", "etc", Dir, b"");
    add(&mut fs, "/etc", "conf", File, b"key=value");
    add(&mut fs, "/etc", "up", Symlink, b"../etc");
    add(&mut fs, "/", "link", Symlink, b"etc");
    add(&mut fs, "/", "abs", Symlink, b"/etc/conf");
    add(&mut fs, "/", "loop", Symlink, b"loop");
    add(
        &mut fs,
        "/",
        "long",
        Symlink,
        b"etc/././././././././././././conf",
    );
    add(&mut fs, "/", "a.txt", File, b"root a");
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

#[cfg(feature = "sync")]
#[path = "mode_sync.rs"]
pub mod sync;

#[cfg(feature = "async")]
#[path = "mode_async.rs"]
pub mod asynch;

/// Whole-file reads and writes through a sync `Volume`.
#[cfg(all(feature = "sync", feature = "std"))]
pub mod files {
    use hadris_fs::sync::{FileSystem, Volume};
    use hadris_fs::{FsResult, OpenOptions};

    pub fn read<F: FileSystem>(vol: &Volume<F>, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
        let mut file = vol.open(path, OpenOptions::new().read())?;
        let mut out = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match file.read(&mut chunk)? {
                0 => break,
                n => out.extend_from_slice(&chunk[..n]),
            }
        }
        file.close()?;
        Ok(out)
    }

    pub fn write<F: FileSystem>(
        vol: &Volume<F>,
        path: &str,
        data: &[u8],
    ) -> FsResult<(), F::DeviceError> {
        let mut file = vol.open(path, OpenOptions::new().write().create().truncate())?;
        let mut rest = data;
        while !rest.is_empty() {
            let n = file.write(rest)?;
            rest = &rest[n..];
        }
        file.close()
    }

    pub fn exists<F: FileSystem>(vol: &Volume<F>, path: &str) -> bool {
        vol.symlink_metadata(path).is_ok()
    }

    pub fn names<F: FileSystem>(vol: &Volume<F>, path: &str) -> Vec<String> {
        vol.read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().name().to_str().unwrap().to_owned())
            .collect()
    }
}
