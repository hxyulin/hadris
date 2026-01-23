//! Error types for partition operations.

use core::fmt::{self, Debug, Display};

/// Errors that can occur during partition table operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionError {
    /// An I/O error occurred.
    Io,

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
            Self::Io => write!(f, "I/O error"),
            Self::InvalidMbrSignature { found } => {
                write!(f, "invalid MBR signature: expected 0x55AA, found 0x{:02X}{:02X}", found[0], found[1])
            }
            Self::InvalidGptSignature { found } => {
                write!(f, "invalid GPT signature: expected 'EFI PART', found {:?}", found)
            }
            Self::GptHeaderCrcMismatch { expected, actual } => {
                write!(f, "GPT header CRC mismatch: expected 0x{:08X}, got 0x{:08X}", expected, actual)
            }
            Self::GptEntriesCrcMismatch { expected, actual } => {
                write!(f, "GPT entries CRC mismatch: expected 0x{:08X}, got 0x{:08X}", expected, actual)
            }
            Self::PartitionOverlap { index1, index2, overlap_start, overlap_end } => {
                write!(f, "partitions {} and {} overlap (LBA {}-{})", index1, index2, overlap_start, overlap_end)
            }
            Self::TooManyPartitions { max, requested } => {
                write!(f, "too many partitions: maximum is {}, requested {}", max, requested)
            }
            Self::PartitionOutOfBounds { index, partition_end, disk_end } => {
                write!(f, "partition {} extends beyond disk (ends at LBA {}, disk ends at {})", index, partition_end, disk_end)
            }
            Self::InvalidPartitionEntrySize { size } => {
                write!(f, "invalid partition entry size: {} (must be 128 * 2^n)", size)
            }
            Self::BackupHeaderMismatch => {
                write!(f, "backup GPT header does not match primary")
            }
            Self::NoProtectiveMbr => {
                write!(f, "no protective MBR found on GPT disk")
            }
            Self::InvalidHybridMbr { reason } => {
                write!(f, "invalid hybrid MBR: {}", reason)
            }
            Self::DiskTooSmall { required, available } => {
                write!(f, "disk too small: requires {} sectors, only {} available", required, available)
            }
            Self::FeatureNotAvailable(feature) => {
                write!(f, "feature not available: {}", feature)
            }
            Self::MisalignedPartition { lba, required_alignment } => {
                write!(
                    f,
                    "partition at LBA {} is not aligned to {} sectors",
                    lba, required_alignment
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for PartitionError {}

/// A specialized `Result` type for partition operations.
pub type Result<T> = core::result::Result<T, PartitionError>;
