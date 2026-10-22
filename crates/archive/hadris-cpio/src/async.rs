#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "write")]
use hadris_fs::async_ as fs;
use hadris_io::async_ as io;

#[path = "read.rs"]
mod read;
pub use read::{CpioReader, Entry};
#[cfg(feature = "write")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "write")]
pub use write::{EntryWriter, Writer, write};
#[cfg(feature = "alloc")]
#[path = "read_tree.rs"]
mod read_tree;
#[cfg(feature = "alloc")]
pub use read_tree::read_tree;
