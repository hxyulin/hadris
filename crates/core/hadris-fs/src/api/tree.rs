use super::*;
use crate::PathError;
#[cfg(feature = "async")]
use crate::tree::AsyncSource;
#[cfg(feature = "sync")]
use crate::tree::SyncSource;
use crate::tree::{Content, Repr};
use alloc::sync::Arc;

fn stored() -> PathError {
    PathError::new(
        ErrorKind::Unsupported,
        "content stored on the output device",
    )
}

io_transform! {

sync_only! {
    const SYNC: bool = true;
}
async_only! {
    const SYNC: bool = false;
}

/// Lazy content of the sync mode when `sync`, else of the async mode, is
/// readable only by writers of that mode.
fn same_mode(sync: bool) -> Result<(), PathError> {
    if sync == SYNC {
        Ok(())
    } else {
        Err(PathError::new(
            ErrorKind::Unsupported,
            "lazy content is read only by writers of the mode that produced it",
        ))
    }
}

sync_only! {
    #[cfg(feature = "sync")]
    fn read_sync(source: &Arc<dyn SyncSource>, offset: u64, buf: &mut [u8]) -> Result<usize, PathError> {
        source.read_at(offset, buf)
    }
    #[cfg(feature = "async")]
    fn read_async(_: &Arc<dyn AsyncSource>, _: u64, _: &mut [u8]) -> Result<usize, PathError> {
        same_mode(false).map(|()| 0)
    }
}
async_only! {
    #[cfg(feature = "sync")]
    fn read_sync(_: &Arc<dyn SyncSource>, _: u64, _: &mut [u8]) -> Result<usize, PathError> {
        same_mode(true).map(|()| 0)
    }
    #[cfg(feature = "async")]
    async fn read_async(source: &Arc<dyn AsyncSource>, offset: u64, buf: &mut [u8]) -> Result<usize, PathError> {
        source.read_at(offset, buf).await
    }
}

/// Reads the [`Content`] of a tree file for a writer of this mode.
///
/// Bytes are read from memory. Host files and nodes of a sync `Volume`
/// are read by the sync writers, nodes of an async `Volume` by the async
/// ones; the other mode fails with [`ErrorKind::Unsupported`]. Content made
/// with [`Content::stored`] fails with [`ErrorKind::Unsupported`], since it
/// lives on the device a session writer updates. A host file whose length
/// changed since it was added fails with [`ErrorKind::Corrupt`] naming it.
pub struct ContentReader<'a> {
    content: &'a Content,
    #[cfg(all(feature = "std", feature = "sync"))]
    file: Option<std::fs::File>,
}

impl<'a> ContentReader<'a> {
    /// Checks without I/O that [`open`](Self::open) can read `content` in
    /// this mode: stored content and the other mode's lazy content fail
    /// with [`ErrorKind::Unsupported`]. Writers call it for every file
    /// before they write anything.
    pub fn check(content: &Content) -> Result<(), PathError> {
        match &content.repr {
            Repr::Bytes(_) => Ok(()),
            Repr::Stored(_) => Err(stored()),
            #[cfg(all(feature = "std", feature = "sync"))]
            Repr::Host(_) => same_mode(true),
            #[cfg(feature = "sync")]
            Repr::Sync(_) => same_mode(true),
            #[cfg(feature = "async")]
            Repr::Async(_) => same_mode(false),
        }
    }

    /// Opens `content`: a host file is opened now.
    pub async fn open(content: &'a Content) -> Result<Self, PathError> {
        Self::check(content)?;
        #[cfg(all(feature = "std", feature = "sync"))]
        let mut file = None;
        #[cfg(all(feature = "std", feature = "sync"))]
        if let Repr::Host(path) = &content.repr {
            sync_only! {
                file = Some(open_host(path, content.len())?);
            }
            async_only! {
                let _ = (path, &mut file);
            }
        }
        Ok(Self {
            content,
            #[cfg(all(feature = "std", feature = "sync"))]
            file,
        })
    }

    /// The length in bytes.
    pub fn len(&self) -> u64 {
        self.content.len()
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Reads from `offset`. Returns 0 at or past the end.
    pub async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, PathError> {
        let len = self.content.len();
        if offset >= len || buf.is_empty() {
            return Ok(0);
        }
        let limit = usize::try_from(len - offset).unwrap_or(usize::MAX).min(buf.len());
        let buf = &mut buf[..limit];
        match &self.content.repr {
            Repr::Bytes(bytes) => {
                let start = offset as usize;
                buf.copy_from_slice(&bytes[start..start + limit]);
                Ok(limit)
            }
            #[cfg(all(feature = "std", feature = "sync"))]
            Repr::Host(path) => {
                use std::io::{Read as _, Seek as _};
                same_mode(true)?;
                let file = self.file.as_mut().ok_or_else(|| PathError::new(ErrorKind::InvalidHandle, "the host file is not open"))?;
                file.seek(std::io::SeekFrom::Start(offset))
                    .and_then(|_| file.read(buf))
                    .map_err(|err| host_error(err, path))
            }
            #[cfg(feature = "sync")]
            Repr::Sync(source) => read_sync(source, offset, buf),
            #[cfg(feature = "async")]
            Repr::Async(source) => read_async(source, offset, buf).await,
            Repr::Stored(_) => Err(stored()),
        }
    }

    /// Fills `buf` from `offset`, failing with [`ErrorKind::Corrupt`] when
    /// the content ends first.
    pub async fn read_exact_at(&mut self, mut offset: u64, mut buf: &mut [u8]) -> Result<(), PathError> {
        while !buf.is_empty() {
            match self.read_at(offset, buf).await? {
                0 => return Err(PathError::new(ErrorKind::Corrupt, "file content ended early")),
                n => {
                    buf = &mut buf[n..];
                    offset += n as u64;
                }
            }
        }
        Ok(())
    }
}

impl core::fmt::Debug for ContentReader<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContentReader").field("content", self.content).finish_non_exhaustive()
    }
}

}

#[cfg(all(feature = "std", feature = "sync"))]
fn host_error(err: std::io::Error, path: &std::path::Path) -> PathError {
    PathError::from(crate::Error::device(err, "reading a host file failed")).with_host_path(path)
}

/// Opens the host file of a [`Content`] and checks that its length is still
/// `len`.
#[cfg(all(feature = "std", feature = "sync"))]
#[allow(dead_code)]
fn open_host(path: &std::path::Path, len: u64) -> Result<std::fs::File, PathError> {
    let file = std::fs::File::open(path).map_err(|err| host_error(err, path))?;
    let now = hadris_storage::host::file_len(&file).map_err(|err| host_error(err, path))?;
    if now != len {
        return Err(PathError::new(
            ErrorKind::Corrupt,
            "host file changed length after it was added",
        )
        .with_host_path(path));
    }
    Ok(file)
}
