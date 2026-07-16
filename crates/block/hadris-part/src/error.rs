//! Error types for partition operations.

use core::fmt::{self, Debug, Display};

/// Errors that can occur during partition table operations.
#[derive(Debug)]
pub enum PartitionError {
    /// An I/O error occurred.
    Io(hadris_io::Error),

    /// The MBR signature (0x55AA) is invalid.
    InvalidMbrSignature {
        /// The actual signature bytes found.
        found: [u8; 2],
    },

    /// The GPT signature ("EFI PART") is invalid.
    InvalidGptSignature {
        /// The actual signature bytes found.
        found: [u8; 8],
    },

    /// The GPT header CRC32 checksum does not match.
    GptHeaderCrcMismatch {
        /// The expected CRC32 value (from header).
        expected: u32,
        /// The actual CRC32 value (calculated).
        actual: u32,
    },

    /// The GPT partition entry array CRC32 checksum does not match.
    GptEntriesCrcMismatch {
        /// The expected CRC32 value (from header).
        expected: u32,
        /// The actual CRC32 value (calculated).
        actual: u32,
    },

    /// Two partitions overlap.
    PartitionOverlap {
        /// Index of the first overlapping partition.
        index1: usize,
        /// Index of the second overlapping partition.
        index2: usize,
        /// Starting LBA of the overlap.
        overlap_start: u64,
        /// Ending LBA of the overlap.
        overlap_end: u64,
    },

    /// Too many partitions requested.
    TooManyPartitions {
        /// Maximum number of partitions allowed.
        max: usize,
        /// Number of partitions requested.
        requested: usize,
    },

    /// A partition extends beyond the disk boundary.
    PartitionOutOfBounds {
        /// Index of the offending partition.
        index: usize,
        /// Ending LBA of the partition.
        partition_end: u64,
        /// Last usable LBA of the disk.
        disk_end: u64,
    },

    /// The partition entry size is invalid.
    InvalidPartitionEntrySize {
        /// The invalid size.
        size: u32,
    },

    /// The backup GPT header does not match the primary.
    BackupHeaderMismatch,

    /// No protective MBR found on a GPT disk.
    NoProtectiveMbr,

    /// Invalid hybrid MBR configuration.
    InvalidHybridMbr {
        /// Description of the error.
        reason: &'static str,
    },

    /// The disk is too small for the requested partitions.
    DiskTooSmall {
        /// Required size in sectors.
        required: u64,
        /// Available size in sectors.
        available: u64,
    },

    /// A required feature is not available.
    FeatureNotAvailable(&'static str),

    /// A partition is not properly aligned.
    MisalignedPartition {
        /// The misaligned LBA.
        lba: u64,
        /// The required alignment in sectors.
        required_alignment: u64,
    },
}

impl Display for PartitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::InvalidMbrSignature { found } => {
                write!(
                    f,
                    "invalid MBR signature: expected 0x55AA, found 0x{:02X}{:02X}",
                    found[0], found[1]
                )
            }
            Self::InvalidGptSignature { found } => {
                write!(
                    f,
                    "invalid GPT signature: expected 'EFI PART', found {found:?}"
                )
            }
            Self::GptHeaderCrcMismatch { expected, actual } => {
                write!(
                    f,
                    "GPT header CRC mismatch: expected 0x{expected:08X}, got 0x{actual:08X}"
                )
            }
            Self::GptEntriesCrcMismatch { expected, actual } => {
                write!(
                    f,
                    "GPT entries CRC mismatch: expected 0x{expected:08X}, got 0x{actual:08X}"
                )
            }
            Self::PartitionOverlap {
                index1,
                index2,
                overlap_start,
                overlap_end,
            } => {
                write!(
                    f,
                    "partitions {index1} and {index2} overlap (LBA {overlap_start}-{overlap_end})"
                )
            }
            Self::TooManyPartitions { max, requested } => {
                write!(
                    f,
                    "too many partitions: maximum is {max}, requested {requested}"
                )
            }
            Self::PartitionOutOfBounds {
                index,
                partition_end,
                disk_end,
            } => {
                write!(
                    f,
                    "partition {index} extends beyond disk (ends at LBA {partition_end}, disk ends at {disk_end})"
                )
            }
            Self::InvalidPartitionEntrySize { size } => {
                write!(
                    f,
                    "invalid partition entry size: {size} (must be 128 * 2^n)"
                )
            }
            Self::BackupHeaderMismatch => {
                write!(f, "backup GPT header does not match primary")
            }
            Self::NoProtectiveMbr => {
                write!(f, "no protective MBR found on GPT disk")
            }
            Self::InvalidHybridMbr { reason } => {
                write!(f, "invalid hybrid MBR: {reason}")
            }
            Self::DiskTooSmall {
                required,
                available,
            } => {
                write!(
                    f,
                    "disk too small: requires {required} sectors, only {available} available"
                )
            }
            Self::FeatureNotAvailable(feature) => {
                write!(f, "feature not available: {feature}")
            }
            Self::MisalignedPartition {
                lba,
                required_alignment,
            } => {
                write!(
                    f,
                    "partition at LBA {lba} is not aligned to {required_alignment} sectors"
                )
            }
        }
    }
}

impl<E: hadris_io::IoError> From<hadris_io::Error<E>> for PartitionError {
    fn from(err: hadris_io::Error<E>) -> Self {
        Self::Io(err.erase())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for PartitionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

/// A specialized `Result` type for partition operations.
pub type Result<T> = core::result::Result<T, PartitionError>;
