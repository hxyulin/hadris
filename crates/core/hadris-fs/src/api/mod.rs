#[cfg(feature = "unstable-contract")]
#[doc(hidden)]
pub mod contract;
#[cfg(feature = "alloc")]
sync_only! { mod copy; pub use copy::copy_tree; }
#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
async_only! { #[path = "copy_async.rs"] mod copy; pub use copy::copy_tree; }
mod filesystem;
mod paths;
#[cfg(feature = "alloc")]
mod tree;
mod walk;

pub use filesystem::FileSystem;
#[cfg(feature = "alloc")]
pub use tree::ContentReader;
pub use walk::Walk;

#[cfg(feature = "std")]
sync_only! {
    mod handle;
    mod read_tree;
    mod volume;
    pub use handle::{File, ReadDir, WalkDir, WalkDirEntry};
    pub use read_tree::read_tree;
    pub use volume::{Volume, VolumeGuard};
}

#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
async_only! {
    mod handle;
    mod read_tree;
    mod volume;
    pub use handle::{File, ReadDir, WalkDir, WalkDirEntry};
    pub use read_tree::read_tree;
    pub use volume::{Volume, VolumeGuard};
}
