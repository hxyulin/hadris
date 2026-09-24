use core::fmt;

use hadris_fs::ErrorKind;

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure or
/// option to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The Volume Recognition Sequence has no NSR descriptor inside an
    /// extended area: the device holds no UDF volume.
    RecognitionSequence,
    /// No Anchor Volume Descriptor Pointer is valid.
    Anchor,
    /// Neither volume descriptor sequence holds a primary, a logical volume
    /// and a partition descriptor.
    DescriptorSequence,
    /// A descriptor tag is wrong: its checksum, CRC, identifier, version or
    /// location.
    Descriptor,
    /// The logical block size is not a power of two from 512 to 4096, it
    /// disagrees with the anchor, or the device block is larger.
    BlockSize,
    /// A partition map other than type 1 (virtual, sparable or metadata
    /// partitions).
    PartitionMap,
    /// A partition reference names no partition, or an extent runs past its
    /// partition.
    Partition,
    /// The File Set Descriptor is invalid.
    FileSet,
    /// A File Entry or Extended File Entry is invalid.
    Icb,
    /// A list of allocation descriptors is invalid, too long, or ends
    /// before the file.
    AllocationDescriptor,
    /// A File Identifier Descriptor is invalid.
    FileIdentifier,
    /// A symbolic link's path components are invalid.
    PathComponent,
    /// A structure lies outside the device.
    OutsideImage,
    /// A volume identifier does not fit its field.
    Identifier,
    /// The image would exceed the 32-bit block numbers of UDF.
    ImageTooLarge,
    /// A time lies outside the years 1 to 9999 a timestamp holds.
    Timestamp,
    /// Reading the content of a file from the tree failed; see
    /// [`Error::content_error`].
    Content,
    /// A file's content is extents on the device: a standalone volume
    /// cannot use them, and in a bridge volume they must be whole aligned
    /// blocks after the UDF structures.
    StoredContent,
    /// The output device's block size does not divide 2048.
    OutputBlockSize,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::RecognitionSequence => "no NSR descriptor in the volume recognition sequence",
            Self::Anchor => "no valid anchor volume descriptor pointer",
            Self::DescriptorSequence => "incomplete volume descriptor sequence",
            Self::Descriptor => "invalid descriptor tag",
            Self::BlockSize => "unsupported logical or device block size",
            Self::PartitionMap => "unsupported partition map",
            Self::Partition => "a partition reference or extent is out of bounds",
            Self::FileSet => "invalid file set descriptor",
            Self::Icb => "invalid file entry",
            Self::AllocationDescriptor => "invalid allocation descriptors",
            Self::FileIdentifier => "invalid file identifier descriptor",
            Self::PathComponent => "invalid symbolic link path components",
            Self::OutsideImage => "a structure lies outside the device",
            Self::Identifier => "volume identifier too long",
            Self::ImageTooLarge => "image exceeds 32-bit block numbers",
            Self::Timestamp => "time outside the years 1 to 9999",
            Self::Content => "reading file content failed",
            Self::StoredContent => "stored content does not fit the volume",
            Self::OutputBlockSize => "output block size does not divide 2048",
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// Error of reading or writing a UDF volume on a device with error `E`.
///
/// Like [`hadris_fs::Error`], it keeps the device's own error without
/// allocation. Callers match on [`kind`](Self::kind); [`detail`](Self::detail)
/// names the structure or option. Two errors are equal when their kinds and
/// device errors are.
///
/// `?` converts it into [`hadris_fs::Error<E>`], and with `std` into
/// [`std::io::Error`], returning an `io::Error` device error as itself.
#[derive(Debug)]
pub struct Error<E> {
    kind: ErrorKind,
    detail: Option<Detail>,
    device: Option<E>,
    #[cfg(feature = "alloc")]
    content: Option<hadris_fs::AnyError>,
}

impl<E> Error<E> {
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail: Some(detail),
            device: None,
            #[cfg(feature = "alloc")]
            content: None,
        }
    }

    pub(crate) const fn device(err: E) -> Self {
        Self {
            kind: ErrorKind::Io,
            detail: None,
            device: Some(err),
            #[cfg(feature = "alloc")]
            content: None,
        }
    }

    pub(crate) const fn corrupt(detail: Detail) -> Self {
        Self::new(ErrorKind::Corrupt, detail)
    }

    pub(crate) const fn invalid(detail: Detail) -> Self {
        Self::new(ErrorKind::InvalidInput, detail)
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn content(err: hadris_fs::AnyError) -> Self {
        Self {
            kind: err.kind(),
            detail: Some(Detail::Content),
            device: None,
            content: Some(err),
        }
    }

    /// What went wrong. [`ErrorKind::Io`] when the device failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which structure or option it concerns, when known.
    pub const fn detail(&self) -> Option<Detail> {
        self.detail
    }

    /// The device error, if the device failed.
    pub const fn device_error(&self) -> Option<&E> {
        self.device.as_ref()
    }

    /// Takes the device error, if the device failed.
    pub fn into_device_error(self) -> Option<E> {
        self.device
    }

    /// The error of reading a file's content from the tree, for
    /// [`Detail::Content`].
    #[cfg(feature = "alloc")]
    pub fn content_error(&self) -> Option<&hadris_fs::AnyError> {
        self.content.as_ref()
    }

    /// Converts the device error.
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F> {
        Error {
            kind: self.kind,
            detail: self.detail,
            device: self.device.map(f),
            #[cfg(feature = "alloc")]
            content: self.content,
        }
    }
}

impl<E: PartialEq> PartialEq for Error<E> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.device == other.device
    }
}

impl<E: Eq> Eq for Error<E> {}

impl<E> From<ErrorKind> for Error<E> {
    fn from(kind: ErrorKind) -> Self {
        Self {
            kind,
            detail: None,
            device: None,
            #[cfg(feature = "alloc")]
            content: None,
        }
    }
}

impl<E> From<hadris_fs::Error<E>> for Error<E> {
    fn from(err: hadris_fs::Error<E>) -> Self {
        let kind = err.kind();
        Self {
            kind,
            detail: None,
            device: err.into_device_error(),
            #[cfg(feature = "alloc")]
            content: None,
        }
    }
}

#[cfg(feature = "alloc")]
impl<E> From<hadris_fs::tree::TreeError> for Error<E> {
    fn from(err: hadris_fs::tree::TreeError) -> Self {
        err.kind().into()
    }
}

impl<E> From<hadris_storage::WriteError<E>> for Error<E> {
    fn from(err: hadris_storage::WriteError<E>) -> Self {
        hadris_fs::Error::from(err).into()
    }
}

impl<E> From<Error<E>> for hadris_fs::Error<E> {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(device) => hadris_fs::Error::from_device(device),
            None => err.kind.into(),
        }
    }
}

#[cfg(feature = "alloc")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for hadris_fs::AnyError {
    fn from(err: Error<E>) -> Self {
        match err.content {
            Some(content) => content,
            None => hadris_fs::Error::from(err).into(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(device) = &self.device {
            return write!(f, "device error: {device}");
        }
        #[cfg(feature = "alloc")]
        if let Some(content) = &self.content {
            return write!(f, "reading file content failed: {content}");
        }
        match self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.kind),
            None => self.kind.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        if let Some(device) = &self.device {
            return Some(device);
        }
        #[cfg(feature = "alloc")]
        if let Some(content) = &self.content {
            return Some(content);
        }
        None
    }
}

/// A device error that is a `std::io::Error` comes back as itself.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for std::io::Error {
    fn from(err: Error<E>) -> Self {
        if err.device.is_some() {
            return hadris_fs::Error::from(err).into();
        }
        if let Some(content) = err.content {
            return content.into();
        }
        std::io::Error::new(err.kind.into(), StdMessage(err.kind, err.detail))
    }
}

#[cfg(feature = "std")]
#[derive(Debug)]
struct StdMessage(ErrorKind, Option<Detail>);

#[cfg(feature = "std")]
impl fmt::Display for StdMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.1 {
            Some(detail) => write!(f, "{}: {detail}", self.0),
            None => self.0.fmt(f),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StdMessage {}
