#[allow(unused_macros)]
macro_rules! send_only {
    ($($item:tt)*) => {};
}

#[allow(unused_imports)]
use hadris_io::local as io;
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[allow(unused_macros)]
macro_rules! sync_only {
    ($($item:tt)*) => {};
}

#[allow(unused_macros)]
macro_rules! async_only {
    ($($item:tt)*) => { $($item)* };
}

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
pub use api::*;

#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
#[allow(clippy::duplicate_mod)]
#[path = "async_lock.rs"]
mod lock;
