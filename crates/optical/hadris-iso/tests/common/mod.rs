#![allow(dead_code)]

use hadris_fs::{
    Content, DateTime, DeviceNumber, FileType, Node, Owner, Permissions, SetAttr, Tree,
};
use hadris_iso::IsoOptions;
use hadris_storage::{BlockSize, MemDevice};

pub const SECTOR: BlockSize = match BlockSize::new(2048) {
    Some(size) => size,
    None => panic!(),
};

/// A tree with files, directories, a symlink, a device, a hard link and a
/// directory nested past the ECMA-119 depth limit.
pub fn sample(deep: bool, posix: bool) -> Tree {
    let mut tree = Tree::new();
    tree.insert("readme.txt", Node::file(Content::bytes("hello world\n")))
        .unwrap();
    tree.insert("docs/big.bin", Node::file(Content::bytes(pattern(100_000))))
        .unwrap();
    tree.insert("docs/empty.txt", Node::file(Content::empty()))
        .unwrap();
    tree.insert(
        "boot/boot.img",
        Node::file(Content::bytes(vec![0x90u8; 4096])),
    )
    .unwrap();
    tree.insert(
        "boot/efi.img",
        Node::file(Content::bytes(vec![0xEFu8; 8192])),
    )
    .unwrap();
    tree.insert("empty", Node::dir()).unwrap();
    tree.insert(
        "Long Name With Spaces é.txt",
        Node::file(Content::bytes("long")),
    )
    .unwrap();
    if deep {
        tree.insert(
            "a/b/c/d/e/f/g/h/i/deep.txt",
            Node::file(Content::bytes("deep")),
        )
        .unwrap();
    }
    if posix {
        tree.insert("docs/link", Node::symlink("../readme.txt"))
            .unwrap();
        tree.insert(
            "dev/null",
            Node::special(FileType::CharDevice, Some(DeviceNumber::new(1, 3))),
        )
        .unwrap();
        let time = DateTime::from_unix_seconds(1_700_000_000).unwrap();
        let attrs = SetAttr::new()
            .with_permissions(Permissions::new(0o600))
            .with_owner(Owner::new(1000, 100))
            .with_modified(time)
            .with_accessed(time);
        tree.replace(
            "readme.txt",
            Node::file(Content::bytes("hello world\n")).with_attrs(attrs),
        )
        .unwrap();
        tree.link("readme.txt", "docs/hard.txt").unwrap();
    }
    tree
}

pub fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Writes `tree` into a memory device sized by `plan`.
pub fn image(tree: &Tree, options: &IsoOptions) -> MemDevice<Vec<u8>> {
    let size = hadris_iso::plan(tree, options).unwrap().size();
    let mut dev = MemDevice::new(vec![0xA5u8; size as usize], SECTOR);
    let report = hadris_iso::sync::write(&mut dev, tree, options).unwrap();
    assert_eq!(report.size(), size);
    dev
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

/// Path reads for tests on the bare tier, as the removed `DriverExt` gave.
pub trait Paths: hadris_fs::sync::FileSystem {
    fn resolve_path(
        &mut self,
        path: &str,
    ) -> hadris_fs::FsResult<hadris_fs::NodeId, Self::DeviceError> {
        self.resolve(path.as_bytes(), hadris_fs::Resolve::Lexical)
    }

    fn metadata(
        &mut self,
        path: &str,
    ) -> hadris_fs::FsResult<hadris_fs::Metadata, Self::DeviceError> {
        let node = self.resolve_path(path)?;
        let meta = self.stat(node);
        self.forget(node, 1);
        meta
    }

    fn exists(&mut self, path: &str) -> hadris_fs::FsResult<bool, Self::DeviceError> {
        match self.resolve_path(path) {
            Ok(node) => {
                self.forget(node, 1);
                Ok(true)
            }
            Err(err) if err.kind() == hadris_fs::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn read_to_vec(&mut self, path: &str) -> hadris_fs::FsResult<Vec<u8>, Self::DeviceError> {
        let node = self.resolve_path(path)?;
        let data = read_node(self, node);
        self.forget(node, 1);
        data
    }

    fn names(&mut self, path: &str) -> hadris_fs::FsResult<Vec<String>, Self::DeviceError> {
        let dir = self.resolve_path(path)?;
        let mut names = Vec::new();
        let mut cursor = hadris_fs::DirCursor::START;
        let listed = loop {
            match self.readdir(dir, cursor) {
                Ok(Some(entry)) => {
                    cursor = entry.next_cursor();
                    names.push(String::from_utf8_lossy(entry.name().as_bytes()).into_owned());
                }
                Ok(None) => break Ok(names),
                Err(err) => break Err(err),
            }
        };
        self.forget(dir, 1);
        listed
    }
}

impl<F: hadris_fs::sync::FileSystem + ?Sized> Paths for F {}

/// Opens `node`, reads it whole and closes it.
pub fn read_node<F: hadris_fs::sync::FileSystem + ?Sized>(
    fs: &mut F,
    node: hadris_fs::NodeId,
) -> hadris_fs::FsResult<Vec<u8>, F::DeviceError> {
    fs.open(node, hadris_fs::OpenMode::Read)?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    let read = loop {
        match fs.read(node, out.len() as u64, &mut chunk) {
            Ok(0) => break Ok(()),
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(err) => break Err(err),
        }
    };
    let closed = fs.close(node);
    read?;
    closed?;
    Ok(out)
}

/// A boot catalog copied out of the caller buffer it was read into.
pub struct Catalog {
    block: u32,
    entries: Vec<hadris_iso::CatalogEntry>,
    valid: bool,
}

impl Catalog {
    pub fn block(&self) -> u32 {
        self.block
    }

    pub fn entries(&self) -> &[hadris_iso::CatalogEntry] {
        &self.entries
    }

    pub fn default_entry(&self) -> &hadris_iso::CatalogEntry {
        &self.entries[0]
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }
}

/// Extras read through the new buffer-lending calls, for tests that want
/// owned results.
#[allow(dead_code)]
pub trait IsoExtras {
    type E;
    fn catalog(&mut self) -> Result<Option<Catalog>, hadris_fs::Error<Self::E>>;
    fn all_extents(&mut self, node: hadris_fs::NodeId) -> Vec<hadris_fs::Extent>;
    fn record(&mut self, node: hadris_fs::NodeId) -> hadris_iso::raw::DirectoryRecord;
}

impl<D: hadris_storage::sync::BlockDevice> IsoExtras for hadris_iso::sync::IsoFs<D> {
    type E = D::Error;

    fn catalog(&mut self) -> Result<Option<Catalog>, hadris_fs::Error<D::Error>> {
        let mut buf = [0u8; 4096];
        Ok(self.boot_catalog(&mut buf)?.map(|catalog| Catalog {
            block: catalog.block(),
            entries: catalog.entries().collect(),
            valid: catalog.as_bytes()[30..32] == [0x55, 0xAA],
        }))
    }

    fn all_extents(&mut self, node: hadris_fs::NodeId) -> Vec<hadris_fs::Extent> {
        let mut out = [hadris_fs::Extent::new(0, 0); 2];
        let mut all = Vec::new();
        loop {
            let from = all
                .last()
                .map_or(0, |e: &hadris_fs::Extent| e.file_offset() + e.len());
            let n = self.extents(node, from, &mut out).unwrap();
            if n == 0 {
                return all;
            }
            all.extend_from_slice(&out[..n]);
        }
    }

    fn record(&mut self, node: hadris_fs::NodeId) -> hadris_iso::raw::DirectoryRecord {
        let mut out = [hadris_fs::Extent::new(0, 0); 1];
        assert_eq!(self.records(node, &mut out).unwrap(), 1);
        let mut bytes = vec![0u8; out[0].len() as usize];
        self.read_raw(out[0].offset(), &mut bytes).unwrap();
        hadris_iso::raw::DirectoryRecord::parse(&bytes)
            .unwrap()
            .unwrap()
    }
}
