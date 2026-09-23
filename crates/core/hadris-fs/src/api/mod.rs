use super::io;
use super::lock::{Lock, LockKind};
use crate::forget_queue::ForgetQueue;
use crate::path::{Component, Components, VPath};
use crate::{
    Capabilities, DirCursor, DirEntry, DirItem, Error, ErrorKind, FileType, FsResult, FsStats,
    Metadata, Name, NameBuf, NewNode, NodeId, OpenOptions, RenameFlags, SetMetadata,
};
use hadris_io::SeekFrom;

#[cfg(feature = "alloc")]
mod copy;
#[macro_use]
mod driver;
mod handle;
#[cfg(feature = "std")]
mod host;
mod paths;
mod resolve;
mod volume;

#[cfg(feature = "alloc")]
pub use copy::copy_tree;
pub use driver::{AsDriver, FileSystem, FsDriver};
pub use handle::{Access, Dir, File, OpenFile};
#[cfg(feature = "std")]
sync_only! {
    pub use host::{extract_to_host, import_from_host};
}
use paths::open_node;
pub use paths::{DriverExt, PathExt};
#[cfg(feature = "alloc")]
use paths::{create_dir_all, resolve_parent};
pub use resolve::{Lexical, Posix, Resolver, WithResolver};
pub use volume::Volume;
