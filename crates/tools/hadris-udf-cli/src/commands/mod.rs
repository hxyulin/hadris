//! Command implementations for hadris-udf CLI

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

use hadris_fs::MountOptions;
use std::io::Write;
use std::path::Path;

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, DirEntry, FileType, OpenMode, Resolve};
use hadris_storage::host::FileDevice;
use hadris_udf::sync::UdfFs;

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) type Udf = UdfFs<FileDevice>;

/// Opens the UDF volume in the image at `path`.
pub(super) fn open(path: &Path) -> Result<Udf> {
    Ok(UdfFs::mount(FileDevice::open(path)?, MountOptions::new())?)
}

/// The entries of the directory at `path`, sorted as the volume lists them.
pub(super) fn entries(udf: &mut Udf, path: &str) -> Result<Vec<DirEntry>> {
    let dir = udf.resolve(path.as_bytes(), Resolve::Lexical)?;
    let mut items = Vec::new();
    let mut cursor = DirCursor::START;
    let listed = loop {
        match udf.readdir(dir, cursor) {
            Ok(Some(entry)) => {
                cursor = entry.next_cursor();
                items.push(entry);
            }
            Ok(None) => break Ok(items),
            Err(err) => break Err(err.into()),
        }
    };
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

fn copy_node(udf: &mut Udf, node: hadris_fs::NodeId, out: &mut impl Write) -> Result<u64> {
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

/// `/`-joined image path of `name` in `dir`.
pub(super) fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

pub(super) fn type_char(file_type: FileType) -> char {
    match file_type {
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        FileType::CharDevice => 'c',
        FileType::BlockDevice => 'b',
        FileType::Fifo => 'p',
        FileType::Socket => 's',
        _ => '-',
    }
}
