macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}
use hadris_fs::sync::FileSystem;
#[allow(clippy::duplicate_mod)]
#[path = "mem_fs.rs"]
mod mem_fs;
pub use mem_fs::MemFs;

pub fn fixture() -> MemFs {
    super::fixture(MemFs::new, MemFs::add)
}
