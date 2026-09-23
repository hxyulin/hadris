macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}
macro_rules! impl_mem_fs {
    ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
}
#[allow(clippy::duplicate_mod)]
#[path = "mem_fs.rs"]
mod mem_fs;
pub use mem_fs::MemFs;

pub fn fixture() -> MemFs {
    super::fixture(MemFs::new, MemFs::add)
}
