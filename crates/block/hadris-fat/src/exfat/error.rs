//! Errors of the exFAT preview.

use core::fmt;

/// Errors that can occur when working with exFAT volumes.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Invalid boot signature (expected 0xAA55)
    InvalidBootSignature {
        /// The signature that was found
        found: u16,
    },
    /// The volume uses a layout this preview does not support.
    UnsupportedFatType(&'static str),
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
    /// A chain walk visited more clusters than the volume contains.
    ClusterLoop {
        /// The cluster the walker was inspecting when it hit the limit.
        cluster: u32,
    },
    /// I/O error from the underlying storage
    Io(hadris_io::legacy::Error),
    /// File is not a regular file (e.g., is a directory)
    NotAFile,
    /// Entry is not a directory
    NotADirectory,
    /// Entry not found in directory
    EntryNotFound,
    /// Path is invalid (empty, malformed)
    InvalidPath,
    /// No free clusters available
    NoFreeSpace,
    /// Directory is full (no free entry slots)
    DirectoryFull,
    /// Filename is invalid or too long
    InvalidFilename,
    /// Entry with this name already exists
    AlreadyExists,
    /// Cannot delete non-empty directory
    DirectoryNotEmpty,
    /// Volume is too small for the requested format
    VolumeTooSmall {
        /// Requested volume size
        size: u64,
        /// Minimum required size
        min_size: u64,
    },
    /// Volume is too large for the requested format
    VolumeTooLarge {
        /// Requested volume size
        size: u64,
        /// Maximum supported size
        max_size: u64,
    },
    /// Invalid format option
    InvalidFormatOption {
        /// The option that was invalid
        option: &'static str,
        /// The reason it was invalid
        reason: &'static str,
    },
    /// Invalid exFAT filesystem signature
    ExFatInvalidSignature {
        /// Expected signature
        expected: [u8; 8],
        /// Found signature
        found: [u8; 8],
    },
    /// Invalid exFAT boot sector
    ExFatInvalidBootSector {
        /// Reason for invalidity
        reason: &'static str,
    },
    /// Invalid exFAT boot region checksum
    ExFatInvalidChecksum {
        /// Expected checksum
        expected: u32,
        /// Found checksum
        found: u32,
    },
    /// Invalid exFAT directory entry
    ExFatInvalidEntry {
        /// Reason for invalidity
        reason: &'static str,
    },
}

/// Result type of the exFAT preview.
pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBootSignature { found } => write!(
                f,
                "invalid boot signature: expected 0xAA55, found {found:#06x}"
            ),
            Self::UnsupportedFatType(ty) => write!(f, "unsupported exFAT layout: {ty}"),
            Self::ClusterOutOfBounds { cluster, max } => {
                write!(f, "cluster {cluster} out of bounds (max: {max})")
            }
            Self::BadCluster { cluster } => {
                write!(f, "bad cluster marker encountered at cluster {cluster}")
            }
            Self::UnexpectedEndOfChain { cluster } => {
                write!(f, "unexpected end of cluster chain at cluster {cluster}")
            }
            Self::ClusterLoop { cluster } => {
                write!(f, "cluster chain loop detected at cluster {cluster}")
            }
            Self::Io(e) => write!(f, "I/O error: {e:?}"),
            Self::NotAFile => f.write_str("entry is not a file"),
            Self::NotADirectory => f.write_str("entry is not a directory"),
            Self::EntryNotFound => f.write_str("entry not found in directory"),
            Self::InvalidPath => f.write_str("path is invalid (empty or malformed)"),
            Self::NoFreeSpace => f.write_str("no free clusters available"),
            Self::DirectoryFull => f.write_str("directory is full (no free entry slots)"),
            Self::InvalidFilename => f.write_str("filename is invalid or too long"),
            Self::AlreadyExists => f.write_str("entry with this name already exists"),
            Self::DirectoryNotEmpty => f.write_str("cannot delete non-empty directory"),
            Self::VolumeTooSmall { size, min_size } => write!(
                f,
                "volume size {size} bytes is too small (minimum: {min_size} bytes)"
            ),
            Self::VolumeTooLarge { size, max_size } => write!(
                f,
                "volume size {size} bytes is too large (maximum: {max_size} bytes)"
            ),
            Self::InvalidFormatOption { option, reason } => {
                write!(f, "invalid format option '{option}': {reason}")
            }
            Self::ExFatInvalidSignature { expected, found } => write!(
                f,
                "invalid exFAT signature: expected {:?}, found {:?}",
                core::str::from_utf8(expected).unwrap_or("<invalid>"),
                core::str::from_utf8(found).unwrap_or("<invalid>")
            ),
            Self::ExFatInvalidBootSector { reason } => {
                write!(f, "invalid exFAT boot sector: {reason}")
            }
            Self::ExFatInvalidChecksum { expected, found } => write!(
                f,
                "invalid exFAT checksum: expected {expected:#010x}, found {found:#010x}"
            ),
            Self::ExFatInvalidEntry { reason } => {
                write!(f, "invalid exFAT directory entry: {reason}")
            }
        }
    }
}

impl core::error::Error for Error {}

impl From<hadris_io::legacy::Error> for Error {
    fn from(e: hadris_io::legacy::Error) -> Self {
        Self::Io(e)
    }
}
