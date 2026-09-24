//! A format-neutral input tree for image writers.
//!
//! ISO 9660, UDF, the hybrid CD writer and cpio all take a [`Tree`] and
//! produce an image. A tree holds files, directories, symlinks, device
//! nodes and hard links, each with a [`SetMetadata`]. File contents are an
//! opaque [`Content`]: bytes in memory, a byte source read while the image
//! is written, a host file opened lazily (`std`), or extents already stored
//! on the device a session writer updates.
//!
//! Paths use `/`. A leading `/` is optional, empty components are ignored,
//! and `.` and `..` are refused. Children are kept sorted by name, so a
//! writer sees the same order whatever order entries were added in.
//!
//! ```rust
//! use hadris_fs::tree::{Content, NodeKind, Tree};
//! use hadris_fs::{DeviceKind, DeviceNumber, Mode, SetMetadata};
//!
//! let mut tree = Tree::new();
//! tree.add_file("boot/grub/grub.cfg", Content::bytes("set timeout=3"))?;
//! tree.add_dir("empty")?;
//! tree.add_symlink("latest", "releases/3.0")?;
//! tree.add_device("dev/console", DeviceKind::Char, DeviceNumber::new(5, 1))?;
//! tree.add_hard_link("boot/grub.cfg", "boot/grub/grub.cfg")?;
//! tree.set_metadata("latest", SetMetadata::new().with_mode(Mode::new(0o777)))?;
//!
//! let cfg = tree.get("/boot/grub.cfg").unwrap();
//! assert_eq!(cfg.links(), 2);
//! assert!(matches!(cfg.kind(), NodeKind::File(content) if content.len() == Some(13)));
//! let names: Vec<_> = tree.root().children().map(|(name, _)| name).collect();
//! assert_eq!(names, ["boot", "dev", "empty", "latest"]);
//! # Ok::<(), hadris_fs::tree::TreeError>(())
//! ```

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use crate::{DeviceKind, DeviceNumber, Error, ErrorKind, FileTimes, FileType, SetMetadata};

pub use crate::Extent;

/// A source whose reads block, read by every writer.
#[cfg(feature = "sync")]
pub(crate) trait BlockingSource: Send {
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, crate::AnyError>;
}

#[cfg(feature = "sync")]
impl<S: hadris_io::sync::ByteSource + Send> BlockingSource for S {
    fn len(&self) -> u64 {
        hadris_io::sync::ByteSource::len(self)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, crate::AnyError> {
        hadris_io::sync::ByteSource::read_at(self, offset, buf)
            .map_err(|err| crate::AnyError::from(Error::from_device(err)))
    }
}

/// The future of an [`AsyncSource`] read.
#[cfg(feature = "async-send")]
pub(crate) type ReadFuture<'a> = core::pin::Pin<
    alloc::boxed::Box<
        dyn core::future::Future<Output = Result<usize, crate::AnyError>> + Send + 'a,
    >,
>;

/// A source read with `.await`, read by the asynchronous writers.
#[cfg(feature = "async-send")]
pub(crate) trait AsyncSource: Send {
    fn len(&self) -> u64;
    fn read_at<'a>(&'a mut self, offset: u64, buf: &'a mut [u8]) -> ReadFuture<'a>;
}

#[cfg(feature = "async-send")]
impl<S: hadris_io::async_send::ByteSource> AsyncSource for S {
    fn len(&self) -> u64 {
        hadris_io::async_send::ByteSource::len(self)
    }

    fn read_at<'a>(&'a mut self, offset: u64, buf: &'a mut [u8]) -> ReadFuture<'a> {
        alloc::boxed::Box::pin(async move {
            hadris_io::async_send::ByteSource::read_at(self, offset, buf)
                .await
                .map_err(|err| crate::AnyError::from(Error::from_device(err)))
        })
    }
}

pub(crate) enum Repr {
    Bytes(Arc<[u8]>),
    #[cfg(feature = "sync")]
    Blocking(spin::Mutex<alloc::boxed::Box<dyn BlockingSource>>),
    #[cfg(feature = "async-send")]
    Async(async_lock::Mutex<alloc::boxed::Box<dyn AsyncSource>>),
    #[cfg(feature = "std")]
    Path {
        path: std::path::PathBuf,
        len: Option<u64>,
    },
    Stored(Arc<[Extent]>),
}

/// The contents of a file in a [`Tree`].
///
/// Opaque, so new kinds of content never break callers. Writers read it
/// through `ContentReader` in their mode module
/// (`hadris_fs::sync::ContentReader` and its async twins).
pub struct Content(pub(crate) Repr);

impl Content {
    /// No bytes.
    pub fn empty() -> Self {
        Self(Repr::Bytes(Arc::from(&[][..])))
    }

    /// Bytes held in memory.
    pub fn bytes(data: impl Into<Vec<u8>>) -> Self {
        Self(Repr::Bytes(Arc::from(data.into())))
    }

    /// Bytes from a blocking [`ByteSource`](hadris_io::sync::ByteSource),
    /// read while the image is written.
    ///
    /// Every writer reads it; the asynchronous writers block on its reads.
    #[cfg(feature = "sync")]
    pub fn source<S: hadris_io::sync::ByteSource + Send + 'static>(source: S) -> Self {
        Self(Repr::Blocking(spin::Mutex::new(alloc::boxed::Box::new(
            source,
        ))))
    }

    /// Bytes from an asynchronous [`ByteSource`](hadris_io::async_send::ByteSource)
    /// with `Send` futures, read while the image is written.
    ///
    /// The `r#async` and `async_send` writers read it; the blocking writers
    /// fail with [`ErrorKind::Unsupported`].
    #[cfg(feature = "async-send")]
    pub fn async_source<S: hadris_io::async_send::ByteSource + 'static>(source: S) -> Self {
        Self(Repr::Async(async_lock::Mutex::new(alloc::boxed::Box::new(
            source,
        ))))
    }

    /// The host file at `path`, opened when the image is written. Its length
    /// is read then too, through [`hadris_storage::file_len`], so a disk
    /// device such as `/dev/sdb` or `\\.\PhysicalDrive1` supplies its
    /// whole contents.
    ///
    /// Every writer reads it; the asynchronous writers block on its reads.
    #[cfg(feature = "std")]
    pub fn path(path: impl Into<std::path::PathBuf>) -> Self {
        Self(Repr::Path {
            path: path.into(),
            len: None,
        })
    }

    #[cfg(feature = "std")]
    fn host_file(path: std::path::PathBuf, len: u64) -> Self {
        Self(Repr::Path {
            path,
            len: Some(len),
        })
    }

    /// Bytes already stored on the device a writer updates, in the order of
    /// `extents`.
    ///
    /// A session writer that updates an image in place reuses them without
    /// copying. Writers that produce a new image fail with
    /// [`ErrorKind::Unsupported`] on such content.
    pub fn stored(extents: impl Into<Vec<Extent>>) -> Self {
        Self(Repr::Stored(Arc::from(extents.into())))
    }

    /// The length in bytes, or `None` for a host file not yet opened.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> Option<u64> {
        match &self.0 {
            Repr::Bytes(bytes) => Some(bytes.len() as u64),
            #[cfg(feature = "sync")]
            Repr::Blocking(source) => Some(source.lock().len()),
            #[cfg(feature = "async-send")]
            Repr::Async(source) => source.try_lock().map(|source| source.len()),
            #[cfg(feature = "std")]
            Repr::Path { len, .. } => *len,
            Repr::Stored(extents) => Some(extents.iter().map(Extent::len).sum()),
        }
    }

    /// The bytes, when they are held in memory.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.0 {
            Repr::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The extents, for content made with [`Content::stored`].
    pub fn stored_extents(&self) -> Option<&[Extent]> {
        match &self.0 {
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
        let mut out = f.debug_struct("Content");
        match &self.0 {
            Repr::Bytes(bytes) => out.field("bytes", &bytes.len()),
            #[cfg(feature = "sync")]
            Repr::Blocking(_) => out.field("source", &"blocking"),
            #[cfg(feature = "async-send")]
            Repr::Async(_) => out.field("source", &"async"),
            #[cfg(feature = "std")]
            Repr::Path { path, .. } => out.field("path", path),
            Repr::Stored(extents) => out.field("stored", extents),
        };
        out.finish()
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

/// A failed tree edit. Converts with `?` into [`Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeError {
    kind: ErrorKind,
}

impl TreeError {
    const fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    /// What went wrong: [`ErrorKind::InvalidInput`] for a bad path,
    /// [`ErrorKind::NotFound`], [`ErrorKind::AlreadyExists`],
    /// [`ErrorKind::NotADirectory`] when a path runs through a
    /// non-directory, or [`ErrorKind::IsADirectory`] when a hard link
    /// targets a directory.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tree edit failed: {}", self.kind)
    }
}

impl core::error::Error for TreeError {}

impl From<TreeError> for ErrorKind {
    fn from(err: TreeError) -> Self {
        err.kind
    }
}

impl<E> From<TreeError> for Error<E> {
    fn from(err: TreeError) -> Self {
        err.kind.into()
    }
}

impl From<TreeError> for crate::AnyError {
    fn from(err: TreeError) -> Self {
        err.kind.into()
    }
}

#[cfg(feature = "std")]
impl From<TreeError> for std::io::Error {
    fn from(err: TreeError) -> Self {
        std::io::Error::new(err.kind.into(), err)
    }
}

/// What a [`Warning`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarningKind {
    /// Metadata the format cannot store was dropped.
    IgnoredMetadata,
    /// The entry is stored under a different name.
    Renamed,
    /// The entry was not stored.
    Skipped,
    /// The entry could not be read and was left out of the tree.
    Unreadable,
}

/// Something a writer or [`Tree::from_fs`] did differently than asked,
/// without failing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Warning {
    path: String,
    kind: WarningKind,
    message: String,
}

impl Warning {
    /// A warning about `path`.
    pub fn new(path: impl Into<String>, kind: WarningKind, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind,
            message: message.into(),
        }
    }

    /// The tree path it concerns.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// What happened.
    pub fn kind(&self) -> WarningKind {
        self.kind
    }

    /// A description for people.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// What [`Tree::from_fs`] does with an entry it cannot read or store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OnError {
    /// Fail the whole import.
    #[default]
    Fail,
    /// Leave the entry out and record a [`Warning`] in [`Tree::warnings`].
    Warn,
}

/// Options for [`Tree::from_fs`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FromFsOptions {
    on_error: OnError,
}

impl FromFsOptions {
    /// The defaults: fail on the first unreadable entry.
    pub const fn new() -> Self {
        Self {
            on_error: OnError::Fail,
        }
    }

    /// Sets what happens to entries that cannot be read or stored: a host
    /// error, a name that is not UTF-8, a FIFO or a socket.
    pub const fn with_on_error(self, on_error: OnError) -> Self {
        Self { on_error }
    }

    /// What happens to entries that cannot be read or stored.
    pub const fn on_error(&self) -> OnError {
        self.on_error
    }
}

enum Kind {
    File(Content),
    Dir(BTreeMap<String, usize>),
    Symlink(Vec<u8>),
    Device(DeviceKind, DeviceNumber),
}

struct Node {
    kind: Kind,
    meta: SetMetadata,
    links: usize,
}

/// The kind of a [`TreeNode`], with its data.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum NodeKind<'a> {
    /// A regular file.
    File(&'a Content),
    /// A directory.
    Dir,
    /// A symbolic link and its target.
    Symlink(&'a [u8]),
    /// A device node.
    Device(DeviceKind, DeviceNumber),
}

/// Files, directories, symlinks, device nodes and hard links to write into
/// an image.
///
/// See the [module documentation](self).
pub struct Tree {
    nodes: Vec<Option<Node>>,
    free: Vec<usize>,
    warnings: Vec<Warning>,
}

const ROOT: usize = 0;

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn entries(node: TreeNode<'_>, f: &mut fmt::DebugMap<'_, '_>, prefix: &str) {
            for (name, child) in node.children() {
                let path = alloc::format!("{prefix}/{name}");
                f.entry(&path, &child.kind());
                entries(child, f, &path);
            }
        }
        let mut map = f.debug_map();
        entries(self.root(), &mut map, "");
        map.finish()
    }
}

/// The components of a tree path.
fn components(path: &str) -> Result<Vec<&str>, TreeError> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" => {}
            "." | ".." => return Err(TreeError::new(ErrorKind::InvalidInput)),
            part if part.contains('\0') => return Err(TreeError::new(ErrorKind::InvalidInput)),
            part => parts.push(part),
        }
    }
    Ok(parts)
}

fn merge_times(old: FileTimes, new: FileTimes) -> FileTimes {
    old.with_created(new.created().or(old.created()))
        .with_modified(new.modified().or(old.modified()))
        .with_accessed(new.accessed().or(old.accessed()))
        .with_changed(new.changed().or(old.changed()))
}

fn merge(old: SetMetadata, new: SetMetadata) -> SetMetadata {
    SetMetadata::new()
        .with_times(merge_times(old.times(), new.times()))
        .with_mode(new.mode().or(old.mode()))
        .with_uid(new.uid().or(old.uid()))
        .with_gid(new.gid().or(old.gid()))
        .with_attributes(new.attributes().or(old.attributes()))
}

impl Tree {
    /// An empty tree: a root directory with default metadata.
    pub fn new() -> Self {
        Self {
            nodes: alloc::vec![Some(Node {
                kind: Kind::Dir(BTreeMap::new()),
                meta: SetMetadata::new(),
                links: 1,
            })],
            free: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn node(&self, index: usize) -> &Node {
        match self.nodes.get(index) {
            Some(Some(node)) => node,
            _ => unreachable!("tree nodes are freed only when their last name goes"),
        }
    }

    fn node_mut(&mut self, index: usize) -> &mut Node {
        match self.nodes.get_mut(index) {
            Some(Some(node)) => node,
            _ => unreachable!("tree nodes are freed only when their last name goes"),
        }
    }

    fn children_mut(&mut self, index: usize) -> Option<&mut BTreeMap<String, usize>> {
        match &mut self.node_mut(index).kind {
            Kind::Dir(children) => Some(children),
            _ => None,
        }
    }

    fn find(&self, parts: &[&str]) -> Result<usize, TreeError> {
        let mut index = ROOT;
        for part in parts {
            match &self.node(index).kind {
                Kind::Dir(children) => {
                    index = *children
                        .get(*part)
                        .ok_or(TreeError::new(ErrorKind::NotFound))?;
                }
                _ => return Err(TreeError::new(ErrorKind::NotADirectory)),
            }
        }
        Ok(index)
    }

    fn alloc(&mut self, kind: Kind) -> usize {
        let node = Some(Node {
            kind,
            meta: SetMetadata::new(),
            links: 1,
        });
        match self.free.pop() {
            Some(index) => {
                self.nodes[index] = node;
                index
            }
            None => {
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        }
    }

    /// Finds the directory holding `parts`' last component, creating missing
    /// directories on the way, and checks that the name is free. Creates
    /// nothing when it fails.
    fn parent_for(&mut self, parts: &[&str]) -> Result<usize, TreeError> {
        let (name, dirs) = parts
            .split_last()
            .ok_or(TreeError::new(ErrorKind::InvalidInput))?;
        let mut index = ROOT;
        let mut missing = None;
        for (depth, part) in dirs.iter().enumerate() {
            match &self.node(index).kind {
                Kind::Dir(children) => match children.get(*part) {
                    Some(&child) => index = child,
                    None => {
                        missing = Some(depth);
                        break;
                    }
                },
                _ => return Err(TreeError::new(ErrorKind::NotADirectory)),
            }
        }
        match missing {
            None => match &self.node(index).kind {
                Kind::Dir(children) if children.contains_key(*name) => {
                    Err(TreeError::new(ErrorKind::AlreadyExists))
                }
                Kind::Dir(_) => Ok(index),
                _ => Err(TreeError::new(ErrorKind::NotADirectory)),
            },
            Some(depth) => {
                if !matches!(self.node(index).kind, Kind::Dir(_)) {
                    return Err(TreeError::new(ErrorKind::NotADirectory));
                }
                for part in &dirs[depth..] {
                    let child = self.alloc(Kind::Dir(BTreeMap::new()));
                    if let Some(children) = self.children_mut(index) {
                        children.insert((*part).to_string(), child);
                    }
                    index = child;
                }
                Ok(index)
            }
        }
    }

    fn insert(&mut self, path: &str, kind: Kind) -> Result<usize, TreeError> {
        let parts = components(path)?;
        let parent = self.parent_for(&parts)?;
        let child = self.alloc(kind);
        let name = parts[parts.len() - 1].to_string();
        if let Some(children) = self.children_mut(parent) {
            children.insert(name, child);
        }
        Ok(child)
    }

    /// Adds a file. Missing parent directories are created.
    ///
    /// Fails with [`ErrorKind::AlreadyExists`] when `path` exists.
    pub fn add_file(&mut self, path: &str, content: Content) -> Result<(), TreeError> {
        self.insert(path, Kind::File(content)).map(drop)
    }

    /// Adds a directory and any missing parents. An existing directory is
    /// left as it is; any other existing node fails with
    /// [`ErrorKind::AlreadyExists`].
    pub fn add_dir(&mut self, path: &str) -> Result<(), TreeError> {
        let parts = components(path)?;
        match self.find(&parts) {
            Ok(index) if matches!(self.node(index).kind, Kind::Dir(_)) => Ok(()),
            Ok(_) => Err(TreeError::new(ErrorKind::AlreadyExists)),
            Err(_) => self.insert(path, Kind::Dir(BTreeMap::new())).map(drop),
        }
    }

    /// Adds a symbolic link to `target`, which is stored as given.
    pub fn add_symlink(&mut self, path: &str, target: impl AsRef<[u8]>) -> Result<(), TreeError> {
        self.insert(path, Kind::Symlink(target.as_ref().to_vec()))
            .map(drop)
    }

    /// Adds a device node.
    pub fn add_device(
        &mut self,
        path: &str,
        kind: DeviceKind,
        number: DeviceNumber,
    ) -> Result<(), TreeError> {
        self.insert(path, Kind::Device(kind, number)).map(drop)
    }

    /// Adds `path` as another name for the existing node at `target`, which
    /// must not be a directory ([`ErrorKind::IsADirectory`]). Both names
    /// share contents and metadata.
    pub fn add_hard_link(&mut self, path: &str, target: &str) -> Result<(), TreeError> {
        let target = self.find(&components(target)?)?;
        if matches!(self.node(target).kind, Kind::Dir(_)) {
            return Err(TreeError::new(ErrorKind::IsADirectory));
        }
        let parts = components(path)?;
        let parent = self.parent_for(&parts)?;
        let name = parts[parts.len() - 1].to_string();
        if let Some(children) = self.children_mut(parent) {
            children.insert(name, target);
        }
        self.node_mut(target).links += 1;
        Ok(())
    }

    /// Merges `meta` into the metadata of the node at `path`: fields set in
    /// `meta` replace the stored ones, unset fields keep them. `""` or `"/"`
    /// is the root.
    pub fn set_metadata(&mut self, path: &str, meta: SetMetadata) -> Result<(), TreeError> {
        let index = self.find(&components(path)?)?;
        let node = self.node_mut(index);
        node.meta = merge(node.meta, meta);
        Ok(())
    }

    /// Removes the entry at `path`, with everything below it for a
    /// directory. Removing the root fails with [`ErrorKind::InvalidInput`].
    pub fn remove(&mut self, path: &str) -> Result<(), TreeError> {
        let parts = components(path)?;
        let (name, dirs) = parts
            .split_last()
            .ok_or(TreeError::new(ErrorKind::InvalidInput))?;
        let parent = self.find(dirs)?;
        let child = match &mut self.node_mut(parent).kind {
            Kind::Dir(children) => children
                .remove(*name)
                .ok_or(TreeError::new(ErrorKind::NotFound))?,
            _ => return Err(TreeError::new(ErrorKind::NotADirectory)),
        };
        self.unlink(child);
        Ok(())
    }

    fn unlink(&mut self, index: usize) {
        let mut pending = alloc::vec![index];
        while let Some(index) = pending.pop() {
            let node = self.node_mut(index);
            node.links -= 1;
            if node.links > 0 {
                continue;
            }
            if let Some(Node {
                kind: Kind::Dir(children),
                ..
            }) = self.nodes[index].take()
            {
                pending.extend(children.into_values());
            }
            self.free.push(index);
        }
    }

    /// The root directory.
    pub fn root(&self) -> TreeNode<'_> {
        TreeNode {
            tree: self,
            index: ROOT,
        }
    }

    /// The node at `path`, if it exists.
    pub fn get(&self, path: &str) -> Option<TreeNode<'_>> {
        let index = self.find(&components(path).ok()?).ok()?;
        Some(TreeNode { tree: self, index })
    }

    /// The content of the file at `path`, for replacing it in place.
    pub fn content_mut(&mut self, path: &str) -> Option<&mut Content> {
        let index = self.find(&components(path).ok()?).ok()?;
        match &mut self.node_mut(index).kind {
            Kind::File(content) => Some(content),
            _ => None,
        }
    }

    /// Entries [`Tree::from_fs`] or `from_filesystem` left out, with the
    /// reason.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    #[cfg(any(feature = "std", feature = "sync", feature = "async"))]
    pub(crate) fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }
}

/// A node of a [`Tree`], borrowed.
#[derive(Clone, Copy)]
pub struct TreeNode<'a> {
    tree: &'a Tree,
    index: usize,
}

impl<'a> TreeNode<'a> {
    fn node(&self) -> &'a Node {
        self.tree.node(self.index)
    }

    /// What the node is.
    pub fn kind(&self) -> NodeKind<'a> {
        match &self.node().kind {
            Kind::File(content) => NodeKind::File(content),
            Kind::Dir(_) => NodeKind::Dir,
            Kind::Symlink(target) => NodeKind::Symlink(target),
            Kind::Device(kind, number) => NodeKind::Device(*kind, *number),
        }
    }

    /// The node's file type.
    pub fn file_type(&self) -> FileType {
        match &self.node().kind {
            Kind::File(_) => FileType::File,
            Kind::Dir(_) => FileType::Dir,
            Kind::Symlink(_) => FileType::Symlink,
            Kind::Device(DeviceKind::Block, _) => FileType::BlockDevice,
            Kind::Device(_, _) => FileType::CharDevice,
        }
    }

    /// The metadata set on the node.
    pub fn metadata(&self) -> &'a SetMetadata {
        &self.node().meta
    }

    /// An identifier shared by every name of the node, so writers can tell
    /// hard links apart. Unique within the tree while the node exists.
    pub fn id(&self) -> usize {
        self.index
    }

    /// The number of names the node has.
    pub fn links(&self) -> usize {
        self.node().links
    }

    /// The children of a directory, sorted by name. Empty for anything else.
    pub fn children(&self) -> impl Iterator<Item = (&'a str, TreeNode<'a>)> + 'a {
        let tree = self.tree;
        let children = match &self.node().kind {
            Kind::Dir(children) => Some(children),
            _ => None,
        };
        children
            .into_iter()
            .flatten()
            .map(move |(name, &index)| (name.as_str(), TreeNode { tree, index }))
    }

    /// The child called `name`.
    pub fn child(&self, name: &str) -> Option<TreeNode<'a>> {
        match &self.node().kind {
            Kind::Dir(children) => children.get(name).map(|&index| TreeNode {
                tree: self.tree,
                index,
            }),
            _ => None,
        }
    }
}

impl fmt::Debug for TreeNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeNode")
            .field("kind", &self.kind())
            .field("links", &self.links())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "std")]
mod host {
    use super::*;
    use std::io;
    use std::path::{Path, PathBuf};

    fn time(value: io::Result<std::time::SystemTime>) -> Option<crate::DateTime> {
        let value = value.ok()?;
        let (seconds, nanoseconds) = match value.duration_since(std::time::UNIX_EPOCH) {
            Ok(after) => (i64::try_from(after.as_secs()).ok()?, after.subsec_nanos()),
            Err(before) => {
                let before = before.duration();
                let seconds = -i64::try_from(before.as_secs()).ok()?;
                match before.subsec_nanos() {
                    0 => (seconds, 0),
                    nanos => (seconds - 1, 1_000_000_000 - nanos),
                }
            }
        };
        crate::DateTime::new(seconds, nanoseconds).ok()
    }

    fn metadata_of(meta: &std::fs::Metadata) -> SetMetadata {
        let times = FileTimes::new()
            .with_modified(time(meta.modified()))
            .with_accessed(time(meta.accessed()))
            .with_created(time(meta.created()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let changed = crate::DateTime::new(meta.ctime(), meta.ctime_nsec() as u32).ok();
            SetMetadata::new()
                .with_times(times.with_changed(changed))
                .with_mode(crate::Mode::new(meta.mode()))
                .with_uid(meta.uid())
                .with_gid(meta.gid())
        }
        #[cfg(not(unix))]
        {
            SetMetadata::new().with_times(times)
        }
    }

    #[cfg(unix)]
    fn device_number(rdev: u64) -> DeviceNumber {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let (major, minor) = (
            ((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff),
            (rdev & 0xff) | ((rdev >> 12) & !0xff),
        );
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let (major, minor) = ((rdev >> 24) & 0xff, rdev & 0xff_ffff);
        DeviceNumber::new(major as u32, minor as u32)
    }

    enum Entry {
        File(u64),
        Dir,
        Symlink(Vec<u8>),
        Device(DeviceKind, DeviceNumber),
    }

    fn classify(path: &Path, meta: &std::fs::Metadata) -> io::Result<Entry> {
        let kind = meta.file_type();
        if kind.is_file() {
            return Ok(Entry::File(meta.len()));
        }
        if kind.is_dir() {
            return Ok(Entry::Dir);
        }
        if kind.is_symlink() {
            let target = std::fs::read_link(path)?;
            #[cfg(unix)]
            let target = std::os::unix::ffi::OsStrExt::as_bytes(target.as_os_str()).to_vec();
            #[cfg(not(unix))]
            let target = target
                .to_str()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "symlink target is not UTF-8")
                })?
                .replace('\\', "/")
                .into_bytes();
            return Ok(Entry::Symlink(target));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt};
            if kind.is_char_device() {
                return Ok(Entry::Device(DeviceKind::Char, device_number(meta.rdev())));
            }
            if kind.is_block_device() {
                return Ok(Entry::Device(DeviceKind::Block, device_number(meta.rdev())));
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "FIFOs and sockets cannot be stored in a tree",
        ))
    }

    struct Import {
        tree: Tree,
        on_error: OnError,
        #[cfg(unix)]
        inodes: BTreeMap<(u64, u64), String>,
    }

    impl Import {
        fn fail(&mut self, path: &str, err: io::Error) -> io::Result<()> {
            match self.on_error {
                OnError::Warn => {
                    self.tree
                        .warn(Warning::new(path, WarningKind::Unreadable, err.to_string()));
                    Ok(())
                }
                _ => Err(err),
            }
        }

        fn add(&mut self, host: &Path, path: &str) -> io::Result<bool> {
            let meta = std::fs::symlink_metadata(host)?;
            let entry = classify(host, &meta)?;
            #[cfg(unix)]
            if let Entry::File(_) = entry {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() > 1 {
                    let key = (meta.dev(), meta.ino());
                    if let Some(first) = self.inodes.get(&key) {
                        self.tree.add_hard_link(path, first)?;
                        return Ok(false);
                    }
                    self.inodes.insert(key, path.to_string());
                }
            }
            let is_dir = matches!(entry, Entry::Dir);
            match entry {
                Entry::File(len) => self
                    .tree
                    .add_file(path, Content::host_file(host.to_path_buf(), len))?,
                Entry::Dir => self.tree.add_dir(path)?,
                Entry::Symlink(target) => self.tree.add_symlink(path, target)?,
                Entry::Device(kind, number) => self.tree.add_device(path, kind, number)?,
            }
            self.tree.set_metadata(path, metadata_of(&meta))?;
            Ok(is_dir)
        }

        fn walk(&mut self, root: &Path) -> io::Result<()> {
            let mut pending: Vec<(PathBuf, String)> =
                alloc::vec![(root.to_path_buf(), String::new())];
            while let Some((dir, prefix)) = pending.pop() {
                let entries = match std::fs::read_dir(&dir) {
                    Ok(entries) => entries,
                    Err(err) => {
                        self.fail(if prefix.is_empty() { "/" } else { &prefix }, err)?;
                        continue;
                    }
                };
                let mut names = Vec::new();
                for entry in entries {
                    match entry {
                        Ok(entry) => names.push(entry.file_name()),
                        Err(err) => {
                            self.fail(if prefix.is_empty() { "/" } else { &prefix }, err)?
                        }
                    }
                }
                names.sort();
                for name in names {
                    let host = dir.join(&name);
                    let Some(text) = name.to_str() else {
                        let shown = alloc::format!("{prefix}/{}", name.to_string_lossy());
                        self.fail(
                            &shown,
                            io::Error::new(io::ErrorKind::InvalidData, "name is not UTF-8"),
                        )?;
                        continue;
                    };
                    let path = alloc::format!("{prefix}/{text}");
                    match self.add(&host, &path) {
                        Ok(true) => pending.push((host, path)),
                        Ok(false) => {}
                        Err(err) => self.fail(&path, err)?,
                    }
                }
            }
            Ok(())
        }
    }

    impl Tree {
        /// Builds a tree from the host directory `root`.
        ///
        /// Files are not read now: each becomes a [`Content::path`] that the
        /// writer opens. Symlinks are stored, never followed; device nodes
        /// keep their numbers; files that share an inode become hard links.
        /// Mode, owner and times are copied where the host has them, and
        /// `root`'s own metadata goes to the tree root.
        ///
        /// Entries that cannot be read, names that are not UTF-8, FIFOs and
        /// sockets fail the import, or with [`OnError::Warn`] are left out
        /// and listed in [`Tree::warnings`].
        pub fn from_fs(root: impl AsRef<Path>, options: FromFsOptions) -> io::Result<Self> {
            let root = root.as_ref();
            let meta = std::fs::metadata(root)?;
            if !meta.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "the root of a tree import must be a directory",
                ));
            }
            let mut import = Import {
                tree: Tree::new(),
                on_error: options.on_error(),
                #[cfg(unix)]
                inodes: BTreeMap::new(),
            };
            import.tree.set_metadata("/", metadata_of(&meta))?;
            import.walk(root)?;
            Ok(import.tree)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DateTime, Mode};

    #[test]
    fn parents_are_created_and_names_sorted() {
        let mut tree = Tree::new();
        tree.add_file("b/c.txt", Content::bytes(*b"c")).unwrap();
        tree.add_file("/a.txt", Content::empty()).unwrap();
        let names: Vec<_> = tree.root().children().map(|(name, _)| name).collect();
        assert_eq!(names, ["a.txt", "b"]);
        assert_eq!(tree.get("b").unwrap().file_type(), FileType::Dir);
        assert!(matches!(
            tree.get("b//c.txt").unwrap().kind(),
            NodeKind::File(content) if content.as_bytes() == Some(b"c")
        ));
    }

    #[test]
    fn bad_paths_and_clashes_change_nothing() {
        let mut tree = Tree::new();
        tree.add_file("f", Content::empty()).unwrap();
        assert_eq!(
            tree.add_file("f", Content::empty()).unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(
            tree.add_file("f/g", Content::empty()).unwrap_err().kind(),
            ErrorKind::NotADirectory
        );
        assert_eq!(
            tree.add_dir("a/../b").unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            tree.add_file("/", Content::empty()).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            tree.add_dir("f").unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        tree.add_dir("d").unwrap();
        tree.add_dir("d").unwrap();
        assert_eq!(tree.root().children().count(), 2);
    }

    #[test]
    fn hard_links_share_the_node_until_the_last_name_goes() {
        let mut tree = Tree::new();
        tree.add_file("a", Content::bytes(*b"x")).unwrap();
        tree.add_hard_link("dir/b", "a").unwrap();
        let (a, b) = (tree.get("a").unwrap(), tree.get("dir/b").unwrap());
        assert_eq!((a.id(), a.links()), (b.id(), 2));
        tree.set_metadata("dir/b", SetMetadata::new().with_uid(7))
            .unwrap();
        assert_eq!(tree.get("a").unwrap().metadata().uid(), Some(7));
        tree.remove("a").unwrap();
        assert_eq!(tree.get("dir/b").unwrap().links(), 1);
        assert_eq!(
            tree.add_hard_link("x", "dir").unwrap_err().kind(),
            ErrorKind::IsADirectory
        );
        tree.remove("dir").unwrap();
        assert!(tree.get("dir/b").is_none());
        tree.add_file("again", Content::empty()).unwrap();
        assert_eq!(tree.nodes.iter().filter(|node| node.is_some()).count(), 2);
    }

    #[test]
    fn metadata_merges() {
        let mut tree = Tree::new();
        tree.add_symlink("l", "t").unwrap();
        let time = DateTime::from_unix_seconds(5).unwrap();
        tree.set_metadata(
            "l",
            SetMetadata::new().with_times(FileTimes::new().with_modified(time)),
        )
        .unwrap();
        tree.set_metadata("l", SetMetadata::new().with_mode(Mode::new(0o700)))
            .unwrap();
        let meta = tree.get("l").unwrap().metadata();
        assert_eq!(
            (meta.times().modified(), meta.mode()),
            (Some(time), Some(Mode::new(0o700)))
        );
        tree.set_metadata("", SetMetadata::new().with_gid(3))
            .unwrap();
        assert_eq!(tree.root().metadata().gid(), Some(3));
    }

    #[test]
    fn contents_report_their_length() {
        let stored = Content::stored([Extent::new(2048, 10), Extent::new(8192, 5)]);
        assert_eq!(stored.len(), Some(15));
        assert_eq!(stored.stored_extents().unwrap().len(), 2);
        assert_eq!(Content::bytes("hi").len(), Some(2));
        #[cfg(feature = "sync")]
        assert_eq!(Content::source(alloc::vec![1u8, 2, 3]).len(), Some(3));
    }

    #[cfg(all(feature = "std", unix))]
    #[test]
    fn host_import_keeps_links_and_metadata() {
        let dir = std::env::temp_dir().join(alloc::format!("hadris-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), b"abc").unwrap();
        std::fs::hard_link(dir.join("sub/a.txt"), dir.join("b.txt")).unwrap();
        std::os::unix::fs::symlink("sub/a.txt", dir.join("link")).unwrap();
        let tree = Tree::from_fs(&dir, FromFsOptions::new()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        let a = tree.get("sub/a.txt").unwrap();
        assert_eq!((a.links(), a.id()), (2, tree.get("b.txt").unwrap().id()));
        assert!(matches!(a.kind(), NodeKind::File(content) if content.len() == Some(3)));
        assert!(matches!(
            tree.get("link").unwrap().kind(),
            NodeKind::Symlink(b"sub/a.txt")
        ));
        assert!(a.metadata().mode().is_some());
        assert!(tree.warnings().is_empty());
    }
}
