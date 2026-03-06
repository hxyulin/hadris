//! Sector layout and allocation for hybrid CD/DVD images
//!
//! This module handles the physical layout of data on disk, ensuring that
//! both ISO 9660 and UDF filesystems can reference the same file data.
//!
//! ## Disk Layout for UDF Bridge Format
//!
//! ```text
//! Sector 0-15:    System area (boot code, partition tables)
//! Sector 16:      ISO Primary Volume Descriptor
//! Sector 17:      UDF BEA01 (Beginning of Extended Area)
//! Sector 18:      UDF NSR02/NSR03 (UDF identifier)
//! Sector 19:      UDF TEA01 (Terminal Extended Area)
//! Sector 20-...:  More ISO Volume Descriptors (Joliet SVD, etc.)
//! Sector ..:      ISO Volume Descriptor Set Terminator
//! Sector 256:     UDF Anchor Volume Descriptor Pointer
//! Sector 257+:    UDF Volume Descriptor Sequence
//! Sector ..:      UDF File Set Descriptor
//! Sector ..:      File data (shared between ISO and UDF)
//! Sector ..:      ISO directory records
//! Sector ..:      UDF directory structures (File Entries, FIDs)
//! Sector ..:      ISO path tables
//! ```

use crate::error::{CdError, CdResult};
use crate::options::CdOptions;
use crate::tree::{Directory, FileExtent, FileTree};

/// Handles sector allocation for the CD image
#[derive(Debug)]
pub struct LayoutManager {
    /// Sector size (usually 2048)
    sector_size: usize,
    /// Next available sector for file data
    next_file_sector: u32,
    /// Next available sector within UDF partition
    next_udf_block: u32,
    /// Next unique ID for UDF
    next_unique_id: u64,
}

impl LayoutManager {
    /// Create a new layout manager
    pub fn new(sector_size: usize) -> Self {
        Self {
            sector_size,
            // File data starts after the system area, volume descriptors, and UDF structures
            // We'll calculate this more precisely during layout
            next_file_sector: 0,
            next_udf_block: 0,
            next_unique_id: 16, // UDF reserves IDs 0-15
        }
    }

    /// Allocate sectors for file data and assign extents to all files
    ///
    /// This is the core layout function that determines where each file's
    /// data will be stored on disk. Both ISO and UDF will reference these
    /// same sectors.
    pub fn layout_files(
        &mut self,
        tree: &mut FileTree,
        options: &CdOptions,
    ) -> CdResult<LayoutInfo> {
        // Calculate starting positions based on what we need to write
        let vds_end = self.calculate_vds_end(options);

        // UDF partition starts after AVDP at sector 256
        let udf_partition_start = 257;

        // Reserve space for UDF structures (FSD, directory entries)
        // We'll use a conservative estimate and may adjust later
        let udf_metadata_sectors = self.estimate_udf_metadata_sectors(tree);

        // File data starts after UDF metadata (within UDF partition)
        self.next_udf_block = udf_metadata_sectors;
        self.next_file_sector = udf_partition_start + udf_metadata_sectors;

        // Assign extents to all files
        self.assign_file_extents(&mut tree.root)?;

        // Assign unique IDs to directories and files
        self.assign_unique_ids(&mut tree.root);

        let file_data_end = self.next_file_sector;

        Ok(LayoutInfo {
            vds_end,
            udf_partition_start,
            udf_metadata_sectors,
            file_data_start: udf_partition_start + udf_metadata_sectors,
            file_data_end,
            total_sectors: file_data_end + 100, // Reserve space for ISO path tables etc.
        })
    }

    /// Calculate where the Volume Descriptor Sequence ends
    fn calculate_vds_end(&self, options: &CdOptions) -> u32 {
        let mut sector = 16; // VDS starts at sector 16

        // ISO Primary Volume Descriptor
        if options.iso.enabled {
            sector += 1;
        }

        // UDF VRS (BEA01, NSR02/03, TEA01) - actually at 16-18 with ISO
        // In hybrid format, VRS is interleaved with ISO VD
        // For simplicity, we'll place VRS at sectors 16-18

        // Joliet SVD
        if options.iso.joliet.is_some() {
            sector += 1;
        }

        // ISO 9660:1999 EVD
        if options.iso.long_filenames {
            sector += 1;
        }

        // Boot record (El-Torito)
        if options.boot.is_some() {
            sector += 1;
        }

        // Volume Set Terminator
        sector += 1;

        sector
    }

    /// Estimate how many sectors we need for UDF metadata
    fn estimate_udf_metadata_sectors(&self, tree: &FileTree) -> u32 {
        // File Set Descriptor: 1 sector
        // Root directory File Entry: 1 sector
        // Root directory FIDs: ceil(entry_count * ~40 bytes / sector_size)
        // For each subdirectory: File Entry + FIDs

        let total_dirs = tree.total_dirs();
        let total_files = tree.total_files();

        // Each directory needs at least 2 sectors (File Entry + FIDs)
        // Plus some buffer for larger directories
        let estimated = (total_dirs * 2 + total_files / 50 + 10) as u32;

        // Round up to be safe
        estimated.max(20)
    }

    /// Recursively assign file extents
    fn assign_file_extents(&mut self, dir: &mut Directory) -> CdResult<()> {
        // Assign extents to files
        for file in &mut dir.files {
            let size = file.size().map_err(CdError::Io)?;

            if size == 0 {
                // Zero-size files have no extent (sector 0 per ISO spec)
                file.extent = FileExtent::new(0, 0);
            } else {
                file.extent = FileExtent::new(self.next_file_sector, size);
                let sectors = file.extent.sector_count(self.sector_size);
                self.next_file_sector += sectors;
            }
        }

        // Recursively handle subdirectories
        for subdir in &mut dir.subdirs {
            self.assign_file_extents(subdir)?;
        }

        Ok(())
    }

    /// Assign unique IDs to all directories and files
    fn assign_unique_ids(&mut self, dir: &mut Directory) {
        dir.unique_id = self.next_unique_id;
        self.next_unique_id += 1;

        for file in &mut dir.files {
            file.unique_id = self.next_unique_id;
            self.next_unique_id += 1;
        }

        for subdir in &mut dir.subdirs {
            self.assign_unique_ids(subdir);
        }
    }

    /// Allocate a single sector within the UDF partition
    pub fn allocate_udf_block(&mut self) -> u32 {
        let block = self.next_udf_block;
        self.next_udf_block += 1;
        block
    }

    /// Get the next available unique ID
    pub fn next_unique_id(&mut self) -> u64 {
        let id = self.next_unique_id;
        self.next_unique_id += 1;
        id
    }
}

/// Information about the disk layout after planning
#[derive(Debug, Clone)]
pub struct LayoutInfo {
    /// Sector where volume descriptor sequence ends
    pub vds_end: u32,
    /// Starting sector of UDF partition
    pub udf_partition_start: u32,
    /// Number of sectors reserved for UDF metadata
    pub udf_metadata_sectors: u32,
    /// Starting sector for file data
    pub file_data_start: u32,
    /// Ending sector for file data
    pub file_data_end: u32,
    /// Total sectors needed for the image
    pub total_sectors: u32,
}

impl core::fmt::Display for LayoutInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "layout: {} total sectors (files at sectors {}-{})",
            self.total_sectors, self.file_data_start, self.file_data_end
        )
    }
}

impl LayoutInfo {
    /// Get the UDF partition length in sectors
    pub fn udf_partition_length(&self) -> u32 {
        self.total_sectors - self.udf_partition_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::FileEntry;

    #[test]
    fn test_layout_empty_tree() {
        let mut tree = FileTree::new();
        let options = CdOptions::default();
        let mut layout = LayoutManager::new(2048);

        let info = layout.layout_files(&mut tree, &options).unwrap();
        assert!(info.file_data_end >= info.file_data_start);
    }

    #[test]
    fn test_layout_with_files() {
        let mut tree = FileTree::new();
        tree.add_file(FileEntry::from_buffer("test.txt", vec![0u8; 4096]));
        tree.add_file(FileEntry::from_buffer("small.txt", vec![0u8; 100]));

        let options = CdOptions::default();
        let mut layout = LayoutManager::new(2048);

        let info = layout.layout_files(&mut tree, &options).unwrap();

        // First file should have a valid extent
        let file1 = tree.root.files.get(0).unwrap();
        assert!(file1.extent.sector > 0);
        assert_eq!(file1.extent.length, 4096);

        // Second file should come after first
        let file2 = tree.root.files.get(1).unwrap();
        assert!(file2.extent.sector > file1.extent.sector);
    }

    #[test]
    fn test_layout_zero_size_file() {
        let mut tree = FileTree::new();
        tree.add_file(FileEntry::from_buffer("empty.txt", vec![]));

        let options = CdOptions::default();
        let mut layout = LayoutManager::new(2048);

        layout.layout_files(&mut tree, &options).unwrap();

        let file = tree.root.files.get(0).unwrap();
        assert_eq!(file.extent.sector, 0);
        assert_eq!(file.extent.length, 0);
    }
}
