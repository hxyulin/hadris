#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::r#async as fs;
#[cfg(feature = "alloc")]
use hadris_part::r#async as part;
#[cfg(feature = "alloc")]
use hadris_storage::r#async as storage;

#[cfg(feature = "alloc")]
use hadris_fs::r#async::FileSystem;

pub(crate) use crate::async_::image;
pub use image::IsoFs;
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
