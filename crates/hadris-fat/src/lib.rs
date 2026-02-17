#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod dir;
pub mod error;
pub mod fat_table;
pub mod file;
pub mod fs;
pub mod io;
pub mod raw;
pub mod read;
pub mod write;

#[cfg(feature = "cache")]
pub mod cache;

#[cfg(feature = "tool")]
pub mod tool;

#[cfg(feature = "exfat")]
pub mod exfat;

#[cfg(feature = "write")]
pub mod format;

// Re-exports from error
pub use error::{FatError, Result};

// Re-exports from io
pub use io::Cluster;

// Re-exports from fs
pub use fs::{FatFs, VolumeInfo};

// Re-exports from dir
pub use dir::{
    DirectoryEntry, FatDir, FatDirIter, FileEntry, FileSystemErrors, FileSystemWarnings, ParseInfo,
};

// Re-exports from fat_table
pub use fat_table::{Fat, Fat12, Fat16, Fat32, FatType};

// Re-exports from raw
pub use raw::{
    BpbExt32Flags, DirEntryAttrFlags, RawBpb, RawBpbExt16, RawBpbExt32, RawDirectoryEntry,
    RawFileEntry, RawFsInfo, RawLfnEntry,
};

// Re-exports from read
pub use read::{FatFsReadExt, FileReader};

// Re-exports from cache
#[cfg(feature = "cache")]
pub use cache::{CacheStats, CachedFat, DEFAULT_CACHE_CAPACITY, FatSectorCache};

// Re-exports from format
#[cfg(feature = "write")]
pub use format::{FatVolumeFormatter, FormatOptions, FormatParams};

// Re-exports from write
#[cfg(feature = "write")]
pub use write::{FatDateTime, FatFsWriteExt, FileWriter};

// Re-exports from tool
#[cfg(feature = "tool")]
pub use tool::{
    analysis::{
        ClusterState, FatAnalysisExt, FatStatistics, FileFragmentInfo, FragmentationReport,
    },
    verify::{FatVerifyExt, VerificationIssue, VerificationReport},
};

// Internal re-exports used by format module
pub(crate) use fs::{FSINFO_LEAD_SIG, FSINFO_STRUC_SIG, FSINFO_TRAIL_SIG};
