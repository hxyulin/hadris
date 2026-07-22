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

use crate::error::{Error, Result};
use crate::options::OpticalImageOptions;
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
        options: &OpticalImageOptions,
    ) -> Result<LayoutInfo> {
        // Calculate starting positions based on what we need to write
        let vds_end = self.calculate_vds_end(options);

        // Main/reserve VDS occupy 257-268 and the LVID occupies 269.
        let udf_partition_start = 270;

        // Plan every UDF metadata object once, globally. Block 0 is the FSD;
        // directory/file ICBs and exact-sized FID extents follow it.
        let mut next_udf_block = 1;
        let udf_root =
            Self::plan_udf_directory(&tree.root, &mut next_udf_block, None, self.sector_size)?;
        let udf_metadata_sectors = next_udf_block;

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
            udf_root,
        })
    }

    /// Calculate where the Volume Descriptor Sequence ends
    fn calculate_vds_end(&self, options: &OpticalImageOptions) -> u32 {
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

    fn plan_udf_directory(
        dir: &Directory,
        next_block: &mut u32,
        parent_icb: Option<u32>,
        sector_size: usize,
    ) -> Result<UdfDirectoryLayout> {
        let icb_block = *next_block;
        *next_block = next_block
            .checked_add(1)
            .ok_or_else(|| Error::InvalidConfig("UDF metadata block overflow".into()))?;

        let mut fid_bytes = 40usize; // parent FID (38-byte base, padded to four)
        for name in dir
            .files
            .iter()
            .map(|file| file.name.as_str())
            .chain(dir.subdirs.iter().map(|child| child.name.as_str()))
        {
            let encoded_len = cs0_filename_len(name)?;
            fid_bytes = fid_bytes
                .checked_add((38 + encoded_len + 3) & !3)
                .ok_or_else(|| Error::InvalidConfig("UDF FID size overflow".into()))?;
        }
        let fid_sectors = fid_bytes.div_ceil(sector_size) as u32;
        let fid_block = *next_block;
        *next_block = next_block
            .checked_add(fid_sectors)
            .ok_or_else(|| Error::InvalidConfig("UDF metadata block overflow".into()))?;

        let mut file_icb_blocks = Vec::with_capacity(dir.files.len());
        for _ in &dir.files {
            file_icb_blocks.push(*next_block);
            *next_block = next_block
                .checked_add(1)
                .ok_or_else(|| Error::InvalidConfig("UDF metadata block overflow".into()))?;
        }

        let mut subdirs = Vec::with_capacity(dir.subdirs.len());
        for child in &dir.subdirs {
            subdirs.push(Self::plan_udf_directory(
                child,
                next_block,
                Some(icb_block),
                sector_size,
            )?);
        }

        Ok(UdfDirectoryLayout {
            icb_block,
            parent_icb_block: parent_icb.unwrap_or(icb_block),
            fid_block,
            fid_bytes,
            file_icb_blocks,
            subdirs,
        })
    }

    /// Recursively assign file extents
    fn assign_file_extents(&mut self, dir: &mut Directory) -> Result<()> {
        // Assign extents to files
        for file in &mut dir.files {
            let size = file
                .size()
                .map_err(|error| Error::Io(hadris_io::Error::from_source(error).erase()))?;

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
    /// Complete collision-free UDF directory/ICB plan.
    pub(crate) udf_root: UdfDirectoryLayout,
}

/// Planned UDF metadata blocks for one directory and its descendants.
#[derive(Debug, Clone)]
pub(crate) struct UdfDirectoryLayout {
    pub(crate) icb_block: u32,
    pub(crate) parent_icb_block: u32,
    pub(crate) fid_block: u32,
    pub(crate) fid_bytes: usize,
    pub(crate) file_icb_blocks: Vec<u32>,
    pub(crate) subdirs: Vec<UdfDirectoryLayout>,
}

fn cs0_filename_len(name: &str) -> Result<usize> {
    let content_len = if name.chars().all(|ch| (ch as u32) <= 0xff) {
        name.chars().count()
    } else {
        name.encode_utf16()
            .count()
            .checked_mul(2)
            .ok_or_else(|| Error::InvalidConfig("UDF filename encoded length overflow".into()))?
    };
    let encoded_len = content_len + 1;
    if encoded_len > u8::MAX as usize {
        return Err(Error::InvalidPath(format!(
            "UDF filename exceeds the 255-byte encoded limit: {name}"
        )));
    }
    Ok(encoded_len)
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
        let options = OpticalImageOptions::default();
        let mut layout = LayoutManager::new(2048);

        let info = layout.layout_files(&mut tree, &options).unwrap();
        assert!(info.file_data_end >= info.file_data_start);
    }

    #[test]
    fn test_layout_with_files() {
        let mut tree = FileTree::new();
        tree.add_file(FileEntry::from_buffer("test.txt", vec![0u8; 4096]));
        tree.add_file(FileEntry::from_buffer("small.txt", vec![0u8; 100]));

        let options = OpticalImageOptions::default();
        let mut layout = LayoutManager::new(2048);

        layout.layout_files(&mut tree, &options).unwrap();

        // First file should have a valid extent
        let file1 = tree.root.files.first().unwrap();
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

        let options = OpticalImageOptions::default();
        let mut layout = LayoutManager::new(2048);

        layout.layout_files(&mut tree, &options).unwrap();

        let file = tree.root.files.first().unwrap();
        assert_eq!(file.extent.sector, 0);
        assert_eq!(file.extent.length, 0);
    }
}
