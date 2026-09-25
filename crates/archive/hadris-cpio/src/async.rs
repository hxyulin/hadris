#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::r#async as fs;
use hadris_io::r#async as io;

#[path = "read.rs"]
mod read;
pub use read::{CpioReader, Entry};
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{EntryWriter, Writer, write};
#[cfg(feature = "alloc")]
#[path = "read_tree.rs"]
mod read_tree;
#[cfg(feature = "alloc")]
pub use read_tree::read_tree;
