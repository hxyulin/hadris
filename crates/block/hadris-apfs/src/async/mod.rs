//! Asynchronous APFS readers and filesystem driver.

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::r#async::FileSystem;
use hadris_storage::r#async as storage;

#[path = "../container.rs"]
mod container;
pub use container::Container;

#[cfg(feature = "alloc")]
#[path = "../fs.rs"]
mod fs;
#[cfg(feature = "alloc")]
pub use fs::ApfsFs;
