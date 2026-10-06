//! Blocking APFS readers and filesystem driver.

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

#[cfg(feature = "alloc")]
use hadris_fs::sync::FileSystem;
use hadris_storage::sync as storage;

#[path = "../container.rs"]
mod container;
pub use container::Container;

#[cfg(feature = "alloc")]
#[path = "../fs.rs"]
mod fs;
#[cfg(feature = "alloc")]
pub use fs::ApfsFs;
