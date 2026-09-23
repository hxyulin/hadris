use std::collections::HashMap;

use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats, Metadata, Name,
    NameBuf, NewNode, NodeId, RemoveKind, RenameFlags, SetMetadata,
};

use crate::common::MemError;

enum Kind {
    Dir,
    File(Vec<u8>),
    Link(Vec<u8>),
}

struct Node {
    name: Vec<u8>,
    parent: usize,
    kind: Kind,
}

/// An in-memory filesystem with symlinks, `parent`, pin and open counting
/// and injectable device errors. It follows the `FsDriver` contract, which
/// the contract kit checks.
pub struct MemFs {
    nodes: Vec<Option<Node>>,
    pins: HashMap<u64, u32>,
    opens: HashMap<u64, u32>,
    writable: bool,
    fail_next: Option<MemError>,
}

fn id(index: usize) -> NodeId {
    NodeId::new(index as u64 + 1)
}

io_transform! {

impl MemFs {
    pub fn new() -> Self {
        let root = Node { name: Vec::new(), parent: 0, kind: Kind::Dir };
        Self {
            nodes: vec![Some(root)],
            pins: HashMap::new(),
            opens: HashMap::new(),
            writable: true,
            fail_next: None,
        }
    }

    pub fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    /// Adds a node without pinning it, for building fixtures.
    pub fn add(&mut self, parent: &str, name: &str, kind: NewNode<'_>, data: &[u8]) {
        let parent = self.find(parent).expect("fixture parent exists");
        let kind = match kind {
            NewNode::Dir => Kind::Dir,
            NewNode::Symlink(target) => Kind::Link(target.to_vec()),
            _ => Kind::File(data.to_vec()),
        };
        self.nodes.push(Some(Node { name: name.as_bytes().to_vec(), parent, kind }));
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
            n.as_ref().filter(|n| n.parent == dir && n.name == name).map(|_| i)
        })
    }

    /// Distinct pinned nodes, the root included.
    pub fn open_nodes(&self) -> usize {
        1 + self.pins.values().filter(|&&n| n > 0).count()
    }

    /// Makes the next device access fail with `err`.
    pub fn fail_next(&mut self, err: MemError) {
        self.fail_next = Some(err);
    }

    fn device(&mut self) -> FsResult<(), MemError> {
        match self.fail_next.take() {
            Some(err) => Err(hadris_fs::Error::from_device(err)),
            None => Ok(()),
        }
    }

    /// A live node; a removed one answers `NotFound` while it is pinned.
    fn node(&self, node: NodeId) -> FsResult<&Node, MemError> {
        let index = (node.get() as usize).checked_sub(1).ok_or(ErrorKind::InvalidHandle)?;
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
        if self.writable { Ok(()) } else { Err(ErrorKind::ReadOnly.into()) }
    }

    fn dir(&self, node: NodeId) -> FsResult<usize, MemError> {
        match self.node(node)?.kind {
            Kind::Dir => Ok(node.get() as usize - 1),
            _ => Err(ErrorKind::NotADirectory.into()),
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        let caps = Capabilities::new().with_symlinks();
        if self.writable { caps.with_writable() } else { caps }
    }

    pub fn root(&self) -> NodeId {
        id(0)
    }

    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, MemError> {
        self.device()?;
        let dir = self.dir(dir)?;
        let found = self.child(dir, name.as_bytes()).ok_or(ErrorKind::NotFound)?;
        Ok(self.pin(found))
    }

    pub async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, MemError> {
        self.device()?;
        Ok(match &self.node(node)?.kind {
            Kind::Dir => Metadata::new(FileType::Dir),
            Kind::File(data) => Metadata::new(FileType::File).with_len(data.len() as u64),
            Kind::Link(target) => Metadata::new(FileType::Symlink).with_len(target.len() as u64),
        })
    }

    pub async fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, MemError> {
        self.device()?;
        let dir = self.dir(dir)?;
        let start = cursor.into_raw() as usize + 1;
        for (i, node) in self.nodes.iter().enumerate().skip(start) {
            let Some(node) = node.as_ref().filter(|n| n.parent == dir) else { continue };
            name.set_bytes(&node.name)?;
            *cursor = DirCursor::from_raw(i as u64);
            let file_type = match node.kind {
                Kind::Dir => FileType::Dir,
                Kind::File(_) => FileType::File,
                Kind::Link(_) => FileType::Symlink,
            };
            return Ok(Some(DirEntry::new(id(i), file_type, node.name.len())));
        }
        Ok(None)
    }

    pub async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, MemError> {
        self.device()?;
        let Kind::File(data) = &self.node(node)?.kind else {
            return Err(ErrorKind::IsADirectory.into());
        };
        let rest = data.get(offset as usize..).unwrap_or(&[]);
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        Ok(n)
    }

    pub async fn stats(&mut self) -> FsResult<FsStats, MemError> {
        Ok(FsStats::new(1024, 512, 512))
    }

    pub fn forget(&mut self, node: NodeId) {
        if let Some(count) = self.pins.get_mut(&node.get()) {
            *count = count.saturating_sub(1);
        }
    }

    pub async fn open_node(&mut self, node: NodeId) -> FsResult<(), MemError> {
        self.node(node)?;
        *self.opens.entry(node.get()).or_default() += 1;
        Ok(())
    }

    pub fn close_node(&mut self, node: NodeId) {
        if let Some(count) = self.opens.get_mut(&node.get()) {
            *count = count.saturating_sub(1);
        }
    }

    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, MemError> {
        let dir = self.dir(dir)?;
        let up = self.nodes[dir].as_ref().map_or(0, |n| n.parent);
        Ok(self.pin(up))
    }

    pub async fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, MemError> {
        let Kind::Link(target) = &self.node(link)?.kind else {
            return Err(ErrorKind::InvalidInput.into());
        };
        let out = buf.get_mut(..target.len()).ok_or(ErrorKind::LimitExceeded)?;
        out.copy_from_slice(target);
        Ok(target.len())
    }

    pub async fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        _meta: &SetMetadata,
    ) -> FsResult<NodeId, MemError> {
        self.writable()?;
        self.device()?;
        let dir = self.dir(dir)?;
        if self.child(dir, name.as_bytes()).is_some() {
            return Err(ErrorKind::AlreadyExists.into());
        }
        let kind = match kind {
            NewNode::File => Kind::File(Vec::new()),
            NewNode::Dir => Kind::Dir,
            NewNode::Symlink(target) => Kind::Link(target.to_vec()),
            _ => return Err(ErrorKind::Unsupported.into()),
        };
        self.nodes.push(Some(Node { name: name.as_bytes().to_vec(), parent: dir, kind }));
        Ok(self.pin(self.nodes.len() - 1))
    }

    pub async fn remove(&mut self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), MemError> {
        self.writable()?;
        self.device()?;
        let dir = self.dir(dir)?;
        let index = self.child(dir, name.as_bytes()).ok_or(ErrorKind::NotFound)?;
        let file_type = self.file_type(index);
        kind.check(file_type)?;
        if self.is_open(index) {
            return Err(ErrorKind::Busy.into());
        }
        if self.nodes.iter().flatten().any(|n| n.parent == index) {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        self.nodes[index] = None;
        Ok(())
    }

    pub async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), MemError> {
        self.writable()?;
        if !RenameFlags::NO_REPLACE.contains(flags) {
            return Err(ErrorKind::Unsupported.into());
        }
        let (from_dir, to_dir) = (self.dir(from_dir)?, self.dir(to_dir)?);
        let index = self.child(from_dir, from.as_bytes()).ok_or(ErrorKind::NotFound)?;
        let moving_dir = self.file_type(index).is_dir();
        if moving_dir && self.is_within(to_dir, index) {
            return Err(ErrorKind::InvalidInput.into());
        }
        match self.child(to_dir, to.as_bytes()) {
            Some(old) if old == index => return Ok(()),
            Some(_) if flags.contains(RenameFlags::NO_REPLACE) => {
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

    pub async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, MemError> {
        self.writable()?;
        self.device()?;
        self.node(node)?;
        let index = node.get() as usize - 1;
        let Some(Node { kind: Kind::File(data), .. }) = self.nodes.get_mut(index).and_then(Option::as_mut) else {
            return Err(ErrorKind::IsADirectory.into());
        };
        let end = offset as usize + buf.len();
        if data.len() < end {
            data.resize(end, 0);
        }
        data[offset as usize..end].copy_from_slice(buf);
        Ok(buf.len())
    }

    pub async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), MemError> {
        self.writable()?;
        self.node(node)?;
        let index = node.get() as usize - 1;
        let Some(Node { kind: Kind::File(data), .. }) = self.nodes.get_mut(index).and_then(Option::as_mut) else {
            return Err(ErrorKind::IsADirectory.into());
        };
        data.resize(len as usize, 0);
        Ok(())
    }

    pub async fn set_metadata(&mut self, node: NodeId, _changes: &SetMetadata) -> FsResult<(), MemError> {
        self.writable()?;
        self.node(node)?;
        Ok(())
    }

    pub async fn sync_node(&mut self, node: NodeId) -> FsResult<(), MemError> {
        self.node(node)?;
        self.device()
    }

    pub async fn publish_node(&mut self, node: NodeId) -> FsResult<(), MemError> {
        self.node(node)?;
        self.device()
    }

    pub async fn sync(&mut self) -> FsResult<(), MemError> {
        self.device()
    }
}

}

impl_mem_fs!(
    impl[] MemFs,
    error = MemError;
    also = [parent, read_link, open_node, close_node, publish_node]
);
