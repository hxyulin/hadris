//! Command implementations for `hadris udf`.

mod cat;
mod create;
mod extract;
mod info;
mod ls;
mod tree;
mod verify;

pub use cat::cat;
pub use create::create;
pub use extract::extract;
pub use info::info;
pub use ls::ls;
pub use tree::tree;
pub use verify::verify;

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirEntry, MountOptions, NodeId, OpenMode, Resolve};
use hadris_udf::sync::UdfFs;

use crate::common::{Device, Locate};
use crate::vfs::entries as list;

pub(super) type Udf = UdfFs<Device>;

/// Opens the UDF volume `locate` selects in the image at `path`.
pub(super) fn open(path: &Path, locate: &Locate) -> Result<Udf> {
    Ok(UdfFs::mount(locate.open(path)?, MountOptions::new())?)
}

/// The entries of the directory at `path`, sorted as the volume lists them.
pub(super) fn entries(udf: &mut Udf, path: &str) -> Result<Vec<DirEntry>> {
    let dir = udf.resolve(path.as_bytes(), Resolve::Lexical)?;
    let listed = list(udf, dir);
    udf.forget(dir, 1);
    listed
}

/// Copies the file at `path` to `out` and returns its length.
pub(super) fn copy_file(udf: &mut Udf, path: &str, out: &mut impl Write) -> Result<u64> {
    let node = udf.resolve(path.as_bytes(), Resolve::Lexical)?;
    let copied = copy_node(udf, node, out);
    udf.forget(node, 1);
    copied
}

pub(super) fn copy_node(udf: &mut Udf, node: NodeId, out: &mut impl Write) -> Result<u64> {
    udf.open(node, OpenMode::Read)?;
    let mut buf = [0u8; 64 * 1024];
    let mut offset = 0u64;
    let copied: Result<u64> = loop {
        match udf.read(node, offset, &mut buf) {
            Ok(0) => break Ok(offset),
            Ok(n) => {
                if let Err(err) = out.write_all(&buf[..n]) {
                    break Err(err.into());
                }
                offset += n as u64;
            }
            Err(err) => break Err(err.into()),
        }
    };
    let closed = udf.close(node);
    let len = copied?;
    closed?;
    Ok(len)
}
