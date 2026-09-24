use super::*;
use crate::PathError;
use crate::tree::{Content, Repr, Tree, Warning, WarningKind};
use alloc::string::String;
use alloc::vec::Vec;

io_transform! {

/// Reads a [`Content`] for a writer in this mode.
///
/// Bytes, sources and host files are read; content made with
/// [`Content::stored`] fails with [`ErrorKind::Unsupported`], since it lives
/// on the device the writer updates.
pub struct ContentReader<'a> {
    content: &'a Content,
    len: u64,
    #[cfg(feature = "std")]
    file: Option<std::fs::File>,
}

impl<'a> ContentReader<'a> {
    /// Opens `content`: a host file is opened and measured now.
    pub async fn open(content: &'a Content) -> Result<Self, PathError> {
        let len = match &content.0 {
            Repr::Bytes(bytes) => bytes.len() as u64,
            #[cfg(feature = "sync")]
            Repr::Blocking(source) => source.lock().len(),
            #[cfg(feature = "async-send")]
            Repr::Async(source) => async_content_len(source).await?,
            #[cfg(feature = "std")]
            Repr::Path { path, .. } => {
                let file = std::fs::File::open(path).map_err(|err| host_error(err, path))?;
                let len = hadris_storage::host::file_len(&file).map_err(|err| host_error(err, path))?;
                return Ok(Self { content, len, file: Some(file) });
            }
            Repr::Stored(_) => return Err(ErrorKind::Unsupported.into()),
        };
        Ok(Self {
            content,
            len,
            #[cfg(feature = "std")]
            file: None,
        })
    }

    /// The length in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reads from `offset`. Returns 0 at or past the end.
    pub async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, PathError> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let limit = usize::try_from(self.len - offset).unwrap_or(usize::MAX).min(buf.len());
        let buf = &mut buf[..limit];
        match &self.content.0 {
            Repr::Bytes(bytes) => {
                let start = offset as usize;
                buf.copy_from_slice(&bytes[start..start + limit]);
                Ok(limit)
            }
            #[cfg(feature = "sync")]
            Repr::Blocking(source) => source.lock().read_at(offset, buf),
            #[cfg(feature = "async-send")]
            Repr::Async(source) => async_content_read(source, offset, buf).await,
            #[cfg(feature = "std")]
            Repr::Path { path, .. } => {
                use std::io::{Read as _, Seek as _};
                let file = self.file.as_mut().ok_or(ErrorKind::InvalidInput)?;
                file.seek(std::io::SeekFrom::Start(offset))
                    .and_then(|_| file.read(buf))
                    .map_err(|err| host_error(err, path))
            }
            Repr::Stored(_) => Err(ErrorKind::Unsupported.into()),
        }
    }

    /// Fills `buf` from `offset`, failing with [`ErrorKind::Corrupt`] when
    /// the content ends first.
    pub async fn read_exact_at(&mut self, mut offset: u64, mut buf: &mut [u8]) -> Result<(), PathError> {
        while !buf.is_empty() {
            match self.read_at(offset, buf).await? {
                0 => return Err(ErrorKind::Corrupt.into()),
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
        f.debug_struct("ContentReader").field("len", &self.len).finish_non_exhaustive()
    }
}

/// Builds a [`Tree`] from a mounted filesystem, in this mode.
///
/// ```rust,ignore
/// use hadris_fs::sync::TreeExt;
/// let tree = Tree::from_filesystem(&mut fat)?;
/// ```
pub trait TreeExt: Sized {
    /// Reads everything below the root of `src` into a tree: file contents
    /// into memory, symlinks with their targets, and metadata where `src`
    /// reports it. A node with more than one link that `src` lists under
    /// the same id again becomes a hard link.
    ///
    /// Device nodes, FIFOs and sockets are left out and listed in
    /// [`Tree::warnings`], since `Metadata` carries no device number. A
    /// directory entry that leads back to a directory on its own path fails
    /// with [`ErrorKind::Corrupt`], and a tree more than 1024 directories
    /// deep with [`ErrorKind::LimitExceeded`].
    async fn from_filesystem<A: Access + io::MaybeSend>(src: A) -> FsResult<Self, A::DeviceError>;
}

impl TreeExt for Tree {
    async fn from_filesystem<A: Access + io::MaybeSend>(src: A) -> FsResult<Self, A::DeviceError> {
        let mut fs = src.into_driver();
        let mut tree = Tree::new();
        let root = fs.root();
        let meta = fs.node_metadata(root).await?;
        tree.set_metadata("/", set_metadata_of(&meta))?;
        let mut stack: Vec<(NodeId, String, DirCursor)> = alloc::vec![(root, String::new(), DirCursor::start())];
        let mut seen: Vec<(NodeId, String)> = Vec::new();
        let result = import(&mut fs, &mut tree, &mut stack, &mut seen).await;
        for (node, _, _) in stack.iter().skip(1) {
            fs.forget(*node);
        }
        result.map(|()| tree)
    }
}

async fn import<D: FsDriver + ?Sized>(
    fs: &mut D,
    tree: &mut Tree,
    stack: &mut Vec<(NodeId, String, DirCursor)>,
    seen: &mut Vec<(NodeId, String)>,
) -> FsResult<(), D::DeviceError> {
    let mut name = NameBuf::new();
    loop {
        let Some((dir, prefix, cursor)) = stack.last_mut() else {
            return Ok(());
        };
        let dir = *dir;
        if fs.read_dir_entry(dir, cursor, &mut name).await?.is_none() {
            if let Some((node, _, _)) = stack.pop()
                && node != fs.root()
            {
                fs.forget(node);
            }
            continue;
        }
        let child_name = name.as_name().ok_or(ErrorKind::Corrupt)?;
        let text = child_name.to_str().map_err(|_| ErrorKind::InvalidInput)?;
        let path = alloc::format!("{prefix}/{text}");
        let child = fs.lookup(dir, child_name).await?;
        match import_node(fs, tree, child, &path, seen).await {
            Ok(true) => {
                if let Err(err) = super::copy::enter(stack.iter().map(|(node, ..)| *node), child) {
                    fs.forget(child);
                    return Err(err.into());
                }
                stack.push((child, path, DirCursor::start()));
            }
            Ok(false) => fs.forget(child),
            Err(err) => {
                fs.forget(child);
                return Err(err);
            }
        }
    }
}

/// Adds the pinned `node` at `path`. Returns whether it is a directory to
/// walk.
async fn import_node<D: FsDriver + ?Sized>(
    fs: &mut D,
    tree: &mut Tree,
    node: NodeId,
    path: &str,
    seen: &mut Vec<(NodeId, String)>,
) -> FsResult<bool, D::DeviceError> {
    let meta = fs.node_metadata(node).await?;
    match meta.file_type() {
        FileType::Dir => tree.add_dir(path)?,
        FileType::File => {
            if meta.nlink() > 1 {
                if let Some((_, first)) = seen.iter().find(|(id, _)| *id == node) {
                    tree.add_hard_link(path, first)?;
                    return Ok(false);
                }
                seen.push((node, String::from(path)));
            }
            let mut data = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = fs.read_at(node, data.len() as u64, &mut chunk).await?;
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&chunk[..n]);
            }
            tree.add_file(path, Content::bytes(data))?;
        }
        FileType::Symlink => {
            let mut target = alloc::vec![0u8; 4096];
            let n = fs.read_link(node, &mut target).await?;
            tree.add_symlink(path, &target[..n])?;
        }
        _ => {
            tree.warn(Warning::new(path, WarningKind::Skipped, "device nodes, FIFOs and sockets are not imported"));
            return Ok(false);
        }
    }
    tree.set_metadata(path, set_metadata_of(&meta))?;
    Ok(meta.file_type().is_dir())
}

}

fn set_metadata_of(meta: &Metadata) -> SetMetadata {
    let set = SetMetadata::new()
        .with_times(meta.times())
        .with_mode(meta.permissions())
        .with_attributes(meta.attributes());
    match meta.owner() {
        Some((uid, gid)) => set.with_uid(uid).with_gid(gid),
        None => set,
    }
}

#[cfg(feature = "std")]
fn host_error(err: std::io::Error, path: &std::path::Path) -> PathError {
    PathError::from(Error::device(err, "reading a host file failed")).with_host_path(path)
}

async_only! {
    #[cfg(feature = "async-send")]
    async fn async_content_len(
        source: &async_lock::Mutex<alloc::boxed::Box<dyn crate::tree::AsyncSource>>,
    ) -> Result<u64, PathError> {
        Ok(source.lock().await.len())
    }

    #[cfg(feature = "async-send")]
    async fn async_content_read(
        source: &async_lock::Mutex<alloc::boxed::Box<dyn crate::tree::AsyncSource>>,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, PathError> {
        source.lock().await.read_at(offset, buf).await
    }
}

sync_only! {
    #[cfg(feature = "async-send")]
    fn async_content_len(
        _: &async_lock::Mutex<alloc::boxed::Box<dyn crate::tree::AsyncSource>>,
    ) -> Result<u64, PathError> {
        Err(ErrorKind::Unsupported.into())
    }

    #[cfg(feature = "async-send")]
    fn async_content_read(
        _: &async_lock::Mutex<alloc::boxed::Box<dyn crate::tree::AsyncSource>>,
        _: u64,
        _: &mut [u8],
    ) -> Result<usize, PathError> {
        Err(ErrorKind::Unsupported.into())
    }
}
