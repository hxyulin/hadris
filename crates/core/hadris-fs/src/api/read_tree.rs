use super::volume::Volume;
use super::*;
use crate::tree::Content;
use crate::{DeviceNumber, Node, PathError, Tree};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The longest symlink target read, as Linux `PATH_MAX`.
const MAX_LINK_TARGET: usize = 4096;

/// A buffer for the target of a symlink whose metadata reports `len` bytes.
/// [`ErrorKind::LimitExceeded`] above [`MAX_LINK_TARGET`].
fn link_buffer(len: u64) -> Result<Vec<u8>, ErrorKind> {
    match usize::try_from(len) {
        Ok(0) => Ok(alloc::vec![0u8; MAX_LINK_TARGET]),
        Ok(len) if len <= MAX_LINK_TARGET => Ok(alloc::vec![0u8; len]),
        _ => Err(ErrorKind::LimitExceeded),
    }
}

/// Directories a tree walk descends before [`ErrorKind::LimitExceeded`].
const MAX_TREE_DEPTH: usize = 1024;

/// Checks that a walk may enter the directory `child` below the directories
/// of `path`, the current path from the top: a directory already on it is a
/// cycle, which only a corrupt volume holds.
fn enter(
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

/// The attributes of a tree node that reproduce `meta`.
pub(super) fn attrs_of(meta: &Metadata) -> SetAttr {
    let mut attrs = SetAttr::new()
        .with_permissions(meta.permissions())
        .with_attributes(meta.attributes());
    if let Some(time) = meta.created() {
        attrs = attrs.with_created(time);
    }
    if let Some(time) = meta.modified() {
        attrs = attrs.with_modified(time);
    }
    if let Some(time) = meta.accessed() {
        attrs = attrs.with_accessed(time);
    }
    if let Some(owner) = meta.owner() {
        attrs = attrs.with_owner(owner);
    }
    attrs
}

/// A pinned file of a volume, read through a clone of the volume.
struct VolumeContent<F: FileSystem> {
    vol: Volume<F>,
    node: NodeId,
}

impl<F: FileSystem> Drop for VolumeContent<F> {
    fn drop(&mut self) {
        self.vol.release(self.node, false);
    }
}

io_transform! {

impl<F: FileSystem> VolumeContent<F> {
    async fn read(&self, offset: u64, buf: &mut [u8]) -> Result<usize, PathError> {
        let mut fs = self.vol.lock().await;
        fs.open(self.node, OpenMode::Read).await?;
        let read = fs.read(self.node, offset, buf).await;
        let closed = fs.close(self.node).await;
        let n = read?;
        closed?;
        Ok(n)
    }
}

/// Turns the file or directory at `path` on `vol` into a [`Tree`] whose file
/// content is read lazily through a clone of the volume, so converting a
/// large volume needs no memory for file data.
///
/// For a directory the tree's root takes the directory's attributes and
/// holds its contents; a file becomes the only node of the tree, under its
/// own name. Symlinks, device nodes, FIFOs and sockets are kept, and a file
/// listed again under the same node id becomes a hard link. Attributes are
/// what the volume reports. `path` resolves lexically.
///
/// Each file keeps its node pinned until the last clone of its content is
/// dropped, and the tree holds a clone of the volume, so
/// [`Volume::into_inner`] fails while the tree lives. The content is read
/// only by writers of this mode. Errors carry the path on the volume; a
/// directory that leads back to one on its own path fails with
/// [`ErrorKind::Corrupt`], and a tree more than 1024 directories deep with
/// [`ErrorKind::LimitExceeded`].
pub async fn read_tree<F: FileSystem + Send + 'static>(vol: &Volume<F>, path: impl AsRef<[u8]>) -> Result<Tree, PathError> {
    let path = path.as_ref();
    let mut fs = vol.lock().await;
    let top = fs.resolve(path, Resolve::Lexical).await.map_err(|err| PathError::from(err).with_path(path))?;
    let mut tree = Tree::new();
    let meta = match fs.stat(top).await {
        Ok(meta) => meta,
        Err(err) => {
            fs.forget(top, 1);
            return Err(PathError::from(err).with_path(path));
        }
    };
    if !meta.file_type().is_dir() {
        let name = path.rsplit(|&byte| byte == b'/').find(|part| !part.is_empty()).unwrap_or(path).to_vec();
        let node = match node_of(&mut *fs, vol, top, &meta).await {
            Ok(node) => node,
            Err(err) => {
                fs.forget(top, 1);
                return Err(err.with_path(path));
            }
        };
        tree.insert(&name, node)?;
        return Ok(tree);
    }
    tree.replace("/", Node::dir().with_attrs(attrs_of(&meta)))?;
    let mut stack: Vec<(NodeId, DirCursor, Vec<u8>)> = alloc::vec![(top, DirCursor::START, Vec::new())];
    let mut seen: Vec<(NodeId, Vec<u8>)> = Vec::new();
    let result = walk(&mut *fs, vol, &mut tree, &mut stack, &mut seen).await;
    for (node, ..) in stack {
        fs.forget(node, 1);
    }
    result.map(|()| tree)
}

async fn walk<F: FileSystem + Send + 'static>(
    fs: &mut F,
    vol: &Volume<F>,
    tree: &mut Tree,
    stack: &mut Vec<(NodeId, DirCursor, Vec<u8>)>,
    seen: &mut Vec<(NodeId, Vec<u8>)>,
) -> Result<(), PathError> {
    while let Some((dir, cursor, prefix)) = stack.last_mut() {
        let dir = *dir;
        let entry = match fs.readdir(dir, *cursor).await {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                if let Some((done, ..)) = stack.pop() {
                    fs.forget(done, 1);
                }
                continue;
            }
            Err(err) => return Err(PathError::from(err).with_path(&prefix[..])),
        };
        *cursor = entry.next_cursor();
        let mut path = prefix.clone();
        path.push(b'/');
        path.extend_from_slice(entry.name().as_bytes());
        let child = fs.lookup(dir, entry.name()).await.map_err(|err| PathError::from(err).with_path(&path))?;
        let meta = match fs.stat(child).await {
            Ok(meta) => meta,
            Err(err) => {
                fs.forget(child, 1);
                return Err(PathError::from(err).with_path(&path));
            }
        };
        if meta.file_type().is_dir() {
            if let Err(kind) = enter(stack.iter().map(|(node, ..)| *node), child) {
                fs.forget(child, 1);
                return Err(PathError::from(kind).with_path(&path));
            }
            tree.insert(&path, Node::dir().with_attrs(attrs_of(&meta)))?;
            stack.push((child, DirCursor::START, path));
            continue;
        }
        if meta.file_type().is_file() && meta.nlink() > 1 {
            if let Some((_, first)) = seen.iter().find(|(node, _)| *node == child) {
                fs.forget(child, 1);
                tree.link(first, &path)?;
                continue;
            }
            seen.push((child, path.clone()));
        }
        match node_of(fs, vol, child, &meta).await {
            Ok(node) => tree.insert(&path, node)?,
            Err(err) => {
                fs.forget(child, 1);
                return Err(err.with_path(&path));
            }
        }
    }
    Ok(())
}

/// The tree node for the pinned non-directory `node`. A file's content
/// keeps the pin and releases it when dropped; for anything else the pin is
/// forgotten here. On failure the caller forgets it.
async fn node_of<F: FileSystem + Send + 'static>(
    fs: &mut F,
    vol: &Volume<F>,
    node: NodeId,
    meta: &Metadata,
) -> Result<Node, PathError> {
    let attrs = attrs_of(meta);
    let made = match meta.file_type() {
        FileType::File => {
            let content = VolumeContent { vol: vol.clone(), node };
            return Ok(Node::file(lazy(content, meta.len())).with_attrs(attrs));
        }
        FileType::Symlink => {
            let mut target = link_buffer(meta.len())?;
            let n = fs.readlink(node, &mut target).await?.len();
            Node::symlink(&target[..n])
        }
        FileType::CharDevice | FileType::BlockDevice => {
            Node::special(meta.file_type(), Some(meta.device().unwrap_or(DeviceNumber::new(0, 0))))
        }
        other => Node::special(other, None),
    };
    fs.forget(node, 1);
    Ok(made.with_attrs(attrs))
}

}

sync_only! {
    impl<F: FileSystem + Send + 'static> crate::tree::SyncSource for VolumeContent<F> {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, PathError> {
            self.read(offset, buf)
        }
    }

    fn lazy<F: FileSystem + Send + 'static>(content: VolumeContent<F>, len: u64) -> Content {
        Content::sync_source(Arc::new(content), len)
    }
}

async_only! {
    impl<F: FileSystem + 'static> crate::tree::AsyncSource for VolumeContent<F> {
        fn read_at<'a>(&'a self, offset: u64, buf: &'a mut [u8]) -> crate::tree::ReadFuture<'a> {
            alloc::boxed::Box::pin(self.read(offset, buf))
        }
    }

    fn lazy<F: FileSystem + 'static>(content: VolumeContent<F>, len: u64) -> Content {
        Content::async_source(Arc::new(content), len)
    }
}
