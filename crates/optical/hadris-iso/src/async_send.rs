#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::async_send as fs;
#[cfg(feature = "alloc")]
use hadris_part::async_send as part;
use hadris_storage::async_send as storage;

macro_rules! impl_iso_driver {
    ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async_send, $($t)*); };
}

#[path = "image.rs"]
mod image;
pub use image::{IsoImage, IsoView};
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{plan, write};
#[cfg(feature = "alloc")]
#[path = "session.rs"]
mod session;
#[cfg(feature = "alloc")]
pub use session::Session;
