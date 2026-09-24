use super::paths::{create_dir_all, resolve_parent, write_all_at};
use super::*;
use crate::{Field, PathError, Stored};
use alloc::vec::Vec;

/// Bytes moved per read.
pub(super) const CHUNK: usize = 4096;

/// The longest symlink target read, as Linux `PATH_MAX`.
pub(super) const MAX_LINK_TARGET: usize = 4096;

/// A buffer for the target of a symlink whose metadata reports `len` bytes.
/// [`ErrorKind::LimitExceeded`] above [`MAX_LINK_TARGET`].
pub(super) fn link_buffer(len: u64) -> Result<Vec<u8>, ErrorKind> {
    match usize::try_from(len) {
        Ok(0) => Ok(alloc::vec![0u8; MAX_LINK_TARGET]),
        Ok(len) if len <= MAX_LINK_TARGET => Ok(alloc::vec![0u8; len]),
        _ => Err(ErrorKind::LimitExceeded),
    }
}

/// Directories a tree walk descends before [`ErrorKind::LimitExceeded`].
pub(super) const MAX_TREE_DEPTH: usize = 1024;

/// Checks that a walk may enter the directory `child` below the directories
/// of `path`, the current path from the top: a directory already on it is a
/// cycle, which only a corrupt volume holds.
pub(super) fn enter(
    path: impl ExactSizeIterator<Item = NodeId>,
    child: NodeId,
) -> Result<(), ErrorKind> {
    if path.len() >= MAX_TREE_DEPTH {
        return Err(ErrorKind::LimitExceeded);
    }
    let mut path = path;
    if path.any(|node| node == child) {
        return Err(ErrorKind::Corrupt);
    }
    Ok(())
}

/// The fields of `meta` a filesystem with `caps` stores, as changes.
/// Permissions and owner are copied only where fully stored.
pub(super) fn settable(meta: &Metadata, caps: &Capabilities) -> SetAttr {
    let stored = |field| caps.stores(field) != Stored::No;
    let mut set = SetAttr::new();
    if let Some(time) = meta.created().filter(|_| stored(Field::Created)) {
        set = set.with_created(time);
    }
    if let Some(time) = meta.modified().filter(|_| stored(Field::Modified)) {
        set = set.with_modified(time);
    }
    if let Some(time) = meta.accessed().filter(|_| stored(Field::Accessed)) {
        set = set.with_accessed(time);
    }
    if caps.stores(Field::Permissions) == Stored::Yes {
        set = set.with_permissions(meta.permissions());
    }
    if let Some(owner) = meta
        .owner()
        .filter(|_| caps.stores(Field::Owner) == Stored::Yes)
    {
        set = set.with_owner(owner);
    }
    if stored(Field::Attributes) {
        set = set.with_attributes(meta.attributes());
    }
    set
}

/// A directory being copied: both nodes pinned, and the changes to apply to
/// the target once its contents are written (none for the top directory).
struct Frame {
    src: NodeId,
    dst: NodeId,
    cursor: DirCursor,
    attrs: Option<SetAttr>,
}

io_transform! {

/// Finds `name` in `dir`, or makes it as a directory (`dir_kind`) or a
/// file, and pins it. An existing node must have the same type
/// ([`ErrorKind::AlreadyExists`] otherwise, and always for a symlink); an
/// existing file is truncated.
pub(super) async fn target_child<F: FileSystem + ?Sized>(
    fs: &mut F,
    dir: NodeId,
    name: &Name,
    kind: FileType,
) -> FsResult<NodeId, F::DeviceError> {
    let node = match fs.lookup(dir, name).await {
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return match kind {
                FileType::Dir => fs.mkdir(dir, name, &SetAttr::new()).await,
                _ => fs.create(dir, name, &SetAttr::new()).await,
            };
        }
        other => other?,
    };
    let checked = match fs.stat(node).await {
        Ok(meta) if meta.file_type() != kind || meta.file_type().is_symlink() => {
            Err(ErrorKind::AlreadyExists.into())
        }
        Ok(meta) if meta.file_type().is_file() && meta.len() > 0 => fs.truncate(node, 0).await,
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    };
    match checked {
        Ok(()) => Ok(node),
        Err(err) => {
            fs.forget(node, 1);
            Err(err)
        }
    }
}

async fn copy_file<S, D>(
    src: &mut S,
    from: NodeId,
    dst: &mut D,
    to: NodeId,
    attrs: &SetAttr,
) -> Result<(), PathError>
where
    S: FileSystem + ?Sized,
    D: FileSystem + ?Sized,
{
    src.open(from, OpenMode::Read).await?;
    if let Err(err) = dst.open(to, OpenMode::Write).await {
        let _ = src.close(from).await;
        return Err(err.into());
    }
    let mut buf = [0u8; CHUNK];
    let mut offset = 0;
    let copied: Result<(), PathError> = loop {
        let n = match src.read(from, offset, &mut buf).await {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(err) => break Err(err.into()),
        };
        if let Err(err) = write_all_at(dst, to, offset, &buf[..n]).await {
            break Err(err.into());
        }
        offset += n as u64;
    };
    let _ = src.close(from).await;
    let set = match copied {
        Ok(()) if !attrs.is_empty() => dst.setattr(to, attrs).await.map_err(PathError::from),
        other => other,
    };
    let closed = dst.close(to).await;
    set?;
    Ok(closed?)
}

/// Copies the pinned `node` to `name` in `dir`. A directory comes back as a
/// frame to walk; anything else is copied whole.
async fn copy_node<S, D>(
    src: &mut S,
    node: NodeId,
    dst: &mut D,
    dir: NodeId,
    name: &Name,
) -> Result<Option<Frame>, PathError>
where
    S: FileSystem + ?Sized,
    D: FileSystem + ?Sized,
{
    let meta = src.stat(node).await?;
    let attrs = settable(&meta, &dst.capabilities());
    match meta.file_type() {
        FileType::Dir => {
            let to = target_child(dst, dir, name, FileType::Dir).await?;
            Ok(Some(Frame { src: node, dst: to, cursor: DirCursor::START, attrs: Some(attrs) }))
        }
        FileType::File => {
            let to = target_child(dst, dir, name, FileType::File).await?;
            let copied = copy_file(src, node, dst, to, &attrs).await;
            dst.forget(to, 1);
            copied.map(|()| None)
        }
        _ => Err(ErrorKind::Unsupported.into()),
    }
}

async fn walk<S, D>(src: &mut S, dst: &mut D, stack: &mut Vec<Frame>) -> Result<(), PathError>
where
    S: FileSystem + ?Sized,
    D: FileSystem + ?Sized,
{
    while let Some(top) = stack.last_mut() {
        let (from_dir, to_dir) = (top.src, top.dst);
        if let Some(entry) = src.readdir(from_dir, top.cursor).await? {
            top.cursor = entry.next_cursor();
            let child = src.lookup(from_dir, entry.name()).await?;
            match copy_node(src, child, dst, to_dir, entry.name()).await {
                Ok(Some(frame)) => {
                    if let Err(err) = enter(stack.iter().map(|frame| frame.src), child) {
                        src.forget(child, 1);
                        dst.forget(frame.dst, 1);
                        return Err(err.into());
                    }
                    stack.push(frame);
                }
                other => {
                    src.forget(child, 1);
                    other?;
                }
            }
        } else if let Some(frame) = stack.pop() {
            let applied = match frame.attrs {
                Some(attrs) if !attrs.is_empty() => dst.setattr(frame.dst, &attrs).await,
                _ => Ok(()),
            };
            src.forget(frame.src, 1);
            dst.forget(frame.dst, 1);
            applied?;
        }
    }
    Ok(())
}

async fn copy_dir<S, D>(src: &mut S, node: NodeId, dst: &mut D, to: &[u8]) -> Result<(), PathError>
where
    S: FileSystem + ?Sized,
    D: FileSystem + ?Sized,
{
    let top = match create_dir_all(dst, to, Resolve::Lexical).await {
        Ok(()) => dst.resolve(to, Resolve::Lexical).await,
        Err(err) => Err(err),
    };
    let top = match top {
        Ok(top) => top,
        Err(err) => {
            src.forget(node, 1);
            return Err(err.into());
        }
    };
    let mut stack = Vec::new();
    stack.push(Frame { src: node, dst: top, cursor: DirCursor::START, attrs: None });
    let result = walk(src, dst, &mut stack).await;
    for frame in stack {
        src.forget(frame.src, 1);
        dst.forget(frame.dst, 1);
    }
    result
}

/// Copies the file or directory tree at `from` on `src` to `to` on `dst`,
/// which may be different filesystems on different devices. Paths resolve
/// lexically, and errors from either device come back as [`PathError`].
///
/// A directory is merged into `to`, which is created with its parents when
/// missing. Existing files are overwritten; an existing node of another
/// type, or an existing symlink, fails with [`ErrorKind::AlreadyExists`].
/// Times, attributes and, where `dst` stores them fully, permissions and
/// owner are copied. Symlinks, device nodes, FIFOs and sockets fail with
/// [`ErrorKind::Unsupported`], since the shared trait cannot create them. A
/// directory entry that leads back to a directory on the path being copied
/// fails with [`ErrorKind::Corrupt`], and a tree more than 1024 directories
/// deep with [`ErrorKind::LimitExceeded`], which also ends a copy of a
/// directory into itself on one volume. Each file is closed, not flushed;
/// call `sync` on `dst` to make the copy durable.
///
/// ```rust,ignore
/// copy_tree(&mut iso, "/EFI", &mut fat, "/EFI")?;
/// ```
pub async fn copy_tree<S, D>(
    src: &mut S,
    from: impl AsRef<[u8]>,
    dst: &mut D,
    to: impl AsRef<[u8]>,
) -> Result<(), PathError>
where
    S: FileSystem + ?Sized,
    D: FileSystem + ?Sized,
{
    let to = to.as_ref();
    let node = src.resolve(from.as_ref(), Resolve::Lexical).await?;
    let is_dir = match src.stat(node).await {
        Ok(meta) => meta.file_type().is_dir(),
        Err(err) => {
            src.forget(node, 1);
            return Err(err.into());
        }
    };
    if is_dir {
        return copy_dir(src, node, dst, to).await;
    }
    let copied = match resolve_parent(dst, to, Resolve::Lexical).await {
        Ok((dir, name)) => {
            let copied = copy_node(src, node, dst, dir, name).await;
            dst.forget(dir, 1);
            copied.map(|_| ())
        }
        Err(err) => Err(err.into()),
    };
    src.forget(node, 1);
    copied
}

}
