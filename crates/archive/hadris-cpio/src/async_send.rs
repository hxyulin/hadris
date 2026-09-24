#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::async_send as fs;
use hadris_io::async_send as io;

#[path = "read.rs"]
mod read;
pub use read::{CpioReader, Entry};
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{CpioWriter, write};
