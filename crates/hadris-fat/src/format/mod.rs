//! FAT volume formatting support.
//!
//! This module provides the ability to format new FAT12, FAT16, and FAT32 volumes.
//!
//! # Example
//!
//! ```no_run
//! use std::fs::OpenOptions;
//! use hadris_fat::format::{FatVolumeFormatter, FormatOptions};
//!
//! # fn main() -> hadris_fat::Result<()> {
//! // Create or open a file for the volume
//! let file = OpenOptions::new()
//!     .read(true)
//!     .write(true)
//!     .create(true)
//!     .open("volume.img")?;
//!
//! // Set the file size (e.g., 64 MB)
//! file.set_len(64 * 1024 * 1024)?;
//!
//! // Format with default options
//! let options = FormatOptions::new(64 * 1024 * 1024)
//!     .with_label("MY VOLUME");
//!
//! let fs = FatVolumeFormatter::format(file, options)?;
//! # Ok(())
//! # }
//! ```

mod calc;
mod init;
mod options;

pub use calc::FormatParams;
pub use options::{FatTypeSelection, FormatOptions, MediaType, OemName, SectorSize, VolumeLabel};

use crate::error::Result;
use crate::io::{Read, Seek, Write};
use crate::FatFs;

/// FAT volume formatter.
///
/// This struct provides the main entry point for formatting new FAT volumes.
pub struct FatVolumeFormatter;

impl FatVolumeFormatter {
    /// Format a new FAT volume.
    ///
    /// This function formats the provided data source as a FAT12, FAT16, or FAT32
    /// volume based on the options provided. The FAT type is automatically selected
    /// based on volume size unless explicitly specified.
    ///
    /// # Arguments
    ///
    /// * `data` - The data source to format (must be seekable and writable)
    /// * `options` - Formatting options including volume size, label, etc.
    ///
    /// # Returns
    ///
    /// Returns an opened `FatFs` handle for the newly formatted volume.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The volume size is too small or too large for the selected FAT type
    /// - The formatting parameters are invalid
    /// - An I/O error occurs during formatting
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::io::Cursor;
    /// use hadris_fat::format::{FatVolumeFormatter, FormatOptions, FatTypeSelection};
    ///
    /// # fn main() -> hadris_fat::Result<()> {
    /// // Create a 2 MB in-memory volume
    /// let mut buffer = vec![0u8; 2 * 1024 * 1024];
    /// let cursor = std::io::Cursor::new(&mut buffer[..]);
    ///
    /// let options = FormatOptions::new(2 * 1024 * 1024)
    ///     .with_label("TEST")
    ///     .with_fat_type(FatTypeSelection::Fat12);
    ///
    /// let fs = FatVolumeFormatter::format(cursor, options)?;
    /// assert_eq!(fs.fat_type(), hadris_fat::FatType::Fat12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn format<DATA>(mut data: DATA, options: FormatOptions) -> Result<FatFs<DATA>>
    where
        DATA: Read + Write + Seek,
    {
        use crate::io::SeekFrom;

        // Calculate formatting parameters
        let params = calc::calculate_params(&options)?;

        // Initialize the volume structures
        init::initialize_volume(&mut data, &options, &params)?;

        // Seek back to the beginning before opening
        data.seek(SeekFrom::Start(0))?;

        // Open and return the newly formatted filesystem
        FatFs::open(data)
    }

    /// Calculate formatting parameters without actually formatting.
    ///
    /// This is useful for validating options and previewing the volume layout
    /// before committing to the format operation.
    ///
    /// # Arguments
    ///
    /// * `options` - Formatting options to validate
    ///
    /// # Returns
    ///
    /// Returns the calculated formatting parameters including:
    /// - FAT type (FAT12, FAT16, or FAT32)
    /// - Cluster count and size
    /// - FAT size
    /// - Root directory parameters
    pub fn calculate_params(options: &FormatOptions) -> Result<FormatParams> {
        calc::calculate_params(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use std::io::Cursor;

    #[test]
    fn test_format_fat12() {
        // 2 MB volume should create FAT12
        let mut buffer = vec![0u8; 2 * 1024 * 1024];
        let cursor = Cursor::new(&mut buffer[..]);

        let options = FormatOptions::new(2 * 1024 * 1024).with_label("FAT12TEST");

        let fs = FatVolumeFormatter::format(cursor, options).unwrap();
        assert_eq!(fs.fat_type(), crate::FatType::Fat12);
        assert_eq!(fs.volume_info().volume_label(), "FAT12TEST");
    }

    #[test]
    fn test_format_fat16() {
        // 64 MB volume should create FAT16
        let mut buffer = vec![0u8; 64 * 1024 * 1024];
        let cursor = Cursor::new(&mut buffer[..]);

        let options = FormatOptions::new(64 * 1024 * 1024).with_label("FAT16TEST");

        let fs = FatVolumeFormatter::format(cursor, options).unwrap();
        assert_eq!(fs.fat_type(), crate::FatType::Fat16);
        assert_eq!(fs.volume_info().volume_label(), "FAT16TEST");
    }

    #[test]
    fn test_format_fat32() {
        // Use forced FAT32 for a moderate size volume
        // (Auto would select FAT16 for 256 MB)
        let mut buffer = vec![0u8; 256 * 1024 * 1024];
        let cursor = Cursor::new(&mut buffer[..]);

        let options = FormatOptions::new(256 * 1024 * 1024)
            .with_label("FAT32TEST")
            .with_fat_type(FatTypeSelection::Fat32);

        let fs = FatVolumeFormatter::format(cursor, options).unwrap();
        assert_eq!(fs.fat_type(), crate::FatType::Fat32);
        assert_eq!(fs.volume_info().volume_label(), "FAT32TEST");
    }

    #[test]
    fn test_format_and_create_file() {
        let mut buffer = vec![0u8; 4 * 1024 * 1024];
        let cursor = Cursor::new(&mut buffer[..]);

        let options = FormatOptions::new(4 * 1024 * 1024);
        let fs = FatVolumeFormatter::format(cursor, options).unwrap();

        // Create a file
        let root = fs.root_dir();
        let file_entry = fs.create_file(&root, "TEST.TXT").unwrap();
        // The name() method returns the 8.3 format with spaces
        assert!(file_entry.name().starts_with("TEST"));
        assert!(file_entry.name().contains("TXT"));
        assert!(file_entry.is_file());
    }
}
