use std::collections::HashMap;

use hadris_fs::{
    Capabilities, CaseRule, Charset, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats,
    Metadata, Name, NodeId, OpenMode, Permissions, RenameMode, SetAttr,
};

use super::FileSystem;
use crate::common::MemError;

enum Kind {
    Dir,
    File(Vec<u8>),
    Link(Vec<u8>),
    /// A directory entry for another directory, as a corrupt image can hold.
    Alias(usize),
}

struct Node {
    name: Vec<u8>,
    parent: usize,
    kind: Kind,
}

/// An in-memory filesystem with symlinks, `parent`, pin and open counting
/// and injectable device errors. It follows the `FileSystem` contract,
/// which the contract kit checks.
pub struct MemFs {
    nodes: Vec<Option<Node>>,
    pins: HashMap<u64, u64>,
    opens: HashMap<u64, u32>,
    closes: u32,
    writable: bool,
    fail_next: Option<MemError>,
}

fn id(index: usize) -> NodeId {
    NodeId::new(index as u64 + 1).unwrap()
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    pub fn new() -> Self {
        let root = Node {
            name: Vec::new(),
            parent: 0,
            kind: Kind::Dir,
        };
        Self {
            nodes: vec![Some(root)],
            pins: HashMap::new(),
            opens: HashMap::new(),
            closes: 0,
            writable: true,
            fail_next: None,
        }
    }

    pub fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    /// Adds a node without pinning it, for building fixtures. A symlink's
    /// target is `data`.
    pub fn add(&mut self, parent: &str, name: &str, kind: FileType, data: &[u8]) {
        let parent = self.find(parent).expect("fixture parent exists");
        let kind = match kind {
            FileType::Dir => Kind::Dir,
            FileType::Symlink => Kind::Link(data.to_vec()),
            _ => Kind::File(data.to_vec()),
        };
        self.nodes.push(Some(Node {
            name: name.as_bytes().to_vec(),
            parent,
            kind,
        }));
    }

    /// Adds `name` in `parent` as an entry for the directory `target`.
    pub fn alias(&mut self, parent: &str, name: &str, target: &str) {
        let parent = self.find(parent).expect("fixture parent exists");
        let target = self.find(target).expect("fixture target exists");
        self.nodes.push(Some(Node {
            name: name.as_bytes().to_vec(),
            parent,
            kind: Kind::Alias(target),
        }));
    }

    /// The contents of the file at `path`, read without the trait.
    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        match &self.nodes[self.find(path)?].as_ref()?.kind {
            Kind::File(data) | Kind::Link(data) => Some(data.clone()),
            _ => None,
        }
    }

    fn find(&self, path: &str) -> Option<usize> {
        let mut at = 0;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            at = self.child(at, part.as_bytes())?;
        }
        Some(at)
    }

    fn child(&self, dir: usize, name: &[u8]) -> Option<usize> {
        self.nodes.iter().enumerate().skip(1).find_map(|(i, n)| {
            n.as_ref()
                .filter(|n| n.parent == dir && n.name == name)
                .map(|_| i)
        })
    }

    /// Distinct pinned nodes, the root included.
    pub fn open_nodes(&self) -> usize {
        1 + self.pins.values().filter(|&&n| n > 0).count()
    }

    /// Nodes open for reading or writing.
    pub fn open_files(&self) -> usize {
        self.opens.values().filter(|&&n| n > 0).count()
    }

    /// `close` calls that succeeded.
    pub fn closes(&self) -> u32 {
        self.closes
    }

    /// Makes the next device access fail with `err`.
    pub fn fail_next(&mut self, err: MemError) {
        self.fail_next = Some(err);
    }

    fn device(&mut self) -> FsResult<(), MemError> {
        match self.fail_next.take() {
            Some(err) => Err(hadris_fs::Error::device(err, "device failed")),
            None => Ok(()),
        }
    }

    /// A live node; a removed one answers `NotFound` while it is pinned.
    fn node(&self, node: NodeId) -> FsResult<&Node, MemError> {
        let index = (node.get() as usize)
            .checked_sub(1)
            .ok_or(ErrorKind::InvalidHandle)?;
        match self.nodes.get(index) {
            Some(Some(found)) => Ok(found),
            Some(None) if self.pins.get(&node.get()).is_some_and(|&n| n > 0) => {
                Err(ErrorKind::NotFound.into())
            }
            _ => Err(ErrorKind::InvalidHandle.into()),
        }
    }

    fn is_open(&self, index: usize) -> bool {
        self.opens.get(&id(index).get()).is_some_and(|&n| n > 0)
    }

    fn is_within(&self, mut index: usize, ancestor: usize) -> bool {
        loop {
            if index == ancestor {
                return true;
            }
            if index == 0 {
                return false;
            }
            index = self.nodes[index].as_ref().map_or(0, |n| n.parent);
        }
    }

    fn pin(&mut self, index: usize) -> NodeId {
        if index != 0 {
            *self.pins.entry(id(index).get()).or_default() += 1;
        }
        id(index)
    }

    fn file_type(&self, index: usize) -> FileType {
        match self.nodes[index].as_ref().map(|n| &n.kind) {
            Some(Kind::Dir) => FileType::Dir,
            Some(Kind::Link(_)) => FileType::Symlink,
            _ => FileType::File,
        }
    }

    fn writable(&self) -> FsResult<(), MemError> {
        if self.writable {
            Ok(())
        } else {
            Err(ErrorKind::ReadOnly.into())
        }
    }

    fn dir(&self, node: NodeId) -> FsResult<usize, MemError> {
        match self.node(node)?.kind {
            Kind::Dir => Ok(node.get() as usize - 1),
            _ => Err(ErrorKind::NotADirectory.into()),
        }
    }

    fn metadata(&self, index: usize) -> Metadata {
        let (file_type, len) = match self.nodes[index].as_ref().map(|n| &n.kind) {
            Some(Kind::File(data)) => (FileType::File, data.len()),
            Some(Kind::Link(target)) => (FileType::Symlink, target.len()),
            _ => (FileType::Dir, 0),
        };
        Metadata::new(file_type, Permissions::new(0o755)).with_len(len as u64)
    }

    fn file_mut(&mut self, node: NodeId) -> FsResult<&mut Vec<u8>, MemError> {
        self.node(node)?;
        let index = node.get() as usize - 1;
        match self.nodes.get_mut(index).and_then(Option::as_mut) {
            Some(Node {
                kind: Kind::File(data),
                ..
            }) => Ok(data),
            _ => Err(ErrorKind::IsADirectory.into()),
        }
    }

    /// The entry `name` of `dir` that `unlink` or `rmdir` removes.
    fn removable(&self, dir: NodeId, name: &Name) -> FsResult<usize, MemError> {
        self.writable()?;
        name.check()?;
        let dir = self.dir(dir)?;
        let index = self
            .child(dir, name.as_bytes())
            .ok_or(ErrorKind::NotFound)?;
        if self.is_open(index) {
            return Err(ErrorKind::Busy.into());
        }
        Ok(index)
    }

    fn add_node(&mut self, dir: NodeId, name: &Name, kind: Kind) -> FsResult<NodeId, MemError> {
        self.writable()?;
        name.check()?;
        self.device()?;
        let dir = self.dir(dir)?;
        if self.child(dir, name.as_bytes()).is_some() {
            return Err(ErrorKind::AlreadyExists.into());
        }
        self.nodes.push(Some(Node {
            name: name.as_bytes().to_vec(),
            parent: dir,
            kind,
        }));
        Ok(self.pin(self.nodes.len() - 1))
    }
}

io_transform! {

impl FileSystem for MemFs {
    type DeviceError = MemError;

    fn capabilities(&self) -> Capabilities {
        let caps = Capabilities::new(CaseRule::Sensitive, Charset::Bytes, 255).with_symlinks();
        if self.writable { caps.with_writable() } else { caps }
    }

    fn root(&self) -> NodeId {
        id(0)
    }

    async fn statfs(&mut self) -> FsResult<FsStats, MemError> {
        Ok(FsStats::new(1024, 512, 512))
    }

    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, MemError> {
        let out = buf.get_mut(..3).ok_or(ErrorKind::LimitExceeded)?;
        out.copy_from_slice(b"MEM");
        Ok(core::str::from_utf8(out).ok())
    }

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, MemError> {
        name.check()?;
        self.device()?;
        let dir = self.dir(dir)?;
        let mut found = self.child(dir, name.as_bytes()).ok_or(ErrorKind::NotFound)?;
        if let Some(Node { kind: Kind::Alias(target), .. }) = &self.nodes[found] {
            found = *target;
        }
        Ok(self.pin(found))
    }

    fn forget(&mut self, node: NodeId, count: u64) {
        if let Some(pins) = self.pins.get_mut(&node.get()) {
            *pins = pins.saturating_sub(count);
        }
    }

    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, MemError> {
        let dir = self.dir(dir)?;
        let up = self.nodes[dir].as_ref().map_or(0, |n| n.parent);
        Ok(self.pin(up))
    }

    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, MemError> {
        self.device()?;
        if let Kind::Alias(_) = self.node(node)?.kind {
            return Err(ErrorKind::InvalidHandle.into());
        }
        Ok(self.metadata(node.get() as usize - 1))
    }

    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, MemError> {
        self.device()?;
        let dir = self.dir(dir)?;
        let start = (from.into_raw() as usize).max(1);
        for (i, node) in self.nodes.iter().enumerate().skip(start) {
            let Some(node) = node.as_ref().filter(|n| n.parent == dir) else { continue };
            let index = match node.kind {
                Kind::Alias(target) => target,
                _ => i,
            };
            let next = DirCursor::from_raw(i as u64 + 1);
            let entry = DirEntry::new(Name::new(&node.name), id(index), self.metadata(index), next)?;
            return Ok(Some(entry));
        }
        Ok(None)
    }

    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], MemError> {
        let Kind::Link(target) = &self.node(node)?.kind else {
            return Err(ErrorKind::InvalidInput.into());
        };
        let out = buf.get_mut(..target.len()).ok_or(ErrorKind::LimitExceeded)?;
        out.copy_from_slice(target);
        Ok(out)
    }

    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), MemError> {
        match self.node(node)?.kind {
            Kind::File(_) => {}
            Kind::Link(_) => return Err(ErrorKind::Symlink.into()),
            _ => return Err(ErrorKind::IsADirectory.into()),
        }
        if mode == OpenMode::Write {
            self.writable()?;
        }
        *self.opens.entry(node.get()).or_default() += 1;
        Ok(())
    }

    async fn close(&mut self, node: NodeId) -> FsResult<(), MemError> {
        if let Some(count) = self.opens.get_mut(&node.get()) {
            *count = count.saturating_sub(1);
        }
        self.device()?;
        self.closes += 1;
        Ok(())
    }

    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, MemError> {
        self.device()?;
        let Kind::File(data) = &self.node(node)?.kind else {
            return Err(ErrorKind::IsADirectory.into());
        };
        let rest = data.get(offset as usize..).unwrap_or(&[]);
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        Ok(n)
    }

    async fn setattr(&mut self, node: NodeId, _changes: &SetAttr) -> FsResult<(), MemError> {
        self.writable()?;
        self.node(node)?;
        Ok(())
    }

    async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, MemError> {
        self.writable()?;
        self.device()?;
        let data = self.file_mut(node)?;
        let end = offset as usize + buf.len();
        if data.len() < end {
            data.resize(end, 0);
        }
        data[offset as usize..end].copy_from_slice(buf);
        Ok(buf.len())
    }

    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), MemError> {
        self.writable()?;
        self.file_mut(node)?.resize(len as usize, 0);
        Ok(())
    }

    async fn fsync(&mut self, node: NodeId) -> FsResult<(), MemError> {
        self.node(node)?;
        self.device()
    }

    async fn create(&mut self, dir: NodeId, name: &Name, _attrs: &SetAttr) -> FsResult<NodeId, MemError> {
        self.add_node(dir, name, Kind::File(Vec::new()))
    }

    async fn mkdir(&mut self, dir: NodeId, name: &Name, _attrs: &SetAttr) -> FsResult<NodeId, MemError> {
        self.add_node(dir, name, Kind::Dir)
    }

    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), MemError> {
        let index = self.removable(dir, name)?;
        if self.file_type(index).is_dir() {
            return Err(ErrorKind::IsADirectory.into());
        }
        self.device()?;
        self.nodes[index] = None;
        Ok(())
    }

    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), MemError> {
        let index = self.removable(dir, name)?;
        if !self.file_type(index).is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        if self.nodes.iter().flatten().any(|n| n.parent == index) {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        self.device()?;
        self.nodes[index] = None;
        Ok(())
    }

    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), MemError> {
        self.writable()?;
        from.check()?;
        to.check()?;
        let (from_dir, to_dir) = (self.dir(from_dir)?, self.dir(to_dir)?);
        let index = self.child(from_dir, from.as_bytes()).ok_or(ErrorKind::NotFound)?;
        let moving_dir = self.file_type(index).is_dir();
        if moving_dir && self.is_within(to_dir, index) {
            return Err(ErrorKind::InvalidInput.into());
        }
        match self.child(to_dir, to.as_bytes()) {
            Some(old) if old == index => return Ok(()),
            Some(_) if mode == RenameMode::NoReplace => {
                return Err(ErrorKind::AlreadyExists.into());
            }
            Some(old) => {
                match (moving_dir, self.file_type(old).is_dir()) {
                    (true, false) => return Err(ErrorKind::NotADirectory.into()),
                    (false, true) => return Err(ErrorKind::IsADirectory.into()),
                    _ => {}
                }
                if self.is_open(old) {
                    return Err(ErrorKind::Busy.into());
                }
                if self.nodes.iter().flatten().any(|n| n.parent == old) {
                    return Err(ErrorKind::DirectoryNotEmpty.into());
                }
                self.nodes[old] = None;
            }
            None => {}
        }
        let node = self.nodes[index].as_mut().ok_or(ErrorKind::NotFound)?;
        node.parent = to_dir;
        node.name = to.as_bytes().to_vec();
        Ok(())
    }

    async fn sync(&mut self) -> FsResult<(), MemError> {
        self.device()
    }
}

}
