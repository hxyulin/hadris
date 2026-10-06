use hadris_fs::DateTime;

/// A cpio header format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// The portable ASCII format, magic `070701`: hexadecimal fields, 4-byte
    /// alignment, 32-bit sizes. What Linux initramfs uses.
    #[default]
    Newc,
    /// `newc` with the byte sum of each file's data, magic `070702`.
    Crc,
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
            Self::Newc | Self::Crc | Self::Binary => u32::MAX as u64,
            Self::Odc => crate::raw::OdcHeader::max(11),
        }
    }
}

/// Options of a cpio writer.
///
/// ```rust
/// use hadris_cpio::{CpioOptions, Format};
///
/// let options = CpioOptions::default().with_format(Format::Crc);
/// assert_eq!(options.format(), Format::Crc);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CpioOptions {
    format: Format,
    time: Option<DateTime>,
}

impl CpioOptions {
    /// The defaults: [`Format::Newc`], and modification time 0 for entries
    /// that set none.
    pub const fn new() -> Self {
        Self {
            format: Format::Newc,
            time: None,
        }
    }

    /// Sets the header format.
    pub const fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// Sets the modification time of entries whose attributes set none,
    /// such as `SOURCE_DATE_EPOCH`. The writer reads no clock.
    pub const fn with_time(mut self, time: DateTime) -> Self {
        self.time = Some(time);
        self
    }

    /// The header format.
    pub const fn format(&self) -> Format {
        self.format
    }

    /// The modification time of entries that set none, if set.
    pub const fn time(&self) -> Option<DateTime> {
        self.time
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
