#[allow(unused_macros)]
macro_rules! sync_only {
    ($($item:tt)*) => {};
}
#[allow(unused_macros)]
macro_rules! async_only { ($($item:tt)*) => { $($item)* }; }

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::local::FileSystem;
use hadris_storage::async_ as storage;

#[allow(clippy::duplicate_mod)]
#[path = "image.rs"]
pub(crate) mod image;
pub use image::IsoFs;

hadris_fs::__forward_filesystem! {
    shims IsoFs<D: storage::BlockDevice> { read resolve write }
}

hadris_fs::__forward_filesystem! {
    #[cfg(feature = "async")]
    send IsoFs<D: hadris_storage::async_::SendBlockDevice> as hadris_fs::async_::FileSystem { read }
}

#[cfg(feature = "alloc")]
use hadris_fs::local as fs;
#[cfg(feature = "alloc")]
use hadris_part::async_ as part;
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::write;
#[cfg(feature = "alloc")]
#[path = "session.rs"]
mod session;
#[cfg(feature = "alloc")]
pub use session::Session;
