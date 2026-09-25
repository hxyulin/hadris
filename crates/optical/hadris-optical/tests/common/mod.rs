#![allow(dead_code)]

use hadris_fs::{Content, Node, Tree};
use hadris_storage::{BlockSize, MemDevice};

pub const PAYLOAD: &[u8] = b"optical traversal";
pub const SECTOR: usize = 2048;

pub fn populated_tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("DOCS/README.TXT", Node::file(Content::bytes(PAYLOAD)))
        .unwrap();
    tree.insert(
        "DOCS/R\u{e9}sum\u{e9}.txt",
        Node::file(Content::bytes(PAYLOAD)),
    )
    .unwrap();
    tree
}

/// An image with the ISO 9660 tree, the UDF volume, or both.
pub fn image_of(iso: bool, udf: bool, tree: &Tree) -> Vec<u8> {
    let mut dev = MemDevice::new(vec![0u8; 4 * 1024 * 1024], BlockSize::new(2048).unwrap());
    match (iso, udf) {
        (true, false) => {
            hadris_optical::iso::sync::write(
                &mut dev,
                tree,
                hadris_optical::cd::CdOptions::default().iso(),
            )
            .unwrap();
        }
        (false, true) => {
            hadris_optical::udf::sync::write(
                &mut dev,
                tree,
                &hadris_optical::udf::UdfOptions::default(),
            )
            .unwrap();
        }
        _ => {
            hadris_optical::cd::sync::write(
                &mut dev,
                tree,
                &hadris_optical::cd::CdOptions::default(),
            )
            .unwrap();
        }
    }
    dev.into_inner()
}

/// Volume descriptor identifiers at the given 2048-byte sectors.
pub fn image_with(ids: &[(usize, &[u8; 5])]) -> Vec<u8> {
    let mut image = vec![0u8; 32 * SECTOR];
    for (sector, id) in ids {
        let offset = sector * SECTOR;
        image[offset + 1..offset + 6].copy_from_slice(*id);
        image[offset + 6] = 1;
    }
    image
}

pub fn device(bytes: Vec<u8>, block: u32) -> MemDevice<Vec<u8>> {
    MemDevice::new(bytes, BlockSize::new(block).unwrap())
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

/// Whole-file reads over the bare `FileSystem` trait.
pub mod sync {
    use hadris_fs::sync::FileSystem;
    use hadris_fs::{FsResult, OpenMode, Resolve};

    /// The contents of the file at `path`.
    pub fn get<F: FileSystem>(fs: &mut F, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
        let node = fs.resolve(path.as_bytes(), Resolve::Lexical)?;
        let mut out = Vec::new();
        let mut read = fs.open(node, OpenMode::Read);
        if read.is_ok() {
            let mut chunk = [0u8; 4096];
            loop {
                match fs.read(node, out.len() as u64, &mut chunk) {
                    Ok(0) => break,
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                    Err(err) => {
                        read = Err(err);
                        break;
                    }
                }
            }
            let closed = fs.close(node);
            read = read.and(closed);
        }
        fs.forget(node, 1);
        read.map(|()| out)
    }
}

/// Whole-file reads over the bare async `FileSystem` trait.
#[cfg(feature = "async")]
pub mod asynch {
    use hadris_fs::r#async::FileSystem;
    use hadris_fs::{FsResult, OpenMode, Resolve};

    /// The contents of the file at `path`.
    pub async fn get<F: FileSystem>(fs: &mut F, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
        let node = fs.resolve(path.as_bytes(), Resolve::Lexical).await?;
        let mut out = Vec::new();
        let mut read = fs.open(node, OpenMode::Read).await;
        if read.is_ok() {
            let mut chunk = [0u8; 4096];
            loop {
                match fs.read(node, out.len() as u64, &mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                    Err(err) => {
                        read = Err(err);
                        break;
                    }
                }
            }
            let closed = fs.close(node).await;
            read = read.and(closed);
        }
        fs.forget(node, 1);
        read.map(|()| out)
    }
}
