//! Unstable exFAT filesystem preview.
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
//!
//! # Stability and supported subset
//!
//! This module is enabled by `unstable-exfat` and is outside the Hadris V2 API
//! stability promise. Its APIs may change in V2 minor releases. It supports
//! basic formatting, reading, traversal, and simple mutation on conventional
//! layouts, but should not be used with irreplaceable data.
//!
//! The preview is synchronous and allocation-backed. Fragmented allocation
//! bitmap and up-case metadata, directory growth, general cross-cluster
//! directory entry-set placement, TexFAT, and repair workflows are not
//! supported. The `hadris-block` facade reports
//! `detect::FatVariant::ExFat` for format detection, but its stable
//! block-volume opener intentionally rejects exFAT until this implementation
//! is release-qualified.

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
pub use fs::ExFatVolume;
pub use time::ExFatTimestamp;
pub use upcase::UpcaseTable;

#[cfg(feature = "write")]
pub use file::ExFatFileWriter;
#[cfg(feature = "write")]
pub use format::{ExFatFormatOptions, ExFatLayoutParams, format_exfat};
