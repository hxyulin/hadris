//! exFAT filesystem implementation.
//!
//! exFAT (Extended File Allocation Table) is a filesystem designed for flash drives
//! and external storage. It differs significantly from FAT12/16/32:
//!
//! - 12-sector boot region (vs single sector)
//! - Allocation bitmap + optional FAT chains
//! - Directory entry sets of 3-19 entries
//! - No 8.3 short names, only Unicode (up to 255 chars)
//! - 64-bit file sizes
//! - UTC timestamps with timezone offsets

mod bitmap;
mod boot;
mod dir;
mod entry;
#[cfg(feature = "write")]
mod entry_writer;
mod fat;
mod file;
#[cfg(feature = "write")]
mod format;
mod fs;
mod time;
mod upcase;

pub use bitmap::AllocationBitmap;
pub use boot::{ExFatBootSector, ExFatInfo};
pub use dir::{ExFatDir, ExFatDirIter};
pub use entry::{
    ExFatFileEntry, FileAttributes, RawFileDirectoryEntry, RawFileNameEntry,
    RawStreamExtensionEntry,
};
pub use fat::ExFatTable;
pub use file::ExFatFileReader;
pub use fs::ExFatFs;
pub use time::ExFatTimestamp;
pub use upcase::UpcaseTable;

#[cfg(feature = "write")]
pub use file::ExFatFileWriter;
#[cfg(feature = "write")]
pub use format::{ExFatFormatOptions, ExFatLayoutParams, format_exfat};
