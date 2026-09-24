use crate::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats, Metadata, Name,
    NodeId, OpenMode, RenameMode, Resolve, SetAttr,
};

#[cfg(feature = "contract")]
pub mod contract;
#[cfg(feature = "alloc")]
mod copy;
mod filesystem;
#[cfg(feature = "std")]
mod host;
mod paths;
#[cfg(feature = "alloc")]
mod tree;

#[cfg(feature = "alloc")]
pub use copy::copy_tree;
pub use filesystem::FileSystem;
#[cfg(feature = "std")]
sync_only! {
    pub use host::{extract_to_host, import_from_host};
}
#[cfg(feature = "alloc")]
pub use tree::{ContentReader, TreeExt};

#[cfg(feature = "std")]
sync_only! {
    mod handle;
    mod volume;
    pub use handle::{File, ReadDir};
    pub use volume::{Volume, VolumeGuard};
}

#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
async_only! {
    mod handle;
    mod volume;
    pub use handle::{File, ReadDir};
    pub use volume::{Volume, VolumeGuard};
}
