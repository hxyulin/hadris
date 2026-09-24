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

use std::path::Path;

use hadris_fs::sync::DriverExt;
use hadris_fs::{DirItem, FileType};
use hadris_storage::host::FileDevice;
use hadris_udf::sync::UdfFs;

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) type Udf = UdfFs<FileDevice>;

/// Opens the UDF volume in the image at `path`.
pub(super) fn open(path: &Path) -> Result<Udf> {
    Ok(UdfFs::open(FileDevice::open(path)?)?)
}

/// The entries of the directory at `path`, sorted as the volume lists them.
pub(super) fn entries(udf: &mut Udf, path: &str) -> Result<Vec<DirItem>> {
    let mut items = Vec::new();
    for item in udf.read_dir(path)? {
        items.push(item?);
    }
    Ok(items)
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
