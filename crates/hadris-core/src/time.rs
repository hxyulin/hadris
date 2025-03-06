//! This module contains types and functions for working with time.

/// A UTC time, using the chrono crate
///
/// The filesystem expects UTC time, so this type is used to represent UTC times.
pub type UtcTime = chrono::DateTime<chrono::Utc>;

/// A trait for providing UTC times. This is used to provide a time source for the filesystem.
///
/// If the `std` feature is enabled, the user can use the [`StdTimeProvider`] to provide UTC times
/// This is mainly intended for testing and no-std environments (kernels, and embedded systems).
/// If the time provided is the UNIX Epoch, the time will not be used (discarded when reaidng or
/// writing to the filesystem), signaling that the system does not support real time.
pub trait TimeProvider {
    fn now(&self) -> UtcTime;
}

/// A utility struct for providing no times to the filesystem.
pub struct NoTimeProvider;

impl NoTimeProvider {
    pub const fn new() -> Self {
        Self
    }
}

impl TimeProvider for NoTimeProvider {
    fn now(&self) -> UtcTime {
        UtcTime::UNIX_EPOCH
    }
}

/// A utility struct for providing UTC times to the filesystem.
/// It uses std::time::SystemTime to provide UTC times.
#[cfg(feature = "std")]
pub struct StdTimeProvider;

impl StdTimeProvider {
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(feature = "std")]
impl TimeProvider for StdTimeProvider {
    fn now(&self) -> UtcTime {
        let system_time = std::time::SystemTime::now();
        chrono::DateTime::<chrono::Utc>::from(system_time)
    }
}

#[cfg(feature = "std")]
pub type DefaultTimeProvider = StdTimeProvider;
#[cfg(not(feature = "std"))]
pub type DefaultTimeProvider = NoTimeProvider;

pub fn default_time_provider() -> &'static DefaultTimeProvider {
    static DEFAULT_TIME_PROVIDER: DefaultTimeProvider = DefaultTimeProvider::new();
    &DEFAULT_TIME_PROVIDER
}
