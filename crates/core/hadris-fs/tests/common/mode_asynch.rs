macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}
macro_rules! impl_mem_fs {
    ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
}
#[allow(clippy::duplicate_mod)]
#[path = "mem_fs.rs"]
mod mem_fs;
pub use mem_fs::MemFs;

pub fn fixture() -> MemFs {
    super::fixture(MemFs::new, MemFs::add)
}
