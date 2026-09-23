use super::*;
use crate::AnyError;
use alloc::vec::Vec;

io_transform! {

/// Bytes moved per read.
pub(super) const CHUNK: usize = 4096;

/// The longest symlink target copied, as Linux `PATH_MAX`.
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

/// A directory being copied: both nodes pinned, and the metadata to apply to
/// the target once its contents are written (none for the top directory).
struct Frame {
    src: NodeId,
    dst: NodeId,
    cursor: DirCursor,
    meta: Option<SetMetadata>,
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

/// Finds `name` in `dir`, or creates it as `kind`, and pins it. An existing
/// node must have the same type ([`ErrorKind::AlreadyExists`] otherwise, and
/// always for a symlink); an existing file is truncated.
pub(super) async fn target_child<D: FsDriver + ?Sized>(
    fs: &mut D,
    dir: NodeId,
    name: &Name,
    kind: NewNode<'_>,
) -> FsResult<NodeId, D::DeviceError> {
    let node = match fs.lookup(dir, name).await {
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return fs.create(dir, name, kind, &SetMetadata::new()).await;
        }
        other => other?,
    };
    let checked = match fs.node_metadata(node).await {
        Ok(meta) if meta.file_type() != kind.file_type() || meta.file_type().is_symlink() => {
            Err(ErrorKind::AlreadyExists.into())
        }
        Ok(meta) if meta.file_type().is_file() && meta.len() > 0 => fs.set_len(node, 0).await,
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    };
    match checked {
        Ok(()) => Ok(node),
        Err(err) => {
            fs.forget(node);
            Err(err)
        }
    }
}

/// Writes all of `buf` at `offset`.
pub(super) async fn write_all_at<D: FsDriver + ?Sized>(
    fs: &mut D,
    node: NodeId,
    mut offset: u64,
    mut buf: &[u8],
) -> FsResult<(), D::DeviceError> {
    while !buf.is_empty() {
        let n = fs.write_at(node, offset, buf).await?;
        if n == 0 {
            return Err(ErrorKind::NoSpace.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

async fn copy_file<S, D>(
    src: &mut S,
    from: NodeId,
    dst: &mut D,
    to: NodeId,
    meta: &SetMetadata,
) -> Result<(), AnyError>
where
    S: FsDriver + ?Sized,
    D: FsDriver + ?Sized,
{
    let mut buf = [0u8; CHUNK];
    let mut offset = 0;
    loop {
        let n = src.read_at(from, offset, &mut buf).await?;
        if n == 0 {
            break;
        }
        write_all_at(dst, to, offset, &buf[..n]).await?;
        offset += n as u64;
    }
    dst.set_metadata(to, meta).await?;
    dst.publish_node(to).await?;
    Ok(())
}

/// Copies the pinned `node` to `name` in `dir`. A directory comes back as a
/// frame to walk; anything else is copied whole.
async fn copy_node<S, D>(
    src: &mut S,
    node: NodeId,
    dst: &mut D,
    dir: NodeId,
    name: &Name,
) -> Result<Option<Frame>, AnyError>
where
    S: FsDriver + ?Sized,
    D: FsDriver + ?Sized,
{
    let meta = src.node_metadata(node).await?;
    let set = set_metadata_of(&meta);
    match meta.file_type() {
        FileType::Dir => {
            let to = target_child(dst, dir, name, NewNode::Dir).await?;
            Ok(Some(Frame { src: node, dst: to, cursor: DirCursor::start(), meta: Some(set) }))
        }
        FileType::File => {
            let to = target_child(dst, dir, name, NewNode::File).await?;
            let copied = copy_file(src, node, dst, to, &set).await;
            dst.forget(to);
            copied.map(|()| None)
        }
        FileType::Symlink => {
            let mut target = link_buffer(meta.len())?;
            let n = src.read_link(node, &mut target).await?;
            let to = target_child(dst, dir, name, NewNode::Symlink(&target[..n])).await?;
            dst.forget(to);
            Ok(None)
        }
        _ => Err(ErrorKind::Unsupported.into()),
    }
}

async fn walk<S, D>(src: &mut S, dst: &mut D, stack: &mut Vec<Frame>) -> Result<(), AnyError>
where
    S: FsDriver + ?Sized,
    D: FsDriver + ?Sized,
{
    let mut name = NameBuf::new();
    while let Some(top) = stack.last_mut() {
        let (from_dir, to_dir) = (top.src, top.dst);
        if src.read_dir_entry(from_dir, &mut top.cursor, &mut name).await?.is_some() {
            let child_name = name.as_name().ok_or(ErrorKind::Corrupt)?;
            let child = src.lookup(from_dir, child_name).await?;
            match copy_node(src, child, dst, to_dir, child_name).await {
                Ok(Some(frame)) => stack.push(frame),
                other => {
                    src.forget(child);
                    other?;
                }
            }
        } else if let Some(frame) = stack.pop() {
            let applied = match frame.meta {
                Some(meta) => dst.set_metadata(frame.dst, &meta).await,
                None => Ok(()),
            };
            src.forget(frame.src);
            dst.forget(frame.dst);
            applied?;
        }
    }
    Ok(())
}

async fn copy_dir<S, D>(src: &mut S, node: NodeId, dst: &mut D, to: &str) -> Result<(), AnyError>
where
    S: FsDriver + ?Sized,
    D: FsDriver + ?Sized,
{
    let top = match create_dir_all(dst, to).await {
        Ok(()) => dst.resolve(to).await,
        Err(err) => Err(err),
    };
    let top = match top {
        Ok(top) => top,
        Err(err) => {
            src.forget(node);
            return Err(err.into());
        }
    };
    let mut stack = Vec::new();
    stack.push(Frame { src: node, dst: top, cursor: DirCursor::start(), meta: None });
    let result = walk(src, dst, &mut stack).await;
    for frame in stack {
        src.forget(frame.src);
        dst.forget(frame.dst);
    }
    result
}

async fn copy_one<S, D>(src: &mut S, node: NodeId, dst: &mut D, to: &str) -> Result<(), AnyError>
where
    S: FsDriver + ?Sized,
    D: FsDriver + ?Sized,
{
    let (dir, name) = resolve_parent(dst, to).await?;
    let copied = copy_node(src, node, dst, dir, name).await;
    dst.forget(dir);
    copied.map(|_| ())
}

/// Copies the file, symlink or directory tree at `from` on `src` to `to` on
/// `dst`, which may be different filesystems on different devices.
///
/// Both sides are any [`Access`]: `&mut fs` for a raw driver, `&vol` for a
/// shared one, or an `Arc`. Errors from either device come back as
/// [`AnyError`].
///
/// A directory is merged into `to`, which is created with its parents when
/// missing. Existing files are overwritten; an existing node of another type,
/// or an existing symlink, fails with [`ErrorKind::AlreadyExists`]. Times,
/// permissions, owner and attributes are copied where `dst` can store them.
/// Device nodes, FIFOs and sockets fail with [`ErrorKind::Unsupported`].
/// Symlinks are copied as links, never followed; a target longer than 4096
/// bytes fails with [`ErrorKind::LimitExceeded`]. Copying a directory into
/// itself on one volume does not end until the volume is full. Each file is
/// published, not flushed; call `sync` on `dst` to make the copy durable.
///
/// ```rust,ignore
/// copy_tree(&mut iso, "/EFI", &card, "/EFI")?;
/// ```
pub async fn copy_tree<S: Access, T: Access>(
    src: S,
    from: &str,
    dst: T,
    to: &str,
) -> Result<(), AnyError> {
    let mut src = src.into_driver();
    let mut dst = dst.into_driver();
    let node = src.resolve(from).await?;
    let is_dir = match src.node_metadata(node).await {
        Ok(meta) => meta.file_type().is_dir(),
        Err(err) => {
            src.forget(node);
            return Err(err.into());
        }
    };
    if is_dir {
        return copy_dir(&mut src, node, &mut dst, to).await;
    }
    let copied = copy_one(&mut src, node, &mut dst, to).await;
    src.forget(node);
    copied
}

}
