//! Error types for the hadris-fat crate.

use core::fmt;

/// Errors that can occur when working with FAT file systems.
#[derive(Debug)]
pub enum FatError {
    /// Invalid boot signature (expected 0xAA55)
    InvalidBootSignature {
        /// The signature that was found
        found: u16,
    },
    /// Unsupported FAT type (FAT12/16 when FAT32 expected, or vice versa)
    UnsupportedFatType(&'static str),
    /// Invalid FSInfo signature
    InvalidFsInfoSignature {
        /// Which signature field failed validation
        field: &'static str,
        /// The expected signature value
        expected: u32,
        /// The actual signature value found
        found: u32,
    },
    /// Invalid short filename (contains disallowed characters)
    InvalidShortFilename,
    /// Cluster number out of bounds
    ClusterOutOfBounds {
        /// The cluster number that was accessed
        cluster: u32,
        /// The maximum valid cluster number
        max: u32,
    },
    /// Bad cluster marker encountered
    BadCluster {
        /// The cluster marked as bad
        cluster: u32,
    },
    /// End of cluster chain reached unexpectedly
    UnexpectedEndOfChain {
        /// The last cluster in the chain
        cluster: u32,
    },
    /// I/O error from the underlying storage
    Io(hadris_io::Error),
    /// File is not a regular file (e.g., is a directory)
    NotAFile,
    /// Entry is not a directory
    NotADirectory,
    /// Entry not found in directory
    EntryNotFound,
    /// Path is invalid (empty, malformed)
    InvalidPath,
    /// No free clusters available
    #[cfg(feature = "write")]
    NoFreeSpace,
    /// Directory is full (no free entry slots)
    #[cfg(feature = "write")]
    DirectoryFull,
    /// Filename is invalid or too long
    #[cfg(feature = "write")]
    InvalidFilename,
    /// Entry with this name already exists
    #[cfg(feature = "write")]
    AlreadyExists,
    /// Cannot delete non-empty directory
    #[cfg(feature = "write")]
    DirectoryNotEmpty,

    /// Volume is too small for the requested format
    #[cfg(feature = "write")]
    VolumeTooSmall {
        /// Requested volume size
        size: u64,
        /// Minimum required size
        min_size: u64,
    },

    /// Volume is too large for the requested FAT type
    #[cfg(feature = "write")]
    VolumeTooLarge {
        /// Requested volume size
        size: u64,
        /// Maximum supported size
        max_size: u64,
    },

    /// Invalid format option
    #[cfg(feature = "write")]
    InvalidFormatOption {
        /// The option that was invalid
        option: &'static str,
        /// The reason it was invalid
        reason: &'static str,
    },

    // exFAT-specific errors
    /// Invalid exFAT filesystem signature
    #[cfg(feature = "exfat")]
    ExFatInvalidSignature {
        /// Expected signature
        expected: [u8; 8],
        /// Found signature
        found: [u8; 8],
    },
    /// Invalid exFAT boot sector
    #[cfg(feature = "exfat")]
    ExFatInvalidBootSector {
        /// Reason for invalidity
        reason: &'static str,
    },
    /// Invalid exFAT boot region checksum
    #[cfg(feature = "exfat")]
    ExFatInvalidChecksum {
        /// Expected checksum
        expected: u32,
        /// Found checksum
        found: u32,
    },
    /// Invalid exFAT directory entry
    #[cfg(feature = "exfat")]
    ExFatInvalidEntry {
        /// Reason for invalidity
        reason: &'static str,
    },
}

impl fmt::Display for FatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBootSignature { found } => {
                write!(
                    f,
                    "invalid boot signature: expected 0xAA55, found {found:#06x}"
                )
            }
            Self::UnsupportedFatType(ty) => {
                write!(f, "unsupported FAT type: {ty}")
            }
            Self::InvalidFsInfoSignature {
                field,
                expected,
                found,
            } => {
                write!(
                    f,
                    "invalid FSInfo signature: {field} expected {expected:#010x}, found {found:#010x}"
                )
            }
            Self::InvalidShortFilename => {
                write!(f, "invalid short filename")
            }
            Self::ClusterOutOfBounds { cluster, max } => {
                write!(f, "cluster {cluster} out of bounds (max: {max})")
            }
            Self::BadCluster { cluster } => {
                write!(f, "bad cluster marker encountered at cluster {cluster}")
            }
            Self::UnexpectedEndOfChain { cluster } => {
                write!(f, "unexpected end of cluster chain at cluster {cluster}")
            }
            Self::Io(e) => {
                write!(f, "I/O error: {e:?}")
            }
            Self::NotAFile => {
                write!(f, "entry is not a file")
            }
            Self::NotADirectory => {
                write!(f, "entry is not a directory")
            }
            Self::EntryNotFound => {
                write!(f, "entry not found in directory")
            }
            Self::InvalidPath => {
                write!(f, "path is invalid (empty or malformed)")
            }
            #[cfg(feature = "write")]
            Self::NoFreeSpace => {
                write!(f, "no free clusters available")
            }
            #[cfg(feature = "write")]
            Self::DirectoryFull => {
                write!(f, "directory is full (no free entry slots)")
            }
            #[cfg(feature = "write")]
            Self::InvalidFilename => {
                write!(f, "filename is invalid or too long")
            }
            #[cfg(feature = "write")]
            Self::AlreadyExists => {
                write!(f, "entry with this name already exists")
            }
            #[cfg(feature = "write")]
            Self::DirectoryNotEmpty => {
                write!(f, "cannot delete non-empty directory")
            }
            #[cfg(feature = "write")]
            Self::VolumeTooSmall { size, min_size } => {
                write!(
                    f,
                    "volume size {} bytes is too small (minimum: {} bytes)",
                    size, min_size
                )
            }
            #[cfg(feature = "write")]
            Self::VolumeTooLarge { size, max_size } => {
                write!(
                    f,
                    "volume size {} bytes is too large (maximum: {} bytes)",
                    size, max_size
                )
            }
            #[cfg(feature = "write")]
            Self::InvalidFormatOption { option, reason } => {
                write!(f, "invalid format option '{}': {}", option, reason)
            }
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidSignature { expected, found } => {
                write!(
                    f,
                    "invalid exFAT signature: expected {:?}, found {:?}",
                    core::str::from_utf8(expected).unwrap_or("<invalid>"),
                    core::str::from_utf8(found).unwrap_or("<invalid>")
                )
            }
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidBootSector { reason } => {
                write!(f, "invalid exFAT boot sector: {reason}")
            }
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidChecksum { expected, found } => {
                write!(
                    f,
                    "invalid exFAT checksum: expected {expected:#010x}, found {found:#010x}"
                )
            }
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidEntry { reason } => {
                write!(f, "invalid exFAT directory entry: {reason}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FatError {}

impl From<hadris_io::Error> for FatError {
    fn from(e: hadris_io::Error) -> Self {
        Self::Io(e)
    }
}

/// Result type alias for FAT operations.
pub type Result<T> = core::result::Result<T, FatError>;
