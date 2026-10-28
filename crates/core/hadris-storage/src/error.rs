use core::fmt;

use hadris_io::Error;

/// A failed stream-device construction, retaining the stream for recovery.
///
/// The stream may have changed position before the seek failed. Recover it
/// with [`into_stream`](Self::into_stream), or inspect both values with
/// [`into_parts`](Self::into_parts).
/// Converting into [`Error`] or, with `std`, `std::io::Error` drops the
/// stream and preserves the failure's context and device error.
pub struct StreamDeviceError<T, E> {
    error: Error<E>,
    stream: T,
}

impl<T, E> StreamDeviceError<T, E> {
    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn new(error: Error<E>, stream: T) -> Self {
        Self { error, stream }
    }

    /// Borrows the failure, including its device error and context.
    pub const fn error(&self) -> &Error<E> {
        &self.error
    }

    /// Borrows the stream that could not be measured.
    pub const fn stream(&self) -> &T {
        &self.stream
    }

    /// Returns the stream, dropping the error.
    pub fn into_stream(self) -> T {
        self.stream
    }

    /// Returns the error, dropping the stream.
    pub fn into_error(self) -> Error<E> {
        self.error
    }

    /// Returns the failure and the stream, in that order.
    pub fn into_parts(self) -> (Error<E>, T) {
        (self.error, self.stream)
    }
}

impl<T, E: fmt::Debug> fmt::Debug for StreamDeviceError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamDeviceError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<T, E> fmt::Display for StreamDeviceError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("stream-device construction failed")
    }
}

impl<T, E: core::error::Error + 'static> core::error::Error for StreamDeviceError<T, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<T, E> From<StreamDeviceError<T, E>> for Error<E> {
    fn from(error: StreamDeviceError<T, E>) -> Self {
        error.into_error()
    }
}

#[cfg(feature = "std")]
impl<T, E: core::error::Error + Send + Sync + 'static> From<StreamDeviceError<T, E>>
    for std::io::Error
{
    fn from(error: StreamDeviceError<T, E>) -> Self {
        error.into_error().into()
    }
}
