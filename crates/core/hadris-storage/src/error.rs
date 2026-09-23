use core::fmt;

/// Failure of a block write.
///
/// A device without write support keeps the default
/// [`write_blocks`](crate::sync::BlockDevice::write_blocks), which returns
/// [`ReadOnly`](Self::ReadOnly), so read-only devices implement no write
/// method. There is no query for whether a device is writable: each write
/// answers for itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteError<E> {
    /// The device does not accept writes.
    ReadOnly,
    /// The device failed.
    Device(E),
}

impl<E> WriteError<E> {
    /// Converts the device error.
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> WriteError<F> {
        match self {
            Self::ReadOnly => WriteError::ReadOnly,
            Self::Device(err) => WriteError::Device(f(err)),
        }
    }
}

impl<E> From<E> for WriteError<E> {
    fn from(err: E) -> Self {
        Self::Device(err)
    }
}

impl<E: fmt::Display> fmt::Display for WriteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => f.write_str("device is read-only"),
            Self::Device(err) => err.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for WriteError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Device(err) => Some(err),
            Self::ReadOnly => None,
        }
    }
}

/// A block request outside the device, or not a whole number of blocks.
///
/// The error of [`MemDevice`](crate::MemDevice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("block request is out of range")
    }
}

impl core::error::Error for OutOfRange {}

/// Error of the adapters that can refuse a request themselves: the refusal,
/// or the error of the device or stream underneath.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageError<E> {
    /// The request lies outside the device, or a block buffer is not a whole
    /// number of blocks.
    OutOfRange,
    /// The stream ended inside a block.
    UnexpectedEof,
    /// The stream accepted no bytes.
    WriteZero,
    /// The device does not accept writes.
    ReadOnly,
    /// The block size is larger than the adapter's buffer (`ByteView`
    /// without `alloc`).
    BlockTooLarge,
    /// The device or stream failed.
    Device(E),
}

impl<E> StorageError<E> {
    /// The device error, if the device failed.
    pub fn device_error(&self) -> Option<&E> {
        match self {
            Self::Device(err) => Some(err),
            _ => None,
        }
    }

    /// Takes the device error, if the device failed.
    pub fn into_device_error(self) -> Option<E> {
        match self {
            Self::Device(err) => Some(err),
            _ => None,
        }
    }
}

impl<E> From<OutOfRange> for StorageError<E> {
    fn from(_: OutOfRange) -> Self {
        Self::OutOfRange
    }
}

impl<E> From<hadris_io::ExactError<E>> for StorageError<E> {
    fn from(err: hadris_io::ExactError<E>) -> Self {
        match err {
            hadris_io::ExactError::UnexpectedEof => Self::UnexpectedEof,
            hadris_io::ExactError::WriteZero => Self::WriteZero,
            hadris_io::ExactError::Io(err) => Self::Device(err),
            _ => Self::UnexpectedEof,
        }
    }
}

impl<E> From<WriteError<E>> for StorageError<E> {
    fn from(err: WriteError<E>) -> Self {
        match err {
            WriteError::ReadOnly => Self::ReadOnly,
            WriteError::Device(err) => Self::Device(err),
        }
    }
}

impl<E> From<WriteError<StorageError<E>>> for StorageError<E> {
    fn from(err: WriteError<StorageError<E>>) -> Self {
        match err {
            WriteError::ReadOnly => Self::ReadOnly,
            WriteError::Device(err) => err,
        }
    }
}

impl<E: fmt::Display> fmt::Display for StorageError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange => OutOfRange.fmt(f),
            Self::UnexpectedEof => f.write_str("stream ended inside a block"),
            Self::WriteZero => f.write_str("stream accepted no bytes"),
            Self::ReadOnly => f.write_str("device is read-only"),
            Self::BlockTooLarge => f.write_str("block size exceeds the adapter buffer"),
            Self::Device(err) => err.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for StorageError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.device_error().map(|err| err as _)
    }
}

#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<WriteError<E>> for std::io::Error {
    fn from(err: WriteError<E>) -> Self {
        match err {
            WriteError::ReadOnly => std::io::ErrorKind::ReadOnlyFilesystem.into(),
            WriteError::Device(err) => hadris_io::into_std_error(err),
        }
    }
}

#[cfg(feature = "std")]
impl From<OutOfRange> for std::io::Error {
    fn from(err: OutOfRange) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, err)
    }
}

/// A device error that is a `std::io::Error` comes back as itself.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<StorageError<E>> for std::io::Error {
    fn from(err: StorageError<E>) -> Self {
        use std::io::ErrorKind as Io;
        let kind = match err {
            StorageError::Device(err) => return hadris_io::into_std_error(err),
            StorageError::OutOfRange => Io::InvalidInput,
            StorageError::UnexpectedEof => Io::UnexpectedEof,
            StorageError::WriteZero => Io::WriteZero,
            StorageError::ReadOnly => Io::ReadOnlyFilesystem,
            StorageError::BlockTooLarge => Io::Unsupported,
        };
        std::io::Error::new(kind, err)
    }
}
