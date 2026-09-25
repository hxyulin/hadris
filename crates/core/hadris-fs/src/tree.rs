//! The input tree of every writer and of `copy_tree`.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use crate::{DeviceNumber, ErrorKind, Extent, FileType, Name, PathError, SetAttr};

/// Content read lazily in the sync mode: a node of a sync `Volume`.
#[cfg(feature = "sync")]
pub(crate) trait SyncSource: Send + Sync {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, PathError>;
}

/// The future of an [`AsyncSource`] read.
#[cfg(feature = "async")]
pub(crate) type ReadFuture<'a> =
    core::pin::Pin<Box<dyn core::future::Future<Output = Result<usize, PathError>> + Send + 'a>>;

/// Content read lazily in the async mode: a node of an async `Volume`.
#[cfg(feature = "async")]
pub(crate) trait AsyncSource: Send + Sync {
    fn read_at<'a>(&'a self, offset: u64, buf: &'a mut [u8]) -> ReadFuture<'a>;
}

#[derive(Clone)]
pub(crate) enum Repr {
    Bytes(Arc<[u8]>),
    Stored(Arc<[Extent]>),
    #[cfg(all(feature = "std", feature = "sync"))]
    Host(Arc<std::path::Path>),
    #[cfg(feature = "sync")]
    Sync(Arc<dyn SyncSource>),
    #[cfg(feature = "async")]
    Async(Arc<dyn AsyncSource>),
}

/// The data of a file node.
///
/// Opaque, so new kinds of content never break callers, and cheap to clone:
/// clones share the data. Its length is fixed when it is made, so planning
/// an image does no I/O.
///
/// Content is bytes ([`Content::bytes`]), a host file (`host::file`, read
/// when the image is written), a node of a mounted volume (from
/// `read_tree`, read lazily through the volume), or extents already on the
/// device a session writer updates ([`Content::stored`]). Lazy content is
/// read only by writers of the mode that produced it: the other mode's
/// writers fail with [`ErrorKind::Unsupported`] naming the path, so async
/// code never blocks on a hidden sync read. Host files are sync content.
#[derive(Clone)]
pub struct Content {
    pub(crate) repr: Repr,
    len: u64,
}

impl Content {
    /// No bytes.
    pub fn empty() -> Self {
        Self::bytes(Vec::new())
    }

    /// Bytes held in memory.
    pub fn bytes(data: impl Into<Vec<u8>>) -> Self {
        let data: Arc<[u8]> = Arc::from(data.into());
        Self {
            len: data.len() as u64,
            repr: Repr::Bytes(data),
        }
    }

    /// Bytes already stored on the device a session writer updates, in the
    /// order of `extents`.
    ///
    /// A writer that updates an image in place (an ISO 9660 session or the
    /// UDF bridge) points at them without copying. Writers that produce a
    /// new image fail with [`ErrorKind::Unsupported`] on such content.
    pub fn stored(extents: impl Into<Vec<Extent>>) -> Self {
        let extents: Arc<[Extent]> = Arc::from(extents.into());
        Self {
            len: extents.iter().map(Extent::len).sum(),
            repr: Repr::Stored(extents),
        }
    }

    #[cfg(all(feature = "std", feature = "sync"))]
    pub(crate) fn host(path: std::path::PathBuf, len: u64) -> Self {
        Self {
            repr: Repr::Host(Arc::from(path.into_boxed_path())),
            len,
        }
    }

    #[cfg(feature = "sync")]
    #[allow(dead_code)]
    pub(crate) fn sync_source(source: Arc<dyn SyncSource>, len: u64) -> Self {
        Self {
            repr: Repr::Sync(source),
            len,
        }
    }

    #[cfg(feature = "async")]
    #[allow(dead_code)]
    pub(crate) fn async_source(source: Arc<dyn AsyncSource>, len: u64) -> Self {
        Self {
            repr: Repr::Async(source),
            len,
        }
    }

    /// The length in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the content has no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes, when they are held in memory.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.repr {
            Repr::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The extents, for content made with [`Content::stored`].
    pub fn stored_extents(&self) -> Option<&[Extent]> {
        match &self.repr {
            Repr::Stored(extents) => Some(extents),
            _ => None,
        }
    }
}

impl Default for Content {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.repr {
            Repr::Bytes(_) => "bytes",
            Repr::Stored(_) => "stored",
            #[cfg(all(feature = "std", feature = "sync"))]
            Repr::Host(_) => "host file",
            #[cfg(feature = "sync")]
            Repr::Sync(_) => "sync volume",
            #[cfg(feature = "async")]
            Repr::Async(_) => "async volume",
        };
        f.debug_struct("Content")
            .field("kind", &kind)
            .field("len", &self.len)
            .finish()
    }
}

impl From<Vec<u8>> for Content {
    fn from(data: Vec<u8>) -> Self {
        Self::bytes(data)
    }
}

impl From<&[u8]> for Content {
    fn from(data: &[u8]) -> Self {
        Self::bytes(data)
    }
}

#[derive(Clone)]
enum Kind {
    File(Content),
    Dir,
    Symlink(Arc<[u8]>),
    Special(FileType, Option<DeviceNumber>),
}

/// One node of a [`Tree`]: a file with its content, a directory, a symlink
/// or a special file, with the attributes it should get.
///
/// Unset attributes take the writer's defaults: the build time, `0o644` or
/// `0o755`, and owner 0 where the format stores owners. A format that
/// cannot store an attribute drops it and says so in its
/// [`Report`](crate::Report).
#[derive(Clone)]
pub struct Node {
    kind: Kind,
    attrs: SetAttr,
}

impl Node {
    /// A regular file.
    pub fn file(content: Content) -> Self {
        Self::of(Kind::File(content))
    }

    /// A directory. Its children are inserted by path.
    pub fn dir() -> Self {
        Self::of(Kind::Dir)
    }

    /// A symbolic link. The target is stored as given, bytes and all.
    pub fn symlink(target: impl AsRef<[u8]>) -> Self {
        Self::of(Kind::Symlink(Arc::from(target.as_ref())))
    }

    /// A character or block device with its number, or a FIFO or socket
    /// without one. Other combinations fail with
    /// [`ErrorKind::InvalidInput`] when inserted.
    pub fn special(file_type: FileType, device: Option<DeviceNumber>) -> Self {
        Self::of(Kind::Special(file_type, device))
    }

    const fn of(kind: Kind) -> Self {
        Self {
            kind,
            attrs: SetAttr::new(),
        }
    }

    /// The node with `attrs` as its attributes.
    #[must_use]
    pub fn with_attrs(self, attrs: SetAttr) -> Self {
        Self { attrs, ..self }
    }

    /// What the node is.
    pub fn file_type(&self) -> FileType {
        match &self.kind {
            Kind::File(_) => FileType::File,
            Kind::Dir => FileType::Dir,
            Kind::Symlink(_) => FileType::Symlink,
            Kind::Special(file_type, _) => *file_type,
        }
    }

    /// The attributes set on the node.
    pub fn attrs(&self) -> &SetAttr {
        &self.attrs
    }

    /// The content of a file.
    pub fn content(&self) -> Option<&Content> {
        match &self.kind {
            Kind::File(content) => Some(content),
            _ => None,
        }
    }

    /// The target of a symlink.
    pub fn target(&self) -> Option<&[u8]> {
        match &self.kind {
            Kind::Symlink(target) => Some(target),
            _ => None,
        }
    }

    /// The device number of a character or block device.
    pub fn device(&self) -> Option<DeviceNumber> {
        match &self.kind {
            Kind::Special(_, device) => *device,
            _ => None,
        }
    }

    fn is_dir(&self) -> bool {
        matches!(self.kind, Kind::Dir)
    }

    fn check(&self) -> Result<(), ErrorKind> {
        match self.kind {
            Kind::Special(FileType::CharDevice | FileType::BlockDevice, Some(_))
            | Kind::Special(FileType::Fifo | FileType::Socket, None) => Ok(()),
            Kind::Special(..) => Err(ErrorKind::InvalidInput),
            _ => Ok(()),
        }
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("Node");
        out.field("file_type", &self.file_type());
        match &self.kind {
            Kind::File(content) => out.field("content", content),
            Kind::Symlink(target) => out.field("target", &Name::new(&**target)),
            Kind::Special(_, Some(device)) => out.field("device", device),
            _ => &mut out,
        };
        out.field("attrs", &self.attrs).finish()
    }
}

struct Slot {
    node: Node,
    children: BTreeMap<Box<[u8]>, usize>,
    links: usize,
}

/// Files, directories, symlinks, special files and hard links for a writer
/// or for `copy_tree`.
///
/// Paths are `/`-separated bytes and need not be UTF-8. A leading `/` or
/// `./` and empty components are ignored; `.` and `..` components and NUL
/// bytes fail with [`ErrorKind::InvalidInput`], so a tree never names
/// anything outside its root. Children are kept sorted by their name bytes,
/// so a writer's output never depends on the order nodes were inserted in
/// or on a host's listing order. The tree holds no layout state.
///
/// ```rust
/// use hadris_fs::{Content, DeviceNumber, FileType, Node, Permissions, SetAttr, Tree};
///
/// let mut tree = Tree::new();
/// tree.insert("boot/grub/grub.cfg", Node::file(Content::bytes("set timeout=3")))?;
/// tree.insert("empty", Node::dir())?;
/// tree.insert("latest", Node::symlink("releases/3.0"))?;
/// tree.insert(
///     "dev/console",
///     Node::special(FileType::CharDevice, Some(DeviceNumber::new(5, 1))),
/// )?;
/// tree.insert("bin/busybox", Node::file(Content::bytes(*b"\x7fELF")))?;
/// tree.link("bin/busybox", "bin/sh")?;
/// tree.replace(
///     "latest",
///     Node::symlink("releases/3.1")
///         .with_attrs(SetAttr::new().with_permissions(Permissions::new(0o777))),
/// )?;
///
/// assert_eq!(tree.get("/bin/sh").unwrap().content().unwrap().len(), 4);
/// let names: Vec<_> = tree.root().children().map(|(name, _)| name.as_bytes()).collect();
/// assert_eq!(names, [&b"bin"[..], b"boot", b"dev", b"empty", b"latest"]);
/// # Ok::<(), hadris_fs::PathError>(())
/// ```
pub struct Tree {
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
}

const ROOT: usize = 0;

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn entries(entry: TreeEntry<'_>, map: &mut fmt::DebugMap<'_, '_>, prefix: &mut Vec<u8>) {
            for (name, child) in entry.children() {
                let len = prefix.len();
                prefix.push(b'/');
                prefix.extend_from_slice(name.as_bytes());
                map.entry(&Name::new(&prefix[..]), child.node());
                if child.node().is_dir() {
                    entries(child, map, prefix);
                }
                prefix.truncate(len);
            }
        }
        let mut map = f.debug_map();
        entries(self.root(), &mut map, &mut Vec::new());
        map.finish()
    }
}

fn path_error(kind: ErrorKind, message: &'static str, path: &[u8]) -> PathError {
    PathError::new(kind, message).with_path(path)
}

/// The components of a tree path.
fn components(path: &[u8]) -> Result<Vec<&[u8]>, PathError> {
    let bad = || {
        path_error(
            ErrorKind::InvalidInput,
            "tree paths have no . or .. components",
            path,
        )
    };
    let mut parts = Vec::new();
    for (index, part) in path.split(|&byte| byte == b'/').enumerate() {
        match part {
            b"" => {}
            b"." if index == 0 => {}
            b"." | b".." => return Err(bad()),
            part if part.contains(&0) => {
                return Err(path_error(
                    ErrorKind::InvalidInput,
                    "tree paths hold no NUL bytes",
                    path,
                ));
            }
            part => parts.push(part),
        }
    }
    Ok(parts)
}

impl Tree {
    /// A tree holding only the root directory, with no attributes set.
    pub fn new() -> Self {
        Self {
            slots: alloc::vec![Some(Slot {
                node: Node::dir(),
                children: BTreeMap::new(),
                links: 1,
            })],
            free: Vec::new(),
        }
    }

    fn slot(&self, index: usize) -> &Slot {
        match self.slots.get(index) {
            Some(Some(slot)) => slot,
            _ => unreachable!("tree slots are freed only when their last name goes"),
        }
    }

    fn slot_mut(&mut self, index: usize) -> &mut Slot {
        match self.slots.get_mut(index) {
            Some(Some(slot)) => slot,
            _ => unreachable!("tree slots are freed only when their last name goes"),
        }
    }

    fn find(&self, parts: &[&[u8]]) -> Result<usize, ErrorKind> {
        let mut index = ROOT;
        for part in parts {
            let slot = self.slot(index);
            if !slot.node.is_dir() {
                return Err(ErrorKind::NotADirectory);
            }
            index = *slot.children.get(*part).ok_or(ErrorKind::NotFound)?;
        }
        Ok(index)
    }

    fn alloc(&mut self, node: Node) -> usize {
        let slot = Some(Slot {
            node,
            children: BTreeMap::new(),
            links: 1,
        });
        match self.free.pop() {
            Some(index) => {
                self.slots[index] = slot;
                index
            }
            None => {
                self.slots.push(slot);
                self.slots.len() - 1
            }
        }
    }

    /// The directory that holds the last component of `parts`, with
    /// missing directories created on the way. Checks that the name is
    /// free first, so a failure creates nothing.
    fn parent_for(&mut self, parts: &[&[u8]]) -> Result<usize, ErrorKind> {
        let (name, dirs) = parts.split_last().ok_or(ErrorKind::InvalidInput)?;
        let mut index = ROOT;
        let mut missing = None;
        for (depth, part) in dirs.iter().enumerate() {
            let slot = self.slot(index);
            if !slot.node.is_dir() {
                return Err(ErrorKind::NotADirectory);
            }
            match slot.children.get(*part) {
                Some(&child) => index = child,
                None => {
                    missing = Some(depth);
                    break;
                }
            }
        }
        let slot = self.slot(index);
        if !slot.node.is_dir() {
            return Err(ErrorKind::NotADirectory);
        }
        let Some(depth) = missing else {
            return match slot.children.contains_key(*name) {
                true => Err(ErrorKind::AlreadyExists),
                false => Ok(index),
            };
        };
        for part in &dirs[depth..] {
            let child = self.alloc(Node::dir());
            self.slot_mut(index)
                .children
                .insert(Box::from(*part), child);
            index = child;
        }
        Ok(index)
    }

    /// Adds `node` at `path`, creating missing parent directories with no
    /// attributes set.
    ///
    /// Fails with [`ErrorKind::AlreadyExists`] when the path exists, with
    /// [`ErrorKind::NotADirectory`] when a parent is not a directory, and
    /// with [`ErrorKind::InvalidInput`] for the root, a bad path or a
    /// special node that is not a device with a number or a FIFO or socket
    /// without one. A failed insert changes nothing.
    pub fn insert(&mut self, path: impl AsRef<[u8]>, node: Node) -> Result<(), PathError> {
        let path = path.as_ref();
        node.check().map_err(|kind| {
            path_error(
                kind,
                "a device needs a number and nothing else has one",
                path,
            )
        })?;
        let parts = components(path)?;
        let parent = self
            .parent_for(&parts)
            .map_err(|kind| path_error(kind, "cannot insert into the tree", path))?;
        let child = self.alloc(node);
        let name = Box::from(parts[parts.len() - 1]);
        self.slot_mut(parent).children.insert(name, child);
        Ok(())
    }

    /// Gives the existing node at `existing` the second name `new`, as
    /// `link(2)` does. Every name shares the node, its content and its
    /// attributes. Missing parents of `new` are created.
    ///
    /// Fails with [`ErrorKind::NotFound`] when `existing` does not exist,
    /// [`ErrorKind::IsADirectory`] when it is a directory, and
    /// [`ErrorKind::AlreadyExists`] when `new` exists.
    pub fn link(
        &mut self,
        existing: impl AsRef<[u8]>,
        new: impl AsRef<[u8]>,
    ) -> Result<(), PathError> {
        let (existing, new) = (existing.as_ref(), new.as_ref());
        let target = self
            .find(&components(existing)?)
            .map_err(|kind| path_error(kind, "the link target does not exist", existing))?;
        if self.slot(target).node.is_dir() {
            return Err(path_error(
                ErrorKind::IsADirectory,
                "directories cannot be hard linked",
                existing,
            ));
        }
        let parts = components(new)?;
        let parent = self
            .parent_for(&parts)
            .map_err(|kind| path_error(kind, "cannot insert into the tree", new))?;
        let name = Box::from(parts[parts.len() - 1]);
        self.slot_mut(parent).children.insert(name, target);
        self.slot_mut(target).links += 1;
        Ok(())
    }

    /// Removes the name `path`, with everything below it for a directory.
    /// The node goes with its last name.
    ///
    /// Fails with [`ErrorKind::NotFound`] when `path` does not exist and
    /// with [`ErrorKind::InvalidInput`] for the root.
    pub fn remove(&mut self, path: impl AsRef<[u8]>) -> Result<(), PathError> {
        let path = path.as_ref();
        let parts = components(path)?;
        let (name, dirs) = parts.split_last().ok_or_else(|| {
            path_error(ErrorKind::InvalidInput, "the root cannot be removed", path)
        })?;
        let parent = self
            .find(dirs)
            .map_err(|kind| path_error(kind, "no such path in the tree", path))?;
        let slot = self.slot_mut(parent);
        if !slot.node.is_dir() {
            return Err(path_error(
                ErrorKind::NotADirectory,
                "no such path in the tree",
                path,
            ));
        }
        let child = slot
            .children
            .remove(*name)
            .ok_or_else(|| path_error(ErrorKind::NotFound, "no such path in the tree", path))?;
        self.unlink(child);
        Ok(())
    }

    fn unlink(&mut self, index: usize) {
        let mut pending = alloc::vec![index];
        while let Some(index) = pending.pop() {
            let slot = self.slot_mut(index);
            slot.links -= 1;
            if slot.links > 0 {
                continue;
            }
            if let Some(slot) = self.slots[index].take() {
                pending.extend(slot.children.into_values());
            }
            self.free.push(index);
        }
    }

    /// Replaces the node at `path` with `node`, as `rename(2)` replaces its
    /// target.
    ///
    /// A directory replaced by a directory keeps its children and takes the
    /// new attributes; this is how the root gets attributes. A directory
    /// with children replaced by anything else fails with
    /// [`ErrorKind::DirectoryNotEmpty`]. Other names of a hard link keep the
    /// old node. Fails with [`ErrorKind::NotFound`] when `path` does not
    /// exist.
    pub fn replace(&mut self, path: impl AsRef<[u8]>, node: Node) -> Result<(), PathError> {
        let path = path.as_ref();
        node.check().map_err(|kind| {
            path_error(
                kind,
                "a device needs a number and nothing else has one",
                path,
            )
        })?;
        let parts = components(path)?;
        let index = self
            .find(&parts)
            .map_err(|kind| path_error(kind, "no such path in the tree", path))?;
        let slot = self.slot(index);
        if slot.node.is_dir() && node.is_dir() {
            self.slot_mut(index).node = node;
            return Ok(());
        }
        if !slot.children.is_empty() {
            return Err(path_error(
                ErrorKind::DirectoryNotEmpty,
                "a directory with children can only become a directory",
                path,
            ));
        }
        let Some((name, dirs)) = parts.split_last() else {
            return Err(path_error(
                ErrorKind::InvalidInput,
                "the root is always a directory",
                path,
            ));
        };
        let parent = self
            .find(dirs)
            .map_err(|kind| path_error(kind, "no such path in the tree", path))?;
        let child = self.alloc(node);
        if let Some(old) = self
            .slot_mut(parent)
            .children
            .insert(Box::from(*name), child)
        {
            self.unlink(old);
        }
        Ok(())
    }

    /// The node at `path`; `""` or `"/"` is the root.
    pub fn get(&self, path: impl AsRef<[u8]>) -> Option<&Node> {
        self.entry(path).map(|entry| entry.node())
    }

    /// The node at `path` with its place in the tree, for walking it.
    pub fn entry(&self, path: impl AsRef<[u8]>) -> Option<TreeEntry<'_>> {
        let index = self.find(&components(path.as_ref()).ok()?).ok()?;
        Some(TreeEntry { tree: self, index })
    }

    /// The root directory, for walking the tree.
    pub fn root(&self) -> TreeEntry<'_> {
        TreeEntry {
            tree: self,
            index: ROOT,
        }
    }
}

/// A node of a [`Tree`] with its place in it: its children and the names
/// it shares with other paths.
#[derive(Clone, Copy)]
pub struct TreeEntry<'a> {
    tree: &'a Tree,
    index: usize,
}

impl<'a> TreeEntry<'a> {
    /// The node.
    pub fn node(&self) -> &'a Node {
        &self.tree.slot(self.index).node
    }

    /// The same for every name of a hard-linked node, and unique within the
    /// tree while the node exists, so writers can tell hard links apart.
    pub fn id(&self) -> usize {
        self.index
    }

    /// The number of names the node has.
    pub fn links(&self) -> usize {
        self.tree.slot(self.index).links
    }

    /// The children of a directory, sorted by their name bytes. Empty for
    /// anything else.
    pub fn children(&self) -> impl Iterator<Item = (&'a Name, TreeEntry<'a>)> + 'a {
        let tree = self.tree;
        tree.slot(self.index)
            .children
            .iter()
            .map(move |(name, &index)| (Name::new(&**name), TreeEntry { tree, index }))
    }

    /// The child called `name`.
    pub fn child(&self, name: impl AsRef<[u8]>) -> Option<TreeEntry<'a>> {
        let index = *self.tree.slot(self.index).children.get(name.as_ref())?;
        Some(TreeEntry {
            tree: self.tree,
            index,
        })
    }
}

impl fmt::Debug for TreeEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeEntry")
            .field("node", self.node())
            .field("links", &self.links())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DateTime, Permissions};

    fn file(bytes: &[u8]) -> Node {
        Node::file(Content::bytes(bytes))
    }

    #[test]
    fn parents_are_created_and_names_sorted() {
        let mut tree = Tree::new();
        tree.insert("b/c.txt", file(b"c")).unwrap();
        tree.insert("/a.txt", Node::file(Content::empty())).unwrap();
        tree.insert("./z", Node::dir()).unwrap();
        tree.insert(b"\xff", Node::dir()).unwrap();
        let names: Vec<_> = tree
            .root()
            .children()
            .map(|(name, _)| name.as_bytes())
            .collect();
        assert_eq!(names, [&b"a.txt"[..], b"b", b"z", b"\xff"]);
        assert_eq!(tree.get("b").unwrap().file_type(), FileType::Dir);
        assert_eq!(
            tree.get("b//c.txt").unwrap().content().unwrap().as_bytes(),
            Some(&b"c"[..])
        );
        assert_eq!(tree.get("").unwrap().file_type(), FileType::Dir);
    }

    #[test]
    fn bad_paths_and_clashes_change_nothing() {
        let mut tree = Tree::new();
        tree.insert("f", Node::file(Content::empty())).unwrap();
        let kind = |result: Result<(), PathError>| result.unwrap_err().kind();
        assert_eq!(
            kind(tree.insert("f", Node::dir())),
            ErrorKind::AlreadyExists
        );
        assert_eq!(
            kind(tree.insert("f/g", Node::dir())),
            ErrorKind::NotADirectory
        );
        assert_eq!(
            kind(tree.insert("a/../b", Node::dir())),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(tree.insert("a/./b", Node::dir())),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(tree.insert("a\0", Node::dir())),
            ErrorKind::InvalidInput
        );
        assert_eq!(kind(tree.insert("/", Node::dir())), ErrorKind::InvalidInput);
        assert_eq!(
            kind(tree.insert("new/dir/x", Node::special(FileType::CharDevice, None))),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(tree.insert(
                "p",
                Node::special(FileType::Fifo, Some(DeviceNumber::new(1, 2)))
            )),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(tree.insert("r", Node::special(FileType::File, None))),
            ErrorKind::InvalidInput
        );
        assert!(tree.get("new").is_none());
        let err = tree.insert("f", Node::dir()).unwrap_err();
        assert_eq!(err.path(), Some(&b"f"[..]));
        assert_eq!(tree.root().children().count(), 1);
    }

    #[test]
    fn hard_links_share_the_node_until_the_last_name_goes() {
        let mut tree = Tree::new();
        tree.insert("a", file(b"x")).unwrap();
        tree.link("a", "dir/b").unwrap();
        let (a, b) = (tree.entry("a").unwrap(), tree.entry("dir/b").unwrap());
        assert_eq!((a.id(), a.links()), (b.id(), 2));
        tree.remove("a").unwrap();
        assert_eq!(tree.entry("dir/b").unwrap().links(), 1);
        assert_eq!(
            tree.link("dir", "x").unwrap_err().kind(),
            ErrorKind::IsADirectory
        );
        assert_eq!(
            tree.link("missing", "x").unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            tree.link("dir/b", "dir/b").unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        tree.remove("dir").unwrap();
        assert!(tree.get("dir/b").is_none());
        tree.insert("again", Node::file(Content::empty())).unwrap();
        assert_eq!(tree.slots.iter().filter(|slot| slot.is_some()).count(), 2);
        assert_eq!(
            tree.remove("/").unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(tree.remove("gone").unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[test]
    fn replace_follows_rename() {
        let mut tree = Tree::new();
        let time = DateTime::from_unix_seconds(5).unwrap();
        tree.insert("d/f", file(b"old")).unwrap();
        tree.link("d/f", "g").unwrap();
        tree.replace(
            "d",
            Node::dir().with_attrs(SetAttr::new().with_modified(time)),
        )
        .unwrap();
        assert_eq!(tree.get("d").unwrap().attrs().modified(), Some(time));
        assert!(tree.get("d/f").is_some());
        assert_eq!(
            tree.replace("d", file(b"")).unwrap_err().kind(),
            ErrorKind::DirectoryNotEmpty
        );
        tree.replace("d/f", file(b"new")).unwrap();
        assert_eq!(
            tree.get("g").unwrap().content().unwrap().as_bytes(),
            Some(&b"old"[..])
        );
        assert_eq!(
            tree.get("d/f").unwrap().content().unwrap().as_bytes(),
            Some(&b"new"[..])
        );
        assert_eq!(tree.entry("g").unwrap().links(), 1);
        tree.replace(
            "/",
            Node::dir().with_attrs(SetAttr::new().with_permissions(Permissions::new(0o700))),
        )
        .unwrap();
        assert_eq!(
            tree.get("/").unwrap().attrs().permissions(),
            Some(Permissions::new(0o700))
        );
        assert_eq!(
            tree.replace("/", file(b"")).unwrap_err().kind(),
            ErrorKind::DirectoryNotEmpty
        );
        assert_eq!(
            tree.replace("none", file(b"")).unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn contents_report_their_length() {
        let stored = Content::stored([Extent::new(2048, 10), Extent::new(8192, 5)]);
        assert_eq!(stored.len(), 15);
        assert_eq!(stored.stored_extents().unwrap().len(), 2);
        assert_eq!(Content::bytes("hi").len(), 2);
        assert!(Content::empty().is_empty());
        let shared = Content::bytes(*b"abc");
        assert_eq!(shared.clone().as_bytes(), Some(&b"abc"[..]));
    }
}
