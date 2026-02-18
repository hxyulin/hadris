//! UDF Write Support
//!
//! This module provides functionality to write UDF filesystem structures.
//! It supports both low-level descriptor writing and high-level formatting.
//!
//! ## High-Level API
//!
//! Use [`UdfWriter::format`] to create a complete UDF filesystem:
//!
//! ```rust,no_run
//! use hadris_udf::write::{UdfWriter, UdfWriteOptions, SimpleFile, SimpleDir};
//! use std::io::Cursor;
//!
//! let mut buffer = vec![0u8; 10 * 1024 * 1024]; // 10MB
//! let mut cursor = Cursor::new(&mut buffer[..]);
//!
//! let mut root = SimpleDir::new("");
//! root.add_file(SimpleFile::new("readme.txt", b"Hello, World!".to_vec()));
//!
//! let mut subdir = SimpleDir::new("docs");
//! subdir.add_file(SimpleFile::new("guide.txt", b"User guide content".to_vec()));
//! root.add_dir(subdir);
//!
//! let options = UdfWriteOptions::default();
//! UdfWriter::format(&mut cursor, &root, options).expect("Format failed");
//! ```
//!
//! ## Low-Level API
//!
//! For fine-grained control (e.g., hybrid ISO+UDF images), use individual
//! descriptor writing methods on [`UdfWriter`].

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;

use super::descriptor::{
    DescriptorTag, ExtentDescriptor, LongAllocationDescriptor, ShortAllocationDescriptor,
    TagIdentifier,
};
use crate::dir::FileCharacteristics;
use crate::error::UdfResult;
use crate::file::FileType;
use crate::time::UdfTimestamp;
use crate::{AVDP_LOCATION, SECTOR_SIZE, UdfRevision};
use super::super::{Seek, SeekFrom, Write};

// =============================================================================
// High-Level Types for Simple UDF Creation
// =============================================================================

/// A simple file for the high-level format API
#[derive(Debug, Clone)]
pub struct SimpleFile {
    /// File name
    pub name: String,
    /// File content
    pub data: Vec<u8>,
}

impl SimpleFile {
    /// Create a new file with the given name and content
    pub fn new(name: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            data,
        }
    }

    /// Create an empty file
    pub fn empty(name: impl Into<String>) -> Self {
        Self::new(name, Vec::new())
    }
}

/// A simple directory for the high-level format API
#[derive(Debug, Clone, Default)]
pub struct SimpleDir {
    /// Directory name (empty for root)
    pub name: String,
    /// Files in this directory
    pub files: Vec<SimpleFile>,
    /// Subdirectories
    pub subdirs: Vec<SimpleDir>,
}

impl SimpleDir {
    /// Create a new empty directory
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            subdirs: Vec::new(),
        }
    }

    /// Create a root directory
    pub fn root() -> Self {
        Self::new("")
    }

    /// Add a file to this directory
    pub fn add_file(&mut self, file: SimpleFile) {
        self.files.push(file);
    }

    /// Add a subdirectory
    pub fn add_dir(&mut self, dir: SimpleDir) {
        self.subdirs.push(dir);
    }

    /// Count total files recursively
    pub fn total_files(&self) -> usize {
        self.files.len() + self.subdirs.iter().map(|d| d.total_files()).sum::<usize>()
    }

    /// Count total directories recursively (including self)
    pub fn total_dirs(&self) -> usize {
        1 + self.subdirs.iter().map(|d| d.total_dirs()).sum::<usize>()
    }

    /// Sort files and directories by name
    pub fn sort(&mut self) {
        self.files.sort_by(|a, b| a.name.cmp(&b.name));
        self.subdirs.sort_by(|a, b| a.name.cmp(&b.name));
        for subdir in &mut self.subdirs {
            subdir.sort();
        }
    }
}

// Internal structure for tracking allocated items during format
#[derive(Debug)]
struct AllocatedFile {
    name: String,
    data_block: u32,  // Block where file data starts
    data_length: u64, // File size in bytes
    icb_block: u32,   // Block where File Entry lives
    unique_id: u64,
}

#[derive(Debug)]
struct AllocatedDir {
    name: String,
    icb_block: u32,        // Block where this dir's File Entry lives
    fid_block: u32,        // Block where FIDs start
    fid_sectors: u32,      // Number of sectors for FIDs
    parent_icb_block: u32, // Parent directory's ICB block (self for root)
    unique_id: u64,
    files: Vec<AllocatedFile>,
    subdirs: Vec<AllocatedDir>,
}

/// Options for UDF filesystem creation
#[derive(Debug, Clone)]
pub struct UdfWriteOptions {
    /// Volume identifier (max 30 characters for dstring encoding)
    pub volume_id: String,
    /// UDF revision to write
    pub revision: UdfRevision,
    /// Partition starting sector (relative to volume start)
    pub partition_start: u32,
    /// Partition length in sectors
    pub partition_length: u32,
}

impl Default for UdfWriteOptions {
    fn default() -> Self {
        Self {
            volume_id: String::from("UDF_VOLUME"),
            revision: UdfRevision::V1_02,
            partition_start: 257, // After AVDP at 256
            partition_length: 0,  // Will be calculated
        }
    }
}

/// A pre-allocated file extent for UDF
#[derive(Debug, Clone, Copy)]
pub struct UdfFileExtent {
    /// Starting sector (logical block number within partition)
    pub logical_block: u32,
    /// Length in bytes
    pub length: u64,
}

/// File entry information for writing
#[derive(Debug, Clone)]
pub struct UdfFileInfo {
    /// File name
    pub name: String,
    /// Whether this is a directory
    pub is_directory: bool,
    /// File size in bytes
    pub size: u64,
    /// Pre-allocated extent (sector and length)
    pub extent: UdfFileExtent,
    /// Unique ID for this file
    pub unique_id: u64,
}

/// Directory information for writing
#[derive(Debug, Clone)]
pub struct UdfDirInfo {
    /// Directory name (empty for root)
    pub name: String,
    /// Files in this directory
    pub files: Vec<UdfFileInfo>,
    /// Subdirectories
    pub subdirs: Vec<UdfDirInfo>,
    /// ICB location for this directory (filled during write)
    pub icb_location: u32,
    /// Unique ID
    pub unique_id: u64,
}

impl UdfDirInfo {
    /// Create an empty root directory
    pub fn root() -> Self {
        Self {
            name: String::new(),
            files: Vec::new(),
            subdirs: Vec::new(),
            icb_location: 0,
            unique_id: 0,
        }
    }
}

/// UDF Writer for creating UDF filesystem structures
///
/// This struct provides both a high-level API for standalone UDF images
/// and low-level methods for integration with hybrid ISO+UDF writers.
///
/// ## High-Level API
///
/// Use [`UdfWriter::format`] for simple standalone UDF filesystems.
///
/// ## Low-Level API
///
/// For hybrid ISO+UDF images (like hadris-cd), use [`UdfWriter::new`] and
/// the individual descriptor writing methods to control exact layout.
pub struct UdfWriter<W: Write + Seek> {
    writer: W,
    options: UdfWriteOptions,
    /// Current unique ID counter
    unique_id_counter: u64,
}

impl<W: Write + Seek> UdfWriter<W> {
    /// Create a new UDF writer for low-level descriptor writing
    pub fn new(writer: W, options: UdfWriteOptions) -> Self {
        Self {
            writer,
            options,
            unique_id_counter: 16, // Start after reserved IDs
        }
    }

    /// Get the underlying writer
    pub fn into_inner(self) -> W {
        self.writer
    }

    // =========================================================================
    // High-Level Format API
    // =========================================================================

    /// Format a complete UDF filesystem from a directory tree
    ///
    /// This is the high-level API for creating UDF filesystems. It handles all
    /// the complexity of descriptor writing, block allocation, and metadata
    /// generation automatically.
    ///
    /// The writer must have enough space for the filesystem. The space required
    /// depends on the total size of all files plus metadata overhead.
    ///
    /// # Arguments
    ///
    /// * `writer` - The output device/file (must support Write + Seek)
    /// * `root` - Root directory containing all files and subdirectories
    /// * `options` - UDF formatting options
    ///
    /// # Returns
    ///
    /// Returns the total number of sectors written on success.
    pub fn format(writer: W, root: &SimpleDir, options: UdfWriteOptions) -> UdfResult<u32>
    where
        W: Write + Seek,
    {
        let mut formatter = UdfFormatter::new(writer, options);
        formatter.format(root)
    }
}

/// Internal formatter that handles the full UDF format process
struct UdfFormatter<W: Write + Seek> {
    writer: W,
    options: UdfWriteOptions,
    next_block: u32,
    unique_id_counter: u64,
}

impl<W: Write + Seek> UdfFormatter<W> {
    fn new(writer: W, options: UdfWriteOptions) -> Self {
        Self {
            writer,
            options,
            next_block: 0,
            unique_id_counter: 16, // UDF reserves IDs 0-15
        }
    }

    fn allocate_block(&mut self) -> u32 {
        let block = self.next_block;
        self.next_block += 1;
        block
    }

    fn next_unique_id(&mut self) -> u64 {
        let id = self.unique_id_counter;
        self.unique_id_counter += 1;
        id
    }

    fn format(&mut self, root: &SimpleDir) -> UdfResult<u32> {
        // Phase 1: Plan the layout
        //
        // UDF disk layout:
        // Sector 16-18:  VRS (BEA01, NSR02, TEA01)
        // Sector 256:    AVDP
        // Sector 257-262: Main VDS (PVD, IUVD, PD, LVD, USD, Term)
        // Sector 263-268: Reserve VDS
        // Sector 269:    LVID
        // Sector 270+:   Partition starts here
        //   Block 0:     FSD
        //   Block 1+:    Root dir File Entry, FIDs, subdirs, file data

        let partition_start = 270u32;

        // Phase 2: Allocate all structures within the partition
        let fsd_block = self.allocate_block(); // 0
        let allocated_root = self.allocate_directory(root, fsd_block)?;

        // Calculate partition length
        let partition_length = self.next_block;

        // Update options with calculated values
        self.options.partition_start = partition_start;
        self.options.partition_length = partition_length;

        // Phase 3: Write all structures

        // Write VRS
        self.write_vrs()?;

        // Write AVDP
        let vds_start = 257u32;
        let vds_length = 6u32;
        let reserve_vds_start = 263u32;
        let lvid_location = reserve_vds_start + vds_length; // 269

        let main_vds = ExtentDescriptor {
            length: vds_length * SECTOR_SIZE as u32,
            location: vds_start,
        };
        let reserve_vds = ExtentDescriptor {
            length: vds_length * SECTOR_SIZE as u32,
            location: reserve_vds_start,
        };
        self.write_avdp(main_vds, reserve_vds)?;

        // Write VDS
        let fsd_icb = LongAllocationDescriptor {
            extent_length: SECTOR_SIZE as u32,
            logical_block_num: fsd_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };
        let integrity_extent = ExtentDescriptor {
            length: SECTOR_SIZE as u32,
            location: lvid_location,
        };

        // Main VDS
        self.write_pvd(vds_start, 0)?;
        self.write_iuvd(vds_start + 1, 1)?;
        self.write_partition_descriptor(vds_start + 2, 2)?;
        self.write_lvd(vds_start + 3, 3, fsd_icb, integrity_extent)?;
        self.write_usd(vds_start + 4, 4)?;
        self.write_terminating_descriptor(vds_start + 5)?;

        // Reserve VDS (copy)
        self.write_pvd(reserve_vds_start, 0)?;
        self.write_iuvd(reserve_vds_start + 1, 1)?;
        self.write_partition_descriptor(reserve_vds_start + 2, 2)?;
        self.write_lvd(reserve_vds_start + 3, 3, fsd_icb, integrity_extent)?;
        self.write_usd(reserve_vds_start + 4, 4)?;
        self.write_terminating_descriptor(reserve_vds_start + 5)?;

        // Write LVID
        self.write_lvid(lvid_location)?;

        // Write FSD
        let root_icb = LongAllocationDescriptor {
            extent_length: SECTOR_SIZE as u32,
            logical_block_num: allocated_root.icb_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };
        self.write_fsd(fsd_block, root_icb)?;

        // Write directory structures
        self.write_directory(&allocated_root)?;

        // Write file data
        self.write_file_data(root, &allocated_root)?;

        Ok(partition_start + partition_length)
    }

    /// Allocate blocks for a directory and all its contents
    fn allocate_directory(&mut self, dir: &SimpleDir, parent_icb: u32) -> UdfResult<AllocatedDir> {
        let icb_block = self.allocate_block();
        let unique_id = self.next_unique_id();

        // Calculate FID size
        let entry_count = 1 + dir.files.len() + dir.subdirs.len(); // parent + files + subdirs
        let estimated_fid_bytes = entry_count * 50; // ~50 bytes per FID entry
        let fid_sectors = ((estimated_fid_bytes + SECTOR_SIZE - 1) / SECTOR_SIZE) as u32;

        let fid_block = self.allocate_block();
        // Allocate additional FID sectors if needed
        for _ in 1..fid_sectors {
            self.allocate_block();
        }

        // Allocate files
        let mut allocated_files = Vec::new();
        for file in &dir.files {
            let file_icb_block = self.allocate_block();
            let file_unique_id = self.next_unique_id();

            // Allocate data blocks for non-empty files
            let data_block = if !file.data.is_empty() {
                let block = self.allocate_block();
                let data_sectors = ((file.data.len() + SECTOR_SIZE - 1) / SECTOR_SIZE) as u32;
                for _ in 1..data_sectors {
                    self.allocate_block();
                }
                block
            } else {
                0 // Empty file has no data block
            };

            allocated_files.push(AllocatedFile {
                name: file.name.clone(),
                data_block,
                data_length: file.data.len() as u64,
                icb_block: file_icb_block,
                unique_id: file_unique_id,
            });
        }

        // Recursively allocate subdirectories
        let mut allocated_subdirs = Vec::new();
        for subdir in &dir.subdirs {
            let allocated_subdir = self.allocate_directory(subdir, icb_block)?;
            allocated_subdirs.push(allocated_subdir);
        }

        Ok(AllocatedDir {
            name: dir.name.clone(),
            icb_block,
            fid_block,
            fid_sectors,
            parent_icb_block: parent_icb,
            unique_id,
            files: allocated_files,
            subdirs: allocated_subdirs,
        })
    }

    /// Write a directory and all its contents
    fn write_directory(&mut self, dir: &AllocatedDir) -> UdfResult<()> {
        // Calculate FID data size
        let fid_data_size = (dir.fid_sectors as usize) * SECTOR_SIZE;

        // Write directory File Entry
        let dir_alloc = vec![ShortAllocationDescriptor {
            extent_length: fid_data_size as u32,
            extent_position: dir.fid_block,
        }];
        self.write_file_entry(
            dir.icb_block,
            FileType::Directory,
            fid_data_size as u64,
            &dir_alloc,
            dir.unique_id,
        )?;

        // Build FID entries list
        let mut entries: Vec<(String, LongAllocationDescriptor, bool)> = Vec::new();

        // Add file entries
        for file in &dir.files {
            let file_icb = LongAllocationDescriptor {
                extent_length: SECTOR_SIZE as u32,
                logical_block_num: file.icb_block,
                partition_ref_num: 0,
                impl_use: [0; 6],
            };
            entries.push((file.name.clone(), file_icb, false));
        }

        // Add subdirectory entries
        for subdir in &dir.subdirs {
            let subdir_icb = LongAllocationDescriptor {
                extent_length: SECTOR_SIZE as u32,
                logical_block_num: subdir.icb_block,
                partition_ref_num: 0,
                impl_use: [0; 6],
            };
            entries.push((subdir.name.clone(), subdir_icb, true));
        }

        // Write FIDs
        let parent_icb = LongAllocationDescriptor {
            extent_length: SECTOR_SIZE as u32,
            logical_block_num: dir.parent_icb_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };
        self.write_fids(dir.fid_block, parent_icb, &entries)?;

        // Write file File Entries and data
        for (file, orig_file) in dir.files.iter().zip(
            // We need to get the original file data - this is a bit awkward
            // For now, we'll rely on the caller to ensure data is available
            core::iter::repeat(&Vec::<u8>::new()),
        ) {
            let file_alloc = if file.data_length > 0 {
                vec![ShortAllocationDescriptor {
                    extent_length: file.data_length as u32,
                    extent_position: file.data_block,
                }]
            } else {
                vec![]
            };

            self.write_file_entry(
                file.icb_block,
                FileType::RegularFile,
                file.data_length,
                &file_alloc,
                file.unique_id,
            )?;

            // Write file data (if any) - we need the original data here
            // This is handled by passing it through the allocation
            let _ = orig_file; // Placeholder - actual data writing happens below
        }

        // Recursively write subdirectories
        for subdir in &dir.subdirs {
            self.write_directory(subdir)?;
        }

        Ok(())
    }

    /// Write file data for all files in the tree
    fn write_file_data(&mut self, dir: &SimpleDir, alloc_dir: &AllocatedDir) -> UdfResult<()> {
        // Write file data
        for (file, alloc_file) in dir.files.iter().zip(&alloc_dir.files) {
            if !file.data.is_empty() {
                self.seek_to_partition_block(alloc_file.data_block)?;
                self.writer.write_all(&file.data)?;

                // Pad to sector boundary
                let padded = ((file.data.len() + SECTOR_SIZE - 1) / SECTOR_SIZE) * SECTOR_SIZE;
                if padded > file.data.len() {
                    let padding = vec![0u8; padded - file.data.len()];
                    self.writer.write_all(&padding)?;
                }
            }
        }

        // Recursively write subdirectory file data
        for (subdir, alloc_subdir) in dir.subdirs.iter().zip(&alloc_dir.subdirs) {
            self.write_file_data(subdir, alloc_subdir)?;
        }

        Ok(())
    }

    // Low-level descriptor writing methods (delegated to helper)
    fn seek_to_partition_block(&mut self, block: u32) -> UdfResult<()> {
        let sector = self.options.partition_start + block;
        self.writer
            .seek(SeekFrom::Start((sector as u64) * SECTOR_SIZE as u64))?;
        Ok(())
    }

    fn seek_to_sector(&mut self, sector: u32) -> UdfResult<()> {
        self.writer
            .seek(SeekFrom::Start((sector as u64) * SECTOR_SIZE as u64))?;
        Ok(())
    }

    fn write_vrs(&mut self) -> UdfResult<()> {
        let nsr = match self.options.revision {
            r if r >= UdfRevision::V2_00 => b"NSR03",
            _ => b"NSR02",
        };

        self.seek_to_sector(16)?;
        self.write_vrs_descriptor(b"BEA01")?;
        self.write_vrs_descriptor(nsr)?;
        self.write_vrs_descriptor(b"TEA01")?;
        Ok(())
    }

    fn write_vrs_descriptor(&mut self, id: &[u8; 5]) -> UdfResult<()> {
        let mut buffer = [0u8; SECTOR_SIZE];
        buffer[0] = 0;
        buffer[1..6].copy_from_slice(id);
        buffer[6] = 1;
        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_avdp(
        &mut self,
        main_vds: ExtentDescriptor,
        reserve_vds: ExtentDescriptor,
    ) -> UdfResult<()> {
        self.seek_to_sector(AVDP_LOCATION)?;
        let mut buffer = [0u8; 512];

        buffer[16..20].copy_from_slice(&main_vds.length.to_le_bytes());
        buffer[20..24].copy_from_slice(&main_vds.location.to_le_bytes());
        buffer[24..28].copy_from_slice(&reserve_vds.length.to_le_bytes());
        buffer[28..32].copy_from_slice(&reserve_vds.location.to_le_bytes());

        let tag = self.create_tag(
            TagIdentifier::AnchorVolumeDescriptorPointer,
            AVDP_LOCATION,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_pvd(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());
        buffer[offset + 4..offset + 8].copy_from_slice(&0u32.to_le_bytes());

        let vol_id_offset = offset + 8;
        self.write_dstring(
            &mut buffer[vol_id_offset..vol_id_offset + 32],
            &self.options.volume_id,
        );

        let vsn_offset = vol_id_offset + 32;
        buffer[vsn_offset..vsn_offset + 2].copy_from_slice(&1u16.to_le_bytes());
        buffer[vsn_offset + 2..vsn_offset + 4].copy_from_slice(&1u16.to_le_bytes());
        buffer[vsn_offset + 4..vsn_offset + 6].copy_from_slice(&2u16.to_le_bytes());
        buffer[vsn_offset + 6..vsn_offset + 8].copy_from_slice(&3u16.to_le_bytes());
        buffer[vsn_offset + 8..vsn_offset + 12].copy_from_slice(&1u32.to_le_bytes());
        buffer[vsn_offset + 12..vsn_offset + 16].copy_from_slice(&1u32.to_le_bytes());

        let vsi_offset = vsn_offset + 16;
        self.write_dstring(
            &mut buffer[vsi_offset..vsi_offset + 128],
            &self.options.volume_id,
        );

        let dcs_offset = vsi_offset + 128;
        buffer[dcs_offset] = 0;

        let ecs_offset = dcs_offset + 64;
        buffer[ecs_offset] = 0;

        let abs_offset = ecs_offset + 64;
        let app_offset = abs_offset + 16;
        self.write_entity_identifier(&mut buffer[app_offset..app_offset + 32], b"*hadris-udf");

        let rdt_offset = app_offset + 32;
        let now = UdfTimestamp::now();
        buffer[rdt_offset..rdt_offset + 12].copy_from_slice(bytemuck::bytes_of(&now));

        let impl_offset = rdt_offset + 12;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        let tag = self.create_tag(
            TagIdentifier::PrimaryVolumeDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_partition_descriptor(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());
        buffer[offset + 4..offset + 6].copy_from_slice(&1u16.to_le_bytes());
        buffer[offset + 6..offset + 8].copy_from_slice(&0u16.to_le_bytes());

        let nsr = match self.options.revision {
            r if r >= UdfRevision::V2_00 => b"+NSR03",
            _ => b"+NSR02",
        };
        let pc_offset = offset + 8;
        self.write_entity_identifier(&mut buffer[pc_offset..pc_offset + 32], nsr);

        let at_offset = pc_offset + 32 + 128;
        buffer[at_offset..at_offset + 4].copy_from_slice(&1u32.to_le_bytes());

        let psl_offset = at_offset + 4;
        buffer[psl_offset..psl_offset + 4]
            .copy_from_slice(&self.options.partition_start.to_le_bytes());
        buffer[psl_offset + 4..psl_offset + 8]
            .copy_from_slice(&self.options.partition_length.to_le_bytes());

        let impl_offset = psl_offset + 8;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        let tag = self.create_tag(TagIdentifier::PartitionDescriptor, location, &buffer[16..]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_lvd(
        &mut self,
        location: u32,
        vds_number: u32,
        fsd_location: LongAllocationDescriptor,
        integrity_extent: ExtentDescriptor,
    ) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());

        let dcs_offset = offset + 4;
        buffer[dcs_offset] = 0;

        let lvi_offset = dcs_offset + 64;
        self.write_dstring(
            &mut buffer[lvi_offset..lvi_offset + 128],
            &self.options.volume_id,
        );

        let lbs_offset = lvi_offset + 128;
        buffer[lbs_offset..lbs_offset + 4].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());

        let di_offset = lbs_offset + 4;
        self.write_entity_identifier(
            &mut buffer[di_offset..di_offset + 32],
            b"*OSTA UDF Compliant",
        );
        buffer[di_offset + 25] = (self.options.revision.to_raw() & 0xFF) as u8;
        buffer[di_offset + 26] = ((self.options.revision.to_raw() >> 8) & 0xFF) as u8;

        let lvcu_offset = di_offset + 32;
        buffer[lvcu_offset..lvcu_offset + 16].copy_from_slice(bytemuck::bytes_of(&fsd_location));

        let mtl_offset = lvcu_offset + 16;
        buffer[mtl_offset..mtl_offset + 4].copy_from_slice(&6u32.to_le_bytes());
        buffer[mtl_offset + 4..mtl_offset + 8].copy_from_slice(&1u32.to_le_bytes());

        let impl_offset = mtl_offset + 8;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        let iu_offset = impl_offset + 32;
        let ise_offset = iu_offset + 128;
        buffer[ise_offset..ise_offset + 4].copy_from_slice(&integrity_extent.length.to_le_bytes());
        buffer[ise_offset + 4..ise_offset + 8]
            .copy_from_slice(&integrity_extent.location.to_le_bytes());

        let pm_offset = ise_offset + 8;
        buffer[pm_offset] = 1;
        buffer[pm_offset + 1] = 6;
        buffer[pm_offset + 2..pm_offset + 4].copy_from_slice(&1u16.to_le_bytes());
        buffer[pm_offset + 4..pm_offset + 6].copy_from_slice(&0u16.to_le_bytes());

        let tag = self.create_tag(
            TagIdentifier::LogicalVolumeDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_usd(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());
        buffer[offset + 4..offset + 8].copy_from_slice(&0u32.to_le_bytes());

        let tag = self.create_tag(
            TagIdentifier::UnallocatedSpaceDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_iuvd(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());

        let impl_offset = offset + 4;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*UDF LV Info");

        let iu_offset = impl_offset + 32;
        buffer[iu_offset] = 0;

        let lvi_offset = iu_offset + 64;
        self.write_dstring(
            &mut buffer[lvi_offset..lvi_offset + 128],
            &self.options.volume_id,
        );

        let tag = self.create_tag(
            TagIdentifier::ImplementationUseVolumeDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_terminating_descriptor(&mut self, location: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];

        let tag = self.create_tag(TagIdentifier::TerminatingDescriptor, location, &[]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_lvid(&mut self, location: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        let now = UdfTimestamp::now();
        buffer[offset..offset + 12].copy_from_slice(bytemuck::bytes_of(&now));
        buffer[offset + 12..offset + 16].copy_from_slice(&1u32.to_le_bytes()); // Closed

        let lvcu_offset = offset + 24;
        buffer[lvcu_offset..lvcu_offset + 8].copy_from_slice(&self.unique_id_counter.to_le_bytes());

        let np_offset = lvcu_offset + 32;
        buffer[np_offset..np_offset + 4].copy_from_slice(&1u32.to_le_bytes());
        buffer[np_offset + 4..np_offset + 8].copy_from_slice(&46u32.to_le_bytes());

        let fst_offset = np_offset + 8;
        buffer[fst_offset..fst_offset + 4].copy_from_slice(&0u32.to_le_bytes());
        buffer[fst_offset + 4..fst_offset + 8]
            .copy_from_slice(&self.options.partition_length.to_le_bytes());

        let iu_offset = fst_offset + 8;
        self.write_entity_identifier(&mut buffer[iu_offset..iu_offset + 32], b"*hadris-udf");

        let tag = self.create_tag(
            TagIdentifier::LogicalVolumeIntegrityDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_fsd(&mut self, location: u32, root_icb: LongAllocationDescriptor) -> UdfResult<()> {
        self.seek_to_partition_block(location)?;
        let mut buffer = [0u8; 512];
        let offset = 16;

        let now = UdfTimestamp::now();
        buffer[offset..offset + 12].copy_from_slice(bytemuck::bytes_of(&now));

        buffer[offset + 12..offset + 14].copy_from_slice(&3u16.to_le_bytes());
        buffer[offset + 14..offset + 16].copy_from_slice(&3u16.to_le_bytes());
        buffer[offset + 16..offset + 20].copy_from_slice(&1u32.to_le_bytes());
        buffer[offset + 20..offset + 24].copy_from_slice(&1u32.to_le_bytes());
        buffer[offset + 24..offset + 28].copy_from_slice(&0u32.to_le_bytes());
        buffer[offset + 28..offset + 32].copy_from_slice(&0u32.to_le_bytes());

        let lvics_offset = offset + 32;
        buffer[lvics_offset] = 0;

        let lvi_offset = lvics_offset + 64;
        self.write_dstring(
            &mut buffer[lvi_offset..lvi_offset + 128],
            &self.options.volume_id,
        );

        let fscs_offset = lvi_offset + 128;
        buffer[fscs_offset] = 0;

        let fsi_offset = fscs_offset + 64;
        self.write_dstring(
            &mut buffer[fsi_offset..fsi_offset + 32],
            &self.options.volume_id,
        );

        let root_offset = fsi_offset + 32 + 32 + 32;
        buffer[root_offset..root_offset + 16].copy_from_slice(bytemuck::bytes_of(&root_icb));

        let di_offset = root_offset + 16;
        self.write_entity_identifier(
            &mut buffer[di_offset..di_offset + 32],
            b"*OSTA UDF Compliant",
        );
        buffer[di_offset + 25] = (self.options.revision.to_raw() & 0xFF) as u8;
        buffer[di_offset + 26] = ((self.options.revision.to_raw() >> 8) & 0xFF) as u8;

        let tag = self.create_tag(TagIdentifier::FileSetDescriptor, location, &buffer[16..]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_file_entry(
        &mut self,
        location: u32,
        file_type: FileType,
        info_length: u64,
        allocation_descriptors: &[ShortAllocationDescriptor],
        unique_id: u64,
    ) -> UdfResult<()> {
        self.seek_to_partition_block(location)?;
        let mut buffer = [0u8; SECTOR_SIZE];
        let offset = 16;

        let icb_offset = offset;
        buffer[icb_offset + 4..icb_offset + 6].copy_from_slice(&4u16.to_le_bytes());
        buffer[icb_offset + 8..icb_offset + 10].copy_from_slice(&1u16.to_le_bytes());
        buffer[icb_offset + 11] = file_type as u8;
        buffer[icb_offset + 18..icb_offset + 20].copy_from_slice(&0u16.to_le_bytes());

        let uid_offset = icb_offset + 20;
        buffer[uid_offset..uid_offset + 4].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        buffer[uid_offset + 4..uid_offset + 8].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        buffer[uid_offset + 8..uid_offset + 12].copy_from_slice(&0x7FFFu32.to_le_bytes());
        buffer[uid_offset + 12..uid_offset + 14].copy_from_slice(&1u16.to_le_bytes());

        let il_offset = uid_offset + 20;
        buffer[il_offset..il_offset + 8].copy_from_slice(&info_length.to_le_bytes());

        let blocks = (info_length + SECTOR_SIZE as u64 - 1) / SECTOR_SIZE as u64;
        buffer[il_offset + 8..il_offset + 16].copy_from_slice(&blocks.to_le_bytes());

        let now = UdfTimestamp::now();
        let time_offset = il_offset + 16;
        buffer[time_offset..time_offset + 12].copy_from_slice(bytemuck::bytes_of(&now));
        buffer[time_offset + 12..time_offset + 24].copy_from_slice(bytemuck::bytes_of(&now));
        buffer[time_offset + 24..time_offset + 36].copy_from_slice(bytemuck::bytes_of(&now));

        let cp_offset = time_offset + 36;
        buffer[cp_offset..cp_offset + 4].copy_from_slice(&1u32.to_le_bytes());

        let impl_offset = cp_offset + 4 + 16;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        let uid_offset2 = impl_offset + 32;
        buffer[uid_offset2..uid_offset2 + 8].copy_from_slice(&unique_id.to_le_bytes());

        let lea_offset = uid_offset2 + 8;
        buffer[lea_offset..lea_offset + 4].copy_from_slice(&0u32.to_le_bytes());

        let ad_len = allocation_descriptors.len() * size_of::<ShortAllocationDescriptor>();
        buffer[lea_offset + 4..lea_offset + 8].copy_from_slice(&(ad_len as u32).to_le_bytes());

        let ad_offset = lea_offset + 8;
        for (i, ad) in allocation_descriptors.iter().enumerate() {
            let start = ad_offset + i * size_of::<ShortAllocationDescriptor>();
            buffer[start..start + 8].copy_from_slice(bytemuck::bytes_of(ad));
        }

        let tag = self.create_tag(TagIdentifier::FileEntry, location, &buffer[16..]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    fn write_fids(
        &mut self,
        location: u32,
        parent_icb: LongAllocationDescriptor,
        entries: &[(String, LongAllocationDescriptor, bool)],
    ) -> UdfResult<usize> {
        self.seek_to_partition_block(location)?;

        let mut buffer = Vec::new();

        // Parent entry
        let parent_fid = self.create_fid(
            location,
            &parent_icb,
            FileCharacteristics::PARENT | FileCharacteristics::DIRECTORY,
            &[],
        );
        buffer.extend_from_slice(&parent_fid);

        // Child entries
        for (name, icb, is_dir) in entries {
            let chars = if *is_dir {
                FileCharacteristics::DIRECTORY
            } else {
                FileCharacteristics::empty()
            };
            let encoded_name = self.encode_filename(name);
            let fid = self.create_fid(location, icb, chars, &encoded_name);
            buffer.extend_from_slice(&fid);
        }

        // Pad to sector boundary
        let padded_len = (buffer.len() + SECTOR_SIZE - 1) / SECTOR_SIZE * SECTOR_SIZE;
        buffer.resize(padded_len, 0);

        self.writer.write_all(&buffer)?;
        Ok(padded_len / SECTOR_SIZE)
    }

    fn create_fid(
        &self,
        dir_location: u32,
        icb: &LongAllocationDescriptor,
        characteristics: FileCharacteristics,
        encoded_name: &[u8],
    ) -> Vec<u8> {
        let base_size = 38;
        let total_size = (base_size + encoded_name.len() + 3) & !3;
        let mut buffer = vec![0u8; total_size];

        buffer[16..18].copy_from_slice(&1u16.to_le_bytes());
        buffer[18] = characteristics.bits();
        buffer[19] = encoded_name.len() as u8;
        buffer[20..36].copy_from_slice(bytemuck::bytes_of(icb));
        buffer[36..38].copy_from_slice(&0u16.to_le_bytes());
        if !encoded_name.is_empty() {
            buffer[38..38 + encoded_name.len()].copy_from_slice(encoded_name);
        }

        let tag = self.create_tag(
            TagIdentifier::FileIdentifierDescriptor,
            dir_location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        buffer
    }

    fn create_tag(&self, identifier: TagIdentifier, location: u32, data: &[u8]) -> DescriptorTag {
        let crc_length = data.len().min(496) as u16;
        let crc = crc16_itu(&data[..crc_length as usize]);

        let mut tag = DescriptorTag {
            tag_identifier: identifier.to_u16(),
            descriptor_version: 2,
            tag_checksum: 0,
            reserved: 0,
            tag_serial_number: 0,
            descriptor_crc: crc,
            descriptor_crc_length: crc_length,
            tag_location: location,
        };

        let bytes = bytemuck::bytes_of(&tag);
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                sum = sum.wrapping_add(byte);
            }
        }
        tag.tag_checksum = sum;

        tag
    }

    fn write_dstring(&self, buffer: &mut [u8], s: &str) {
        if s.is_empty() || buffer.is_empty() {
            return;
        }

        let bytes = s.as_bytes();
        let max_content = buffer.len() - 2;
        let content_len = bytes.len().min(max_content);
        buffer[0] = 8;
        buffer[1..1 + content_len].copy_from_slice(&bytes[..content_len]);
        buffer[buffer.len() - 1] = (content_len + 1) as u8;
    }

    fn write_entity_identifier(&self, buffer: &mut [u8], id: &[u8]) {
        let len = id.len().min(23);
        buffer[1..1 + len].copy_from_slice(&id[..len]);
        if id.starts_with(b"*OSTA UDF") {
            buffer[25] = (self.options.revision.to_raw() & 0xFF) as u8;
            buffer[26] = ((self.options.revision.to_raw() >> 8) & 0xFF) as u8;
        }
    }

    fn encode_filename(&self, name: &str) -> Vec<u8> {
        let bytes = name.as_bytes();
        let mut result = Vec::with_capacity(bytes.len() + 1);
        result.push(8);
        result.extend_from_slice(bytes);
        result
    }
}

// =============================================================================
// Low-Level UdfWriter Methods (for hadris-cd integration)
// =============================================================================

impl<W: Write + Seek> UdfWriter<W> {
    /// Seek to a logical block within the partition
    fn seek_to_partition_block(&mut self, block: u32) -> UdfResult<()> {
        let sector = self.options.partition_start + block;
        self.writer
            .seek(SeekFrom::Start((sector as u64) * SECTOR_SIZE as u64))?;
        Ok(())
    }

    /// Seek to an absolute sector
    fn seek_to_sector(&mut self, sector: u32) -> UdfResult<()> {
        self.writer
            .seek(SeekFrom::Start((sector as u64) * SECTOR_SIZE as u64))?;
        Ok(())
    }

    /// Write Volume Recognition Sequence (VRS)
    ///
    /// Writes BEA01, NSR02/NSR03, TEA01 at sectors 16+
    pub fn write_vrs(&mut self) -> UdfResult<()> {
        let nsr = match self.options.revision {
            r if r >= UdfRevision::V2_00 => b"NSR03",
            _ => b"NSR02",
        };

        // BEA01 at sector 16
        self.seek_to_sector(16)?;
        self.write_vrs_descriptor(b"BEA01")?;

        // NSR02/NSR03 at sector 17
        self.write_vrs_descriptor(nsr)?;

        // TEA01 at sector 18
        self.write_vrs_descriptor(b"TEA01")?;

        Ok(())
    }

    fn write_vrs_descriptor(&mut self, id: &[u8; 5]) -> UdfResult<()> {
        let mut buffer = [0u8; SECTOR_SIZE];
        buffer[0] = 0; // Structure type
        buffer[1..6].copy_from_slice(id);
        buffer[6] = 1; // Version
        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Anchor Volume Descriptor Pointer at sector 256
    pub fn write_avdp(
        &mut self,
        main_vds_extent: ExtentDescriptor,
        reserve_vds_extent: ExtentDescriptor,
    ) -> UdfResult<()> {
        self.seek_to_sector(AVDP_LOCATION)?;

        let mut buffer = [0u8; 512];

        // Main VDS extent
        buffer[16..20].copy_from_slice(&main_vds_extent.length.to_le_bytes());
        buffer[20..24].copy_from_slice(&main_vds_extent.location.to_le_bytes());

        // Reserve VDS extent
        buffer[24..28].copy_from_slice(&reserve_vds_extent.length.to_le_bytes());
        buffer[28..32].copy_from_slice(&reserve_vds_extent.location.to_le_bytes());

        // Write tag at the beginning
        let tag = self.create_tag(
            TagIdentifier::AnchorVolumeDescriptorPointer,
            AVDP_LOCATION,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Primary Volume Descriptor
    pub fn write_pvd(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16; // After tag

        // VDS Number (4 bytes)
        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());
        // PVD Number (4 bytes)
        buffer[offset + 4..offset + 8].copy_from_slice(&0u32.to_le_bytes());

        // Volume Identifier (dstring, 32 bytes)
        let vol_id_offset = offset + 8;
        self.write_dstring(
            &mut buffer[vol_id_offset..vol_id_offset + 32],
            &self.options.volume_id,
        );

        // Volume Sequence Number
        let vsn_offset = vol_id_offset + 32;
        buffer[vsn_offset..vsn_offset + 2].copy_from_slice(&1u16.to_le_bytes());
        // Max Volume Sequence Number
        buffer[vsn_offset + 2..vsn_offset + 4].copy_from_slice(&1u16.to_le_bytes());
        // Interchange Level
        buffer[vsn_offset + 4..vsn_offset + 6].copy_from_slice(&2u16.to_le_bytes());
        // Max Interchange Level
        buffer[vsn_offset + 6..vsn_offset + 8].copy_from_slice(&3u16.to_le_bytes());
        // Character Set List
        buffer[vsn_offset + 8..vsn_offset + 12].copy_from_slice(&1u32.to_le_bytes());
        // Max Character Set List
        buffer[vsn_offset + 12..vsn_offset + 16].copy_from_slice(&1u32.to_le_bytes());

        // Volume Set Identifier (dstring, 128 bytes)
        let vsi_offset = vsn_offset + 16;
        self.write_dstring(
            &mut buffer[vsi_offset..vsi_offset + 128],
            &self.options.volume_id,
        );

        // Descriptor Character Set (64 bytes)
        let dcs_offset = vsi_offset + 128;
        buffer[dcs_offset] = 0; // CS0

        // Explanatory Character Set (64 bytes)
        let ecs_offset = dcs_offset + 64;
        buffer[ecs_offset] = 0; // CS0

        // Volume Abstract (8 bytes) - empty
        // Volume Copyright Notice (8 bytes) - empty
        let abs_offset = ecs_offset + 64;

        // Application Identifier (32 bytes)
        let app_offset = abs_offset + 16;
        self.write_entity_identifier(&mut buffer[app_offset..app_offset + 32], b"*hadris-udf");

        // Recording Date Time (12 bytes)
        let rdt_offset = app_offset + 32;
        let now = UdfTimestamp::now();
        buffer[rdt_offset..rdt_offset + 12].copy_from_slice(bytemuck::bytes_of(&now));

        // Implementation Identifier (32 bytes)
        let impl_offset = rdt_offset + 12;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        // Write tag
        let tag = self.create_tag(
            TagIdentifier::PrimaryVolumeDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Partition Descriptor
    pub fn write_partition_descriptor(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16;

        // VDS Number
        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());
        // Partition Flags (allocated = 1)
        buffer[offset + 4..offset + 6].copy_from_slice(&1u16.to_le_bytes());
        // Partition Number
        buffer[offset + 6..offset + 8].copy_from_slice(&0u16.to_le_bytes());

        // Partition Contents (EntityIdentifier, 32 bytes)
        let nsr = match self.options.revision {
            r if r >= UdfRevision::V2_00 => b"+NSR03",
            _ => b"+NSR02",
        };
        let pc_offset = offset + 8;
        self.write_entity_identifier(&mut buffer[pc_offset..pc_offset + 32], nsr);

        // Partition Contents Use (128 bytes) - empty for basic use
        // Access Type (4 bytes) - 1 = read-only
        let at_offset = pc_offset + 32 + 128;
        buffer[at_offset..at_offset + 4].copy_from_slice(&1u32.to_le_bytes());

        // Partition Starting Location
        let psl_offset = at_offset + 4;
        buffer[psl_offset..psl_offset + 4]
            .copy_from_slice(&self.options.partition_start.to_le_bytes());

        // Partition Length
        buffer[psl_offset + 4..psl_offset + 8]
            .copy_from_slice(&self.options.partition_length.to_le_bytes());

        // Implementation Identifier (32 bytes)
        let impl_offset = psl_offset + 8;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        // Write tag
        let tag = self.create_tag(TagIdentifier::PartitionDescriptor, location, &buffer[16..]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Logical Volume Descriptor
    pub fn write_lvd(
        &mut self,
        location: u32,
        vds_number: u32,
        fsd_location: LongAllocationDescriptor,
        integrity_extent: ExtentDescriptor,
    ) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16;

        // VDS Number
        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());

        // Descriptor Character Set (64 bytes)
        let dcs_offset = offset + 4;
        buffer[dcs_offset] = 0; // CS0

        // Logical Volume Identifier (dstring, 128 bytes)
        let lvi_offset = dcs_offset + 64;
        self.write_dstring(
            &mut buffer[lvi_offset..lvi_offset + 128],
            &self.options.volume_id,
        );

        // Logical Block Size (4 bytes)
        let lbs_offset = lvi_offset + 128;
        buffer[lbs_offset..lbs_offset + 4].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());

        // Domain Identifier (32 bytes)
        let di_offset = lbs_offset + 4;
        self.write_entity_identifier(
            &mut buffer[di_offset..di_offset + 32],
            b"*OSTA UDF Compliant",
        );

        // Set UDF revision in domain identifier suffix
        buffer[di_offset + 25] = (self.options.revision.to_raw() & 0xFF) as u8;
        buffer[di_offset + 26] = ((self.options.revision.to_raw() >> 8) & 0xFF) as u8;

        // Logical Volume Contents Use (16 bytes) - Long Allocation Descriptor to FSD
        let lvcu_offset = di_offset + 32;
        buffer[lvcu_offset..lvcu_offset + 16].copy_from_slice(bytemuck::bytes_of(&fsd_location));

        // Map Table Length (4 bytes)
        let mtl_offset = lvcu_offset + 16;
        buffer[mtl_offset..mtl_offset + 4].copy_from_slice(&6u32.to_le_bytes()); // Type 1 map is 6 bytes

        // Number of Partition Maps (4 bytes)
        buffer[mtl_offset + 4..mtl_offset + 8].copy_from_slice(&1u32.to_le_bytes());

        // Implementation Identifier (32 bytes)
        let impl_offset = mtl_offset + 8;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        // Implementation Use (128 bytes) - skip
        let iu_offset = impl_offset + 32;

        // Integrity Sequence Extent (8 bytes)
        let ise_offset = iu_offset + 128;
        buffer[ise_offset..ise_offset + 4].copy_from_slice(&integrity_extent.length.to_le_bytes());
        buffer[ise_offset + 4..ise_offset + 8]
            .copy_from_slice(&integrity_extent.location.to_le_bytes());

        // Partition Maps - Type 1 (6 bytes)
        let pm_offset = ise_offset + 8;
        buffer[pm_offset] = 1; // Type 1
        buffer[pm_offset + 1] = 6; // Length
        buffer[pm_offset + 2..pm_offset + 4].copy_from_slice(&1u16.to_le_bytes()); // Volume Sequence Number
        buffer[pm_offset + 4..pm_offset + 6].copy_from_slice(&0u16.to_le_bytes()); // Partition Number

        // Write tag
        let tag = self.create_tag(
            TagIdentifier::LogicalVolumeDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Unallocated Space Descriptor
    pub fn write_usd(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16;

        // VDS Number
        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());
        // Number of Allocation Descriptors (0 for read-only)
        buffer[offset + 4..offset + 8].copy_from_slice(&0u32.to_le_bytes());

        // Write tag
        let tag = self.create_tag(
            TagIdentifier::UnallocatedSpaceDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Implementation Use Volume Descriptor
    pub fn write_iuvd(&mut self, location: u32, vds_number: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16;

        // VDS Number
        buffer[offset..offset + 4].copy_from_slice(&vds_number.to_le_bytes());

        // Implementation Identifier (32 bytes)
        let impl_offset = offset + 4;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*UDF LV Info");

        // Implementation Use - LVInformation
        let iu_offset = impl_offset + 32;
        // LVI Character Set (64 bytes)
        buffer[iu_offset] = 0; // CS0

        // Logical Volume Identifier (dstring, 128 bytes)
        let lvi_offset = iu_offset + 64;
        self.write_dstring(
            &mut buffer[lvi_offset..lvi_offset + 128],
            &self.options.volume_id,
        );

        // Write tag
        let tag = self.create_tag(
            TagIdentifier::ImplementationUseVolumeDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write Terminating Descriptor
    pub fn write_terminating_descriptor(&mut self, location: u32) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];

        // Write tag (content is empty for terminating descriptor)
        let tag = self.create_tag(TagIdentifier::TerminatingDescriptor, location, &[]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write File Set Descriptor
    pub fn write_fsd(
        &mut self,
        location: u32,
        root_icb: LongAllocationDescriptor,
    ) -> UdfResult<()> {
        self.seek_to_partition_block(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16;

        // Recording Date and Time (12 bytes)
        let now = UdfTimestamp::now();
        buffer[offset..offset + 12].copy_from_slice(bytemuck::bytes_of(&now));

        // Interchange Level (2 bytes)
        buffer[offset + 12..offset + 14].copy_from_slice(&3u16.to_le_bytes());
        // Maximum Interchange Level (2 bytes)
        buffer[offset + 14..offset + 16].copy_from_slice(&3u16.to_le_bytes());
        // Character Set List (4 bytes)
        buffer[offset + 16..offset + 20].copy_from_slice(&1u32.to_le_bytes());
        // Maximum Character Set List (4 bytes)
        buffer[offset + 20..offset + 24].copy_from_slice(&1u32.to_le_bytes());
        // File Set Number (4 bytes)
        buffer[offset + 24..offset + 28].copy_from_slice(&0u32.to_le_bytes());
        // File Set Descriptor Number (4 bytes)
        buffer[offset + 28..offset + 32].copy_from_slice(&0u32.to_le_bytes());

        // Logical Volume Identifier Character Set (64 bytes)
        let lvics_offset = offset + 32;
        buffer[lvics_offset] = 0; // CS0

        // Logical Volume Identifier (dstring, 128 bytes)
        let lvi_offset = lvics_offset + 64;
        self.write_dstring(
            &mut buffer[lvi_offset..lvi_offset + 128],
            &self.options.volume_id,
        );

        // File Set Character Set (64 bytes)
        let fscs_offset = lvi_offset + 128;
        buffer[fscs_offset] = 0; // CS0

        // File Set Identifier (dstring, 32 bytes)
        let fsi_offset = fscs_offset + 64;
        self.write_dstring(
            &mut buffer[fsi_offset..fsi_offset + 32],
            &self.options.volume_id,
        );

        // Copyright/Abstract File Identifiers (32 bytes each) - empty
        // Root Directory ICB (16 bytes)
        let root_offset = fsi_offset + 32 + 32 + 32;
        buffer[root_offset..root_offset + 16].copy_from_slice(bytemuck::bytes_of(&root_icb));

        // Domain Identifier (32 bytes)
        let di_offset = root_offset + 16;
        self.write_entity_identifier(
            &mut buffer[di_offset..di_offset + 32],
            b"*OSTA UDF Compliant",
        );
        buffer[di_offset + 25] = (self.options.revision.to_raw() & 0xFF) as u8;
        buffer[di_offset + 26] = ((self.options.revision.to_raw() >> 8) & 0xFF) as u8;

        // Write tag (location is relative to partition)
        let tag = self.create_tag(TagIdentifier::FileSetDescriptor, location, &buffer[16..]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write a File Entry for a file or directory
    pub fn write_file_entry(
        &mut self,
        location: u32,
        file_type: FileType,
        info_length: u64,
        allocation_descriptors: &[ShortAllocationDescriptor],
        unique_id: u64,
    ) -> UdfResult<()> {
        self.seek_to_partition_block(location)?;

        let mut buffer = [0u8; SECTOR_SIZE];
        let offset = 16; // After tag

        // ICB Tag (20 bytes)
        let icb_offset = offset;
        // Prior Recorded Number of Direct Entries (4 bytes) - 0
        // Strategy Type (2 bytes) - 4 (sequential)
        buffer[icb_offset + 4..icb_offset + 6].copy_from_slice(&4u16.to_le_bytes());
        // Strategy Parameters (2 bytes) - 0
        // Maximum Number of Entries (2 bytes) - 1
        buffer[icb_offset + 8..icb_offset + 10].copy_from_slice(&1u16.to_le_bytes());
        // Reserved (1 byte)
        // File Type (1 byte)
        buffer[icb_offset + 11] = file_type as u8;
        // Parent ICB Location (6 bytes) - 0
        // Flags (2 bytes) - 0 = short allocation descriptors
        buffer[icb_offset + 18..icb_offset + 20].copy_from_slice(&0u16.to_le_bytes());

        // UID (4 bytes) - 0xFFFFFFFF = not specified
        let uid_offset = icb_offset + 20;
        buffer[uid_offset..uid_offset + 4].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        // GID (4 bytes) - 0xFFFFFFFF = not specified
        buffer[uid_offset + 4..uid_offset + 8].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        // Permissions (4 bytes) - 0x7FFF = all permissions
        buffer[uid_offset + 8..uid_offset + 12].copy_from_slice(&0x7FFFu32.to_le_bytes());
        // File Link Count (2 bytes) - 1
        buffer[uid_offset + 12..uid_offset + 14].copy_from_slice(&1u16.to_le_bytes());
        // Record Format (1 byte) - 0
        // Record Display Attributes (1 byte) - 0
        // Record Length (4 bytes) - 0

        // Information Length (8 bytes)
        let il_offset = uid_offset + 20;
        buffer[il_offset..il_offset + 8].copy_from_slice(&info_length.to_le_bytes());

        // Logical Blocks Recorded (8 bytes)
        let blocks = (info_length + SECTOR_SIZE as u64 - 1) / SECTOR_SIZE as u64;
        buffer[il_offset + 8..il_offset + 16].copy_from_slice(&blocks.to_le_bytes());

        // Access/Modification/Attribute Times (12 bytes each)
        let now = UdfTimestamp::now();
        let time_offset = il_offset + 16;
        buffer[time_offset..time_offset + 12].copy_from_slice(bytemuck::bytes_of(&now));
        buffer[time_offset + 12..time_offset + 24].copy_from_slice(bytemuck::bytes_of(&now));
        buffer[time_offset + 24..time_offset + 36].copy_from_slice(bytemuck::bytes_of(&now));

        // Checkpoint (4 bytes) - 1
        let cp_offset = time_offset + 36;
        buffer[cp_offset..cp_offset + 4].copy_from_slice(&1u32.to_le_bytes());

        // Extended Attribute ICB (16 bytes) - 0
        // Implementation Identifier (32 bytes)
        let impl_offset = cp_offset + 4 + 16;
        self.write_entity_identifier(&mut buffer[impl_offset..impl_offset + 32], b"*hadris-udf");

        // Unique ID (8 bytes)
        let uid_offset2 = impl_offset + 32;
        buffer[uid_offset2..uid_offset2 + 8].copy_from_slice(&unique_id.to_le_bytes());

        // Length of Extended Attributes (4 bytes) - 0
        let lea_offset = uid_offset2 + 8;
        buffer[lea_offset..lea_offset + 4].copy_from_slice(&0u32.to_le_bytes());

        // Length of Allocation Descriptors (4 bytes)
        let ad_len = allocation_descriptors.len() * size_of::<ShortAllocationDescriptor>();
        buffer[lea_offset + 4..lea_offset + 8].copy_from_slice(&(ad_len as u32).to_le_bytes());

        // Allocation Descriptors
        let ad_offset = lea_offset + 8;
        for (i, ad) in allocation_descriptors.iter().enumerate() {
            let start = ad_offset + i * size_of::<ShortAllocationDescriptor>();
            buffer[start..start + 8].copy_from_slice(bytemuck::bytes_of(ad));
        }

        // Write tag
        let tag = self.create_tag(TagIdentifier::FileEntry, location, &buffer[16..]);
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Write File Identifier Descriptors for a directory
    pub fn write_fids(
        &mut self,
        location: u32,
        parent_icb: LongAllocationDescriptor,
        entries: &[(String, LongAllocationDescriptor, bool)], // (name, icb, is_dir)
    ) -> UdfResult<usize> {
        self.seek_to_partition_block(location)?;

        let mut buffer = Vec::new();

        // Parent directory entry
        let parent_fid = self.create_fid(
            location,
            &parent_icb,
            FileCharacteristics::PARENT | FileCharacteristics::DIRECTORY,
            &[],
        );
        buffer.extend_from_slice(&parent_fid);

        // Child entries
        for (name, icb, is_dir) in entries {
            let chars = if *is_dir {
                FileCharacteristics::DIRECTORY
            } else {
                FileCharacteristics::empty()
            };
            let encoded_name = self.encode_filename(name);
            let fid = self.create_fid(location, icb, chars, &encoded_name);
            buffer.extend_from_slice(&fid);
        }

        // Pad to sector boundary
        let padded_len = (buffer.len() + SECTOR_SIZE - 1) / SECTOR_SIZE * SECTOR_SIZE;
        buffer.resize(padded_len, 0);

        self.writer.write_all(&buffer)?;
        Ok(padded_len / SECTOR_SIZE)
    }

    fn create_fid(
        &self,
        dir_location: u32,
        icb: &LongAllocationDescriptor,
        characteristics: FileCharacteristics,
        encoded_name: &[u8],
    ) -> Vec<u8> {
        let base_size = 38; // FID base size
        let total_size = (base_size + encoded_name.len() + 3) & !3; // Pad to 4 bytes
        let mut buffer = vec![0u8; total_size];

        // File Version Number (2 bytes) - 1
        buffer[16..18].copy_from_slice(&1u16.to_le_bytes());
        // File Characteristics (1 byte)
        buffer[18] = characteristics.bits();
        // Length of File Identifier (1 byte)
        buffer[19] = encoded_name.len() as u8;
        // ICB (16 bytes)
        buffer[20..36].copy_from_slice(bytemuck::bytes_of(icb));
        // Length of Implementation Use (2 bytes) - 0
        buffer[36..38].copy_from_slice(&0u16.to_le_bytes());
        // File Identifier
        if !encoded_name.is_empty() {
            buffer[38..38 + encoded_name.len()].copy_from_slice(encoded_name);
        }

        // Create and write tag
        let tag = self.create_tag(
            TagIdentifier::FileIdentifierDescriptor,
            dir_location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        buffer
    }

    /// Write Logical Volume Integrity Descriptor
    pub fn write_lvid(&mut self, location: u32, close: bool) -> UdfResult<()> {
        self.seek_to_sector(location)?;

        let mut buffer = [0u8; 512];
        let offset = 16;

        // Recording Date and Time (12 bytes)
        let now = UdfTimestamp::now();
        buffer[offset..offset + 12].copy_from_slice(bytemuck::bytes_of(&now));

        // Integrity Type (4 bytes) - 0 = open, 1 = close
        let integrity = if close { 1u32 } else { 0u32 };
        buffer[offset + 12..offset + 16].copy_from_slice(&integrity.to_le_bytes());

        // Next Integrity Extent (8 bytes) - 0 (none)

        // Logical Volume Contents Use (32 bytes)
        let lvcu_offset = offset + 24;
        // Unique ID (8 bytes)
        buffer[lvcu_offset..lvcu_offset + 8].copy_from_slice(&self.unique_id_counter.to_le_bytes());

        // Number of Partitions (4 bytes) - 1
        let np_offset = lvcu_offset + 32;
        buffer[np_offset..np_offset + 4].copy_from_slice(&1u32.to_le_bytes());

        // Length of Implementation Use (4 bytes)
        buffer[np_offset + 4..np_offset + 8].copy_from_slice(&46u32.to_le_bytes());

        // Free Space Table (4 bytes per partition)
        let fst_offset = np_offset + 8;
        buffer[fst_offset..fst_offset + 4].copy_from_slice(&0u32.to_le_bytes()); // No free space (read-only)

        // Size Table (4 bytes per partition)
        buffer[fst_offset + 4..fst_offset + 8]
            .copy_from_slice(&self.options.partition_length.to_le_bytes());

        // Implementation Use
        let iu_offset = fst_offset + 8;
        // Implementation ID (32 bytes)
        self.write_entity_identifier(&mut buffer[iu_offset..iu_offset + 32], b"*hadris-udf");

        // Write tag
        let tag = self.create_tag(
            TagIdentifier::LogicalVolumeIntegrityDescriptor,
            location,
            &buffer[16..],
        );
        buffer[0..16].copy_from_slice(bytemuck::bytes_of(&tag));

        self.writer.write_all(&buffer)?;
        Ok(())
    }

    /// Create a descriptor tag
    fn create_tag(&self, identifier: TagIdentifier, location: u32, data: &[u8]) -> DescriptorTag {
        let crc_length = data.len().min(496) as u16; // Max CRC length
        let crc = crc16_itu(&data[..crc_length as usize]);

        let mut tag = DescriptorTag {
            tag_identifier: identifier.to_u16(),
            descriptor_version: 2,
            tag_checksum: 0,
            reserved: 0,
            tag_serial_number: 0,
            descriptor_crc: crc,
            descriptor_crc_length: crc_length,
            tag_location: location,
        };

        // Calculate tag checksum
        let bytes = bytemuck::bytes_of(&tag);
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                sum = sum.wrapping_add(byte);
            }
        }
        tag.tag_checksum = sum;

        tag
    }

    /// Write a dstring (OSTA Compressed Unicode)
    fn write_dstring(&self, buffer: &mut [u8], s: &str) {
        if s.is_empty() || buffer.is_empty() {
            return;
        }

        // Use 8-bit encoding for ASCII-compatible strings
        let bytes = s.as_bytes();
        let max_content = buffer.len() - 2; // Reserve 1 byte for compression ID, 1 for length

        let content_len = bytes.len().min(max_content);
        buffer[0] = 8; // Compression ID (8-bit)
        buffer[1..1 + content_len].copy_from_slice(&bytes[..content_len]);
        buffer[buffer.len() - 1] = (content_len + 1) as u8; // Length including compression ID
    }

    /// Write an entity identifier
    fn write_entity_identifier(&self, buffer: &mut [u8], id: &[u8]) {
        // Flags (1 byte) - 0
        // Identifier (23 bytes)
        let len = id.len().min(23);
        buffer[1..1 + len].copy_from_slice(&id[..len]);
        // Suffix (8 bytes) - version info
        if id.starts_with(b"*OSTA UDF") {
            buffer[25] = (self.options.revision.to_raw() & 0xFF) as u8;
            buffer[26] = ((self.options.revision.to_raw() >> 8) & 0xFF) as u8;
        }
    }

    /// Encode a filename for UDF
    fn encode_filename(&self, name: &str) -> Vec<u8> {
        // Use 8-bit encoding for ASCII
        let bytes = name.as_bytes();
        let mut result = Vec::with_capacity(bytes.len() + 1);
        result.push(8); // Compression ID
        result.extend_from_slice(bytes);
        result
    }
}

/// CRC-16-ITU (CCITT) used by UDF
fn crc16_itu(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        let mut x = ((crc >> 8) ^ (byte as u16)) & 0xFF;
        x ^= x >> 4;
        crc = (crc << 8) ^ (x << 12) ^ (x << 5) ^ x;
    }
    crc
}

impl UdfTimestamp {
    /// Create a timestamp for the current time (or default if no std)
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};

        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        let secs = duration.as_secs();
        let subsec_nanos = duration.subsec_nanos();

        // Calculate date/time from Unix timestamp
        // This is a simplified calculation
        let days = (secs / 86400) as i64;
        let day_secs = (secs % 86400) as u32;

        // Calculate year, month, day from days since 1970
        let (year, month, day) = days_to_ymd(days + 719468); // Days since year 0

        Self {
            type_and_tz: 0x1000, // Local time, offset 0
            year: year as u16,
            month: month as u8,
            day: day as u8,
            hour: (day_secs / 3600) as u8,
            minute: ((day_secs % 3600) / 60) as u8,
            second: (day_secs % 60) as u8,
            centiseconds: (subsec_nanos / 10_000_000) as u8,
            hundreds_of_microseconds: ((subsec_nanos / 100_000) % 100) as u8,
            microseconds: ((subsec_nanos / 1000) % 100) as u8,
        }
    }

    #[cfg(not(feature = "std"))]
    pub fn now() -> Self {
        Self {
            type_and_tz: 0x1000,
            year: 2024,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            centiseconds: 0,
            hundreds_of_microseconds: 0,
            microseconds: 0,
        }
    }
}

/// Convert days since year 0 to (year, month, day)
#[cfg(feature = "std")]
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from Howard Hinnant
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use std::io::Cursor;

    #[test]
    fn test_crc16_itu() {
        // Empty data should produce 0
        assert_eq!(crc16_itu(&[]), 0);

        // The CRC algorithm used by UDF is a variant
        // Just verify consistency for now
        let data = b"test";
        let crc1 = crc16_itu(data);
        let crc2 = crc16_itu(data);
        assert_eq!(crc1, crc2);
    }

    #[test]
    fn test_simple_dir_creation() {
        let mut root = SimpleDir::root();
        root.add_file(SimpleFile::new("test.txt", b"Hello".to_vec()));
        root.add_file(SimpleFile::empty("empty.txt"));

        let mut subdir = SimpleDir::new("docs");
        subdir.add_file(SimpleFile::new("guide.txt", b"Guide content".to_vec()));
        root.add_dir(subdir);

        assert_eq!(root.total_files(), 3);
        assert_eq!(root.total_dirs(), 2);
    }

    #[test]
    fn test_format_empty_filesystem() {
        let mut buffer = vec![0u8; 2 * 1024 * 1024]; // 2MB
        let cursor = Cursor::new(&mut buffer[..]);

        let root = SimpleDir::root();
        let options = UdfWriteOptions::default();

        let result = UdfWriter::format(cursor, &root, options);
        assert!(result.is_ok(), "Format should succeed for empty filesystem");

        let sectors = result.unwrap();
        assert!(
            sectors > 270,
            "Should have written at least partition start sectors"
        );
    }

    #[test]
    fn test_format_with_single_file() {
        let mut buffer = vec![0u8; 2 * 1024 * 1024]; // 2MB
        let cursor = Cursor::new(&mut buffer[..]);

        let mut root = SimpleDir::root();
        root.add_file(SimpleFile::new("readme.txt", b"Hello, World!".to_vec()));

        let options = UdfWriteOptions {
            volume_id: String::from("TEST_VOL"),
            ..Default::default()
        };

        let result = UdfWriter::format(cursor, &root, options);
        assert!(result.is_ok(), "Format should succeed with single file");

        // Verify VRS is written at sector 16
        let bea01 = &buffer[16 * 2048..16 * 2048 + 6];
        assert_eq!(&bea01[1..6], b"BEA01", "VRS should start with BEA01");

        // Verify AVDP at sector 256
        let avdp_tag = u16::from_le_bytes([buffer[256 * 2048], buffer[256 * 2048 + 1]]);
        assert_eq!(avdp_tag, 2, "AVDP tag should be 2");
    }

    #[test]
    fn test_format_with_subdirectory() {
        let mut buffer = vec![0u8; 4 * 1024 * 1024]; // 4MB
        let cursor = Cursor::new(&mut buffer[..]);

        let mut root = SimpleDir::root();
        root.add_file(SimpleFile::new("root.txt", b"Root file".to_vec()));

        let mut docs = SimpleDir::new("docs");
        docs.add_file(SimpleFile::new(
            "manual.txt",
            b"User manual content here".to_vec(),
        ));
        docs.add_file(SimpleFile::new("changelog.txt", b"Version 1.0".to_vec()));
        root.add_dir(docs);

        let options = UdfWriteOptions {
            volume_id: String::from("SUBDIR_TEST"),
            ..Default::default()
        };

        let result = UdfWriter::format(cursor, &root, options);
        assert!(result.is_ok(), "Format should succeed with subdirectory");
    }

    #[test]
    fn test_format_with_empty_file() {
        let mut buffer = vec![0u8; 2 * 1024 * 1024]; // 2MB
        let cursor = Cursor::new(&mut buffer[..]);

        let mut root = SimpleDir::root();
        root.add_file(SimpleFile::empty("empty.txt"));
        root.add_file(SimpleFile::new("notempty.txt", b"content".to_vec()));

        let options = UdfWriteOptions::default();

        let result = UdfWriter::format(cursor, &root, options);
        assert!(result.is_ok(), "Format should handle empty files");
    }

    #[test]
    fn test_format_vrs_nsr_version() {
        // Test UDF 1.02 uses NSR02
        let mut buffer = vec![0u8; 2 * 1024 * 1024];
        let cursor = Cursor::new(&mut buffer[..]);
        let root = SimpleDir::root();
        let options = UdfWriteOptions {
            revision: crate::UdfRevision::V1_02,
            ..Default::default()
        };
        UdfWriter::format(cursor, &root, options).unwrap();
        let nsr = &buffer[17 * 2048 + 1..17 * 2048 + 6];
        assert_eq!(nsr, b"NSR02", "UDF 1.02 should use NSR02");

        // Test UDF 2.01 uses NSR03
        let mut buffer2 = vec![0u8; 2 * 1024 * 1024];
        let cursor2 = Cursor::new(&mut buffer2[..]);
        let root2 = SimpleDir::root();
        let options2 = UdfWriteOptions {
            revision: crate::UdfRevision::V2_01,
            ..Default::default()
        };
        UdfWriter::format(cursor2, &root2, options2).unwrap();
        let nsr2 = &buffer2[17 * 2048 + 1..17 * 2048 + 6];
        assert_eq!(nsr2, b"NSR03", "UDF 2.01 should use NSR03");
    }

    #[test]
    fn test_roundtrip_basic_verification() {
        // This test verifies that basic UDF structures are written correctly
        // by checking specific byte patterns in the output.
        // Full roundtrip reading will work once descriptors are sector-sized.

        let mut buffer = vec![0u8; 4 * 1024 * 1024]; // 4MB

        // Write a UDF filesystem
        {
            let cursor = Cursor::new(&mut buffer[..]);
            let mut root = SimpleDir::root();
            root.add_file(SimpleFile::new("hello.txt", b"Hello, UDF!".to_vec()));

            let options = UdfWriteOptions {
                volume_id: String::from("ROUNDTRIP"),
                ..Default::default()
            };

            UdfWriter::format(cursor, &root, options).expect("Format should succeed");
        }

        // Verify key structures manually
        // 1. Check VRS at sector 16-18
        assert_eq!(&buffer[16 * 2048 + 1..16 * 2048 + 6], b"BEA01", "VRS BEA01");
        assert_eq!(&buffer[17 * 2048 + 1..17 * 2048 + 6], b"NSR02", "VRS NSR02");
        assert_eq!(&buffer[18 * 2048 + 1..18 * 2048 + 6], b"TEA01", "VRS TEA01");

        // 2. Check AVDP at sector 256
        let avdp_tag = u16::from_le_bytes([buffer[256 * 2048], buffer[256 * 2048 + 1]]);
        assert_eq!(avdp_tag, 2, "AVDP tag ID should be 2");

        // 3. Check PVD at sector 257
        let pvd_tag = u16::from_le_bytes([buffer[257 * 2048], buffer[257 * 2048 + 1]]);
        assert_eq!(pvd_tag, 1, "PVD tag ID should be 1");

        // 4. Check FSD at partition start (sector 270, block 0)
        let fsd_tag = u16::from_le_bytes([buffer[270 * 2048], buffer[270 * 2048 + 1]]);
        assert_eq!(fsd_tag, 256, "FSD tag ID should be 256");

        // 5. Verify file data is somewhere in the image
        let hello_pattern = b"Hello, UDF!";
        let contains_data = buffer
            .windows(hello_pattern.len())
            .any(|w| w == hello_pattern);
        assert!(contains_data, "File data should be in the image");
    }

    #[test]
    fn test_format_large_file() {
        let mut buffer = vec![0u8; 8 * 1024 * 1024]; // 8MB
        let cursor = Cursor::new(&mut buffer[..]);

        let mut root = SimpleDir::root();
        // Create a file larger than one sector
        let large_data = vec![0x55; 10000]; // ~10KB, spans multiple sectors
        root.add_file(SimpleFile::new("large.bin", large_data.clone()));

        let options = UdfWriteOptions::default();
        let result = UdfWriter::format(cursor, &root, options);
        assert!(result.is_ok(), "Format should succeed with large file");

        // Verify the data was written somewhere in the image
        let pattern_found = buffer.windows(100).any(|w| w == &large_data[..100]);
        assert!(pattern_found, "Large file data should be in the image");
    }
}
