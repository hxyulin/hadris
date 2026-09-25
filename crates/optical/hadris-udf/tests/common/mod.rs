#![allow(dead_code)]

use hadris_fs::{
    Content, DateTime, DeviceNumber, FileType, Node, Owner, Permissions, SetAttr, Tree,
};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::UdfOptions;

pub const SECTOR: BlockSize = BlockSize::new(2048).unwrap();

pub fn pattern(len: usize, seed: u32) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(31).wrapping_add(seed) % 251) as u8)
        .collect()
}

/// Files of every size class, nested and empty directories, names that
/// need 8-bit and 16-bit CS0, symlinks and a hard link.
pub fn sample() -> Tree {
    let mut tree = Tree::new();
    tree.insert("readme.txt", Node::file(Content::bytes("hello")))
        .unwrap();
    tree.insert("empty.txt", Node::file(Content::empty()))
        .unwrap();
    tree.insert("caf\u{e9}.txt", Node::file(Content::bytes("latin1")))
        .unwrap();
    tree.insert("emoji-\u{1F600}.bin", Node::file(Content::bytes("wide")))
        .unwrap();
    tree.insert(
        "docs/big.bin",
        Node::file(Content::bytes(pattern(70_000, 1))),
    )
    .unwrap();
    tree.insert("docs/sub/deep.txt", Node::file(Content::bytes("deep")))
        .unwrap();
    tree.insert("emptydir", Node::dir()).unwrap();
    for i in 0..80 {
        tree.insert(
            format!("many/file-{i:03}.txt"),
            Node::file(Content::bytes(format!("file {i}"))),
        )
        .unwrap();
    }
    tree.insert("abs", Node::symlink("/docs/sub/deep.txt"))
        .unwrap();
    tree.insert("docs/rel", Node::symlink("../readme.txt"))
        .unwrap();
    tree.link("readme.txt", "docs/link.txt").unwrap();
    tree
}

pub fn time(seconds: i64) -> DateTime {
    DateTime::from_unix_seconds(seconds).unwrap()
}

pub fn with_metadata(tree: &mut Tree) {
    let attrs = SetAttr::new()
        .with_permissions(Permissions::new(0o4751))
        .with_owner(Owner::new(1000, 100))
        .with_modified(time(1_700_000_000))
        .with_accessed(time(1_700_000_100));
    tree.remove("docs/link.txt").unwrap();
    tree.replace(
        "readme.txt",
        Node::file(Content::bytes("hello")).with_attrs(attrs),
    )
    .unwrap();
    tree.link("readme.txt", "docs/link.txt").unwrap();
    tree.replace(
        "docs",
        Node::dir().with_attrs(SetAttr::new().with_permissions(Permissions::new(0o750))),
    )
    .unwrap();
    tree.insert(
        "dev/null",
        Node::special(FileType::CharDevice, Some(DeviceNumber::new(1, 3))),
    )
    .unwrap();
}

/// Writes `tree` with `options` to a device of exactly the planned size.
pub fn image(tree: &Tree, options: &UdfOptions) -> Vec<u8> {
    let report = hadris_udf::plan(tree, options).unwrap();
    let mut dev = MemDevice::new(vec![0xAAu8; report.size() as usize], SECTOR);
    let written = hadris_udf::sync::write(&mut dev, tree, options).unwrap();
    assert_eq!(written, report);
    dev.into_inner()
}

pub fn open(bytes: Vec<u8>) -> hadris_udf::sync::UdfFs<MemDevice<Vec<u8>>> {
    hadris_udf::sync::UdfFs::open(MemDevice::new(bytes, SECTOR)).unwrap()
}

/// Recomputes the tag of the descriptor at `sector` over `crc_length`
/// bytes, keeping its identifier, version and location.
pub fn reseal(bytes: &mut [u8], sector: u64, crc_length: usize) {
    let at = sector as usize * 2048;
    let tag = hadris_udf::raw::Tag::read(&bytes[at..]).unwrap();
    hadris_udf::raw::Tag::seal(
        &mut bytes[at..at + 2048],
        tag.identifier.get(),
        tag.version.get(),
        tag.location.get(),
        crc_length,
    );
}

pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    use core::task::{Context, Poll, Waker};
    let mut future = core::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
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
