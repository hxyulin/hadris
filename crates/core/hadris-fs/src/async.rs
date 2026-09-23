use hadris_io::r#async as io;

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

macro_rules! sync_only {
    ($($item:tt)*) => {};
}

#[allow(unused_macros)]
macro_rules! local_only {
    ($($item:tt)*) => { $($item)* };
}

/// Locks for [`Volume`].
#[allow(clippy::duplicate_mod)]
#[path = "lock.rs"]
pub mod lock;

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
pub use api::*;

#[cfg(feature = "alloc")]
impl<T> lock::Lock<T> for async_lock::Mutex<T> {
    type Guard<'a>
        = async_lock::MutexGuard<'a, T>
    where
        Self: 'a;

    fn new(value: T) -> Self {
        async_lock::Mutex::new(value)
    }

    async fn lock(&self) -> Self::Guard<'_> {
        async_lock::Mutex::lock(self).await
    }

    fn try_lock(&self) -> Option<Self::Guard<'_>> {
        async_lock::Mutex::try_lock(self)
    }

    fn get_mut(&mut self) -> &mut T {
        async_lock::Mutex::get_mut(self)
    }

    fn into_inner(self) -> T {
        async_lock::Mutex::into_inner(self)
    }
}

/// `async_lock::Mutex`, safe to hold across `.await`. Needs `alloc`, which
/// `async-lock` links even without `std`.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct AsyncMutex;

#[cfg(feature = "alloc")]
impl lock::LockKind for AsyncMutex {
    type Lock<T> = async_lock::Mutex<T>;
}

#[cfg(feature = "alloc")]
impl<F: FsDriver> Volume<F, AsyncMutex> {
    /// Shares `driver` behind an async mutex.
    pub fn new(driver: F) -> Self {
        Self::with_lock(driver)
    }
}

#[cfg(feature = "embassy-sync")]
type EmbassyMutex<T> =
    embassy_sync::mutex::Mutex<embassy_sync::blocking_mutex::raw::NoopRawMutex, T>;

#[cfg(feature = "embassy-sync")]
impl<T> lock::Lock<T> for EmbassyMutex<T> {
    type Guard<'a>
        = embassy_sync::mutex::MutexGuard<'a, embassy_sync::blocking_mutex::raw::NoopRawMutex, T>
    where
        Self: 'a;

    fn new(value: T) -> Self {
        embassy_sync::mutex::Mutex::new(value)
    }

    async fn lock(&self) -> Self::Guard<'_> {
        embassy_sync::mutex::Mutex::lock(self).await
    }

    fn try_lock(&self) -> Option<Self::Guard<'_>> {
        embassy_sync::mutex::Mutex::try_lock(self).ok()
    }

    fn get_mut(&mut self) -> &mut T {
        embassy_sync::mutex::Mutex::get_mut(self)
    }

    fn into_inner(self) -> T {
        embassy_sync::mutex::Mutex::into_inner(self)
    }
}

/// An `embassy-sync` mutex for tasks on one executor thread, with no
/// allocator. Not `Sync`.
#[cfg(feature = "embassy-sync")]
#[derive(Debug)]
pub struct Local;

#[cfg(feature = "embassy-sync")]
impl lock::LockKind for Local {
    type Lock<T> = EmbassyMutex<T>;
}

#[cfg(feature = "embassy-sync")]
impl<F: FsDriver> Volume<F, Local> {
    /// Shares `driver` between tasks on one executor thread.
    pub fn local(driver: F) -> Self {
        Self::with_lock(driver)
    }
}
