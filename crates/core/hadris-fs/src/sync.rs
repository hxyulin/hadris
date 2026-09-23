use hadris_io::sync as io;

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

macro_rules! sync_only {
    ($($item:tt)*) => { $($item)* };
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

impl<T> lock::Lock<T> for spin::Mutex<T> {
    type Guard<'a>
        = spin::MutexGuard<'a, T>
    where
        Self: 'a;

    fn new(value: T) -> Self {
        spin::Mutex::new(value)
    }

    fn lock(&self) -> Self::Guard<'_> {
        spin::Mutex::lock(self)
    }

    fn try_lock(&self) -> Option<Self::Guard<'_>> {
        spin::Mutex::try_lock(self)
    }

    fn get_mut(&mut self) -> &mut T {
        spin::Mutex::get_mut(self)
    }

    fn into_inner(self) -> T {
        spin::Mutex::into_inner(self)
    }
}

#[cfg(feature = "std")]
impl<T> lock::Lock<T> for std::sync::Mutex<T> {
    type Guard<'a>
        = std::sync::MutexGuard<'a, T>
    where
        Self: 'a;

    fn new(value: T) -> Self {
        std::sync::Mutex::new(value)
    }

    fn lock(&self) -> Self::Guard<'_> {
        std::sync::Mutex::lock(self).unwrap_or_else(|poison| poison.into_inner())
    }

    fn try_lock(&self) -> Option<Self::Guard<'_>> {
        match std::sync::Mutex::try_lock(self) {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::Poisoned(poison)) => Some(poison.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    fn get_mut(&mut self) -> &mut T {
        std::sync::Mutex::get_mut(self).unwrap_or_else(|poison| poison.into_inner())
    }

    fn into_inner(self) -> T {
        std::sync::Mutex::into_inner(self).unwrap_or_else(|poison| poison.into_inner())
    }
}

impl<T> lock::Lock<T> for core::cell::RefCell<T> {
    type Guard<'a>
        = core::cell::RefMut<'a, T>
    where
        Self: 'a;

    fn new(value: T) -> Self {
        core::cell::RefCell::new(value)
    }

    fn lock(&self) -> Self::Guard<'_> {
        self.borrow_mut()
    }

    fn try_lock(&self) -> Option<Self::Guard<'_>> {
        self.try_borrow_mut().ok()
    }

    fn get_mut(&mut self) -> &mut T {
        core::cell::RefCell::get_mut(self)
    }

    fn into_inner(self) -> T {
        core::cell::RefCell::into_inner(self)
    }
}

/// `std::sync::Mutex`. Poisoning is ignored: a panic inside a driver call
/// leaves the driver as it was, as a failed operation must.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct StdMutex;

#[cfg(feature = "std")]
impl lock::LockKind for StdMutex {
    type Lock<T> = std::sync::Mutex<T>;
}

/// `spin::Mutex`, for `no_std` code on several cores.
#[derive(Debug)]
pub struct Spin;

impl lock::LockKind for Spin {
    type Lock<T> = spin::Mutex<T>;
}

/// `RefCell`, for one thread. Not `Sync`; a call made while holding
/// [`Volume::lock`] panics instead of deadlocking.
#[derive(Debug)]
pub struct Local;

impl lock::LockKind for Local {
    type Lock<T> = core::cell::RefCell<T>;
}

#[cfg(feature = "std")]
impl<F: FsDriver> Volume<F, StdMutex> {
    /// Shares `driver` behind a `std::sync::Mutex`.
    pub fn new(driver: F) -> Self {
        Self::with_lock(driver)
    }
}

impl<F: FsDriver> Volume<F, Spin> {
    /// Shares `driver` behind a spin lock.
    pub fn spin(driver: F) -> Self {
        Self::with_lock(driver)
    }
}

impl<F: FsDriver> Volume<F, Local> {
    /// Shares `driver` on one thread behind a `RefCell`.
    pub fn local(driver: F) -> Self {
        Self::with_lock(driver)
    }
}
