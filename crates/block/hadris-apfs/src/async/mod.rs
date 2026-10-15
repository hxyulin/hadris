//! Unified asynchronous APFS readers for local and Send devices.
#[cfg(feature = "alloc")]
macro_rules! send_filesystem { ($($item:tt)*) => { $($item)* }; }

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::local::FileSystem;
use hadris_storage::async_ as storage;

#[path = "../container.rs"]
mod container;
pub use container::Container;

#[cfg(feature = "alloc")]
#[path = "../fs.rs"]
mod fs;
#[cfg(feature = "alloc")]
pub use fs::ApfsFs;

#[cfg(feature = "alloc")]
hadris_fs::__forward_filesystem! {
    shims ApfsFs<D: storage::BlockDevice> { resolve write }
}
