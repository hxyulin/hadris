macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}
use hadris_fs::r#async::FileSystem;
#[allow(clippy::duplicate_mod)]
#[path = "mem_fs.rs"]
mod mem_fs;
pub use mem_fs::MemFs;

pub fn fixture() -> MemFs {
    super::fixture(MemFs::new, MemFs::add)
}
