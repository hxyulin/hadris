#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::r#async as fs;
#[cfg(feature = "alloc")]
use hadris_part::r#async as part;
use hadris_storage::r#async as storage;

use hadris_fs::r#async::FileSystem;

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
