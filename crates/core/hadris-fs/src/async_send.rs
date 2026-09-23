use hadris_io::async_send as io;

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

macro_rules! sync_only {
    ($($item:tt)*) => {};
}

#[allow(unused_macros)]
macro_rules! async_only {
    ($($item:tt)*) => { $($item)* };
}

#[allow(unused_macros)]
macro_rules! local_only {
    ($($item:tt)*) => {};
}

/// Locks for [`Volume`] whose futures and guards are `Send`.
#[path = "lock_send.rs"]
pub mod lock;

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
pub use api::*;

#[cfg(feature = "alloc")]
impl<T: Send> lock::Lock<T> for async_lock::Mutex<T> {
    fn new(value: T) -> Self {
        async_lock::Mutex::new(value)
    }

    async fn lock(&self) -> impl core::ops::DerefMut<Target = T> + Send + '_ {
        async_lock::Mutex::lock(self).await
    }

    fn try_lock(&self) -> Option<impl core::ops::DerefMut<Target = T> + Send + '_> {
        async_lock::Mutex::try_lock(self)
    }

    fn get_mut(&mut self) -> &mut T {
        async_lock::Mutex::get_mut(self)
    }

    fn into_inner(self) -> T {
        async_lock::Mutex::into_inner(self)
    }
}

#[cfg(feature = "alloc")]
pub use super::r#async::AsyncMutex;

#[cfg(feature = "alloc")]
impl lock::LockKind for AsyncMutex {
    type Lock<T: Send> = async_lock::Mutex<T>;
}

#[cfg(feature = "alloc")]
impl<F: FsDriver> Volume<F, AsyncMutex> {
    /// Shares `driver` behind an async mutex.
    pub fn new(driver: F) -> Self {
        Self::with_lock(driver)
    }
}
