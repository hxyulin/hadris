/// A cpio header format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// The portable ASCII format, magic `070701`: hexadecimal fields, 4-byte
    /// alignment, 32-bit sizes. What Linux initramfs uses.
    #[default]
    Newc,
    /// `newc` with the byte sum of each file's data, magic `070702`.
    NewcCrc,
    /// The old portable ASCII format, magic `070707`: octal fields, no
    /// alignment, files below 8 GiB, 18-bit inode, owner and device numbers.
    Odc,
    /// The old binary format, 16-bit words in either byte order. Read only:
    /// writing it fails with [`ErrorKind::Unsupported`](hadris_fs::ErrorKind::Unsupported).
    Binary,
}

impl Format {
    /// The largest file this format stores.
    pub const fn max_file_size(self) -> u64 {
        match self {
            Self::Newc | Self::NewcCrc | Self::Binary => u32::MAX as u64,
            Self::Odc => crate::raw::OdcHeader::max(11),
        }
    }
}

/// Options of a cpio writer.
///
/// ```rust
/// use hadris_cpio::{CpioOptions, Format};
///
/// let options = CpioOptions::default().with_format(Format::NewcCrc);
/// assert_eq!(options.format(), Format::NewcCrc);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CpioOptions {
    format: Format,
}

impl CpioOptions {
    /// The defaults: [`Format::Newc`].
    pub const fn new() -> Self {
        Self {
            format: Format::Newc,
        }
    }

    /// Sets the header format.
    pub const fn with_format(self, format: Format) -> Self {
        Self { format }
    }

    /// The header format.
    pub const fn format(&self) -> Format {
        self.format
    }
}

/// Options of a cpio reader.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ReaderOptions {
    strict_trailer: bool,
}

impl ReaderOptions {
    /// The defaults: an archive may end at an entry boundary without a
    /// trailer, as the Linux initramfs format allows.
    pub const fn new() -> Self {
        Self {
            strict_trailer: false,
        }
    }

    /// Requires a `TRAILER!!!` entry: an archive that ends without one fails
    /// with [`ErrorKind::Corrupt`](hadris_fs::ErrorKind::Corrupt) and
    /// [`Detail::Trailer`](crate::Detail::Trailer).
    pub const fn with_strict_trailer(self) -> Self {
        Self {
            strict_trailer: true,
        }
    }

    /// Whether a trailer is required.
    pub const fn strict_trailer(&self) -> bool {
        self.strict_trailer
    }
}
