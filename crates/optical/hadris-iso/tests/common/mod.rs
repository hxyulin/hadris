#![allow(dead_code)]

use hadris_fs::tree::{Content, Tree};
use hadris_fs::{DateTime, DeviceKind, DeviceNumber, FileTimes, Permissions, SetMetadata};
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
    tree.add_file("readme.txt", Content::bytes("hello world\n"))
        .unwrap();
    tree.add_file("docs/big.bin", Content::bytes(pattern(100_000)))
        .unwrap();
    tree.add_file("docs/empty.txt", Content::empty()).unwrap();
    tree.add_file("boot/boot.img", Content::bytes(vec![0x90u8; 4096]))
        .unwrap();
    tree.add_file("boot/efi.img", Content::bytes(vec![0xEFu8; 8192]))
        .unwrap();
    tree.add_dir("empty").unwrap();
    tree.add_file("Long Name With Spaces é.txt", Content::bytes("long"))
        .unwrap();
    if deep {
        tree.add_file("a/b/c/d/e/f/g/h/i/deep.txt", Content::bytes("deep"))
            .unwrap();
    }
    if posix {
        tree.add_symlink("docs/link", "../readme.txt").unwrap();
        tree.add_device("dev/null", DeviceKind::Char, DeviceNumber::new(1, 3))
            .unwrap();
        tree.add_hard_link("docs/hard.txt", "readme.txt").unwrap();
        let time = DateTime::from_unix_seconds(1_700_000_000).unwrap();
        tree.set_metadata(
            "readme.txt",
            SetMetadata::new()
                .with_mode(Permissions::new(0o600))
                .with_uid(1000)
                .with_gid(100)
                .with_times(FileTimes::new().with_modified(time).with_accessed(time)),
        )
        .unwrap();
    }
    tree
}

pub fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Writes `tree` into a memory device sized by `plan`.
pub fn image(tree: &Tree, options: &IsoOptions) -> MemDevice<Vec<u8>> {
    let size = hadris_iso::sync::plan(tree, options).unwrap().size_bytes();
    let mut dev = MemDevice::new(vec![0xA5u8; size as usize], SECTOR);
    let report = hadris_iso::sync::write(&mut dev, tree, options).unwrap();
    assert_eq!(report.size_bytes(), size);
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
