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
    /// I/O error annotated with the operation and (optionally) the sector
    /// where it happened. Used at trust boundaries — boot sector parse,
    /// FSInfo read, FAT chain walk entry, directory iteration head — so
    /// callers can tell *which* on-disk structure tripped the failure
    /// rather than just seeing an opaque [`Self::Io`].
    IoContext {
        /// Static label describing what was being read (e.g. `"boot sector"`).
        op: &'static str,
        /// Sector where the failure originated, when known.
        sector: Option<u64>,
        /// The wrapped underlying I/O error.
        source: hadris_io::Error,
    },
    /// On-disk arithmetic on an untrusted value would overflow or underflow.
    ///
    /// The `context` string identifies which on-disk value triggered the
    /// failure (e.g. `"sectors_per_fat * sector_size"` when a corrupt BPB
    /// would push the FAT region past `usize::MAX`). This is distinct from
    /// `ClusterOutOfBounds`, which catches *valid* arithmetic that lands
    /// outside the cluster range.
    CorruptFilesystem {
        /// Static label describing which arithmetic check failed.
        context: &'static str,
    },
    /// A FAT chain walk visited more clusters than the filesystem contains,
    /// which means the chain has a loop. Always corruption — a healthy chain
    /// is bounded by `max_cluster - 1` (clusters 0 and 1 are reserved).
    ClusterLoop {
        /// The cluster the walker was inspecting when it hit the limit.
        cluster: u32,
    },
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
    /// A single name needs a run of directory entries (LFN slots + the
    /// short entry) that does not fit in one cluster. This implementation
    /// keeps every run within a single cluster, so the name is rejected.
    /// Distinct from [`Self::DirectoryFull`]: the directory has room, but
    /// the volume's cluster size is too small for a name this long.
    #[cfg(feature = "write")]
    DirEntryRunTooLong {
        /// Directory entries the name requires (LFN slots + 1 short entry).
        entries_needed: usize,
        /// Directory entries that fit in one cluster.
        entries_per_cluster: usize,
    },
    /// Filename is invalid or too long
    #[cfg(feature = "write")]
    InvalidFilename,
    /// Entry with this name already exists
    #[cfg(feature = "write")]
    AlreadyExists,
    /// Cannot delete non-empty directory
    #[cfg(feature = "write")]
    DirectoryNotEmpty,

    /// Attempted to flip an immutable attribute bit (`DIRECTORY` or
    /// `VOLUME_ID`) on an existing entry. Those bits identify the *kind* of
    /// entry on disk; changing them in-place would leave the cluster chain
    /// or root volume label inconsistent.
    #[cfg(feature = "write")]
    InvalidAttributeChange {
        /// Which immutable bit the caller tried to flip.
        bit: &'static str,
    },

    /// A read-only [`crate::cache::FatSectorCache`] operation needed to evict
    /// a sector to make room for a new one, but every cached sector is
    /// dirty. The caller must call [`crate::cache::FatSectorCache::flush`]
    /// (or [`crate::FatFs::flush`]) before continuing — read paths can't
    /// safely drop unwritten dirty data.
    #[cfg(feature = "cache")]
    CacheDirtyEviction {
        /// The dirty sector that would have been evicted.
        sector: u32,
    },

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
            Self::IoContext { op, sector, source } => match sector {
                Some(s) => write!(f, "I/O error reading {op} (sector {s}): {source:?}"),
                None => write!(f, "I/O error reading {op}: {source:?}"),
            },
            Self::CorruptFilesystem { context } => {
                write!(f, "corrupt filesystem: {context}")
            }
            Self::ClusterLoop { cluster } => {
                write!(f, "cluster chain loop detected at cluster {cluster}")
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
            Self::DirEntryRunTooLong {
                entries_needed,
                entries_per_cluster,
            } => {
                write!(
                    f,
                    "name needs {entries_needed} directory entries but only {entries_per_cluster} fit in a cluster (name too long for this cluster size)"
                )
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
            Self::InvalidAttributeChange { bit } => {
                write!(f, "cannot change immutable attribute bit `{bit}` in place")
            }
            #[cfg(feature = "cache")]
            Self::CacheDirtyEviction { sector } => {
                write!(
                    f,
                    "FAT cache is full and every sector is dirty (sector {sector}); call flush() before continuing"
                )
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

impl<E: hadris_io::IoError> From<hadris_io::Error<E>> for FatError {
    fn from(e: hadris_io::Error<E>) -> Self {
        Self::Io(e.erase())
    }
}

/// Manual `defmt::Format` impl rather than `derive`, because the wrapped
/// `hadris_io::Error` does not (yet) implement `Format`. The Io variant logs
/// without details; everything else mirrors the Display output.
#[cfg(feature = "defmt")]
impl defmt::Format for FatError {
    fn format(&self, f: defmt::Formatter) {
        match self {
            Self::InvalidBootSignature { found } => {
                defmt::write!(
                    f,
                    "invalid boot signature: expected 0xAA55, found {=u16:#06x}",
                    *found
                )
            }
            Self::UnsupportedFatType(ty) => defmt::write!(f, "unsupported FAT type: {=str}", *ty),
            Self::InvalidFsInfoSignature {
                field,
                expected,
                found,
            } => defmt::write!(
                f,
                "invalid FSInfo signature: {=str} expected {=u32:#010x}, found {=u32:#010x}",
                *field,
                *expected,
                *found
            ),
            Self::InvalidShortFilename => defmt::write!(f, "invalid short filename"),
            Self::ClusterOutOfBounds { cluster, max } => {
                defmt::write!(
                    f,
                    "cluster {=u32} out of bounds (max: {=u32})",
                    *cluster,
                    *max
                )
            }
            Self::BadCluster { cluster } => {
                defmt::write!(f, "bad cluster marker at cluster {=u32}", *cluster)
            }
            Self::UnexpectedEndOfChain { cluster } => {
                defmt::write!(f, "unexpected end of cluster chain at {=u32}", *cluster)
            }
            // hadris_io::Error doesn't implement defmt::Format yet — log a
            // generic message rather than dragging the dependency tree.
            Self::Io(_) => defmt::write!(f, "I/O error"),
            Self::IoContext { op, sector, .. } => match sector {
                Some(s) => defmt::write!(f, "I/O error reading {=str} (sector {=u64})", *op, *s),
                None => defmt::write!(f, "I/O error reading {=str}", *op),
            },
            Self::CorruptFilesystem { context } => {
                defmt::write!(f, "corrupt filesystem: {=str}", *context)
            }
            Self::ClusterLoop { cluster } => {
                defmt::write!(f, "cluster chain loop at {=u32}", *cluster)
            }
            Self::NotAFile => defmt::write!(f, "entry is not a file"),
            Self::NotADirectory => defmt::write!(f, "entry is not a directory"),
            Self::EntryNotFound => defmt::write!(f, "entry not found"),
            Self::InvalidPath => defmt::write!(f, "path is invalid"),
            #[cfg(feature = "write")]
            Self::NoFreeSpace => defmt::write!(f, "no free clusters"),
            #[cfg(feature = "write")]
            Self::DirectoryFull => defmt::write!(f, "directory is full"),
            #[cfg(feature = "write")]
            Self::DirEntryRunTooLong {
                entries_needed,
                entries_per_cluster,
            } => defmt::write!(
                f,
                "name needs {=usize} dir entries but only {=usize} fit in a cluster",
                *entries_needed,
                *entries_per_cluster
            ),
            #[cfg(feature = "write")]
            Self::InvalidFilename => defmt::write!(f, "filename invalid or too long"),
            #[cfg(feature = "write")]
            Self::AlreadyExists => defmt::write!(f, "entry already exists"),
            #[cfg(feature = "write")]
            Self::DirectoryNotEmpty => defmt::write!(f, "directory not empty"),
            #[cfg(feature = "write")]
            Self::InvalidAttributeChange { bit } => {
                defmt::write!(f, "cannot change immutable attribute bit `{=str}`", *bit)
            }
            #[cfg(feature = "cache")]
            Self::CacheDirtyEviction { sector } => defmt::write!(
                f,
                "FAT cache full and every sector dirty (sector {=u32}); flush() needed",
                *sector
            ),
            #[cfg(feature = "write")]
            Self::VolumeTooSmall { size, min_size } => defmt::write!(
                f,
                "volume size {=u64} too small (min: {=u64})",
                *size,
                *min_size
            ),
            #[cfg(feature = "write")]
            Self::VolumeTooLarge { size, max_size } => defmt::write!(
                f,
                "volume size {=u64} too large (max: {=u64})",
                *size,
                *max_size
            ),
            #[cfg(feature = "write")]
            Self::InvalidFormatOption { option, reason } => {
                defmt::write!(
                    f,
                    "invalid format option `{=str}`: {=str}",
                    *option,
                    *reason
                )
            }
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidSignature { .. } => defmt::write!(f, "invalid exFAT signature"),
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidBootSector { reason } => {
                defmt::write!(f, "invalid exFAT boot sector: {=str}", *reason)
            }
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidChecksum { expected, found } => defmt::write!(
                f,
                "invalid exFAT checksum: expected {=u32:#010x}, found {=u32:#010x}",
                *expected,
                *found
            ),
            #[cfg(feature = "exfat")]
            Self::ExFatInvalidEntry { reason } => {
                defmt::write!(f, "invalid exFAT directory entry: {=str}", *reason)
            }
        }
    }
}

/// Result type alias for FAT operations.
pub type Result<T> = core::result::Result<T, FatError>;
