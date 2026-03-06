//! Main writer for creating hybrid ISO+UDF images
//!
//! The `CdWriter` orchestrates the creation of a hybrid image by:
//! 1. Building a shared file tree
//! 2. Laying out file data (shared between both filesystems)
//! 3. Writing ISO 9660 metadata
//! 4. Writing UDF metadata
//! 5. Finalizing the image

use super::super::{Read, Seek, SeekFrom, Write};

use hadris_iso::read::PathSeparator;
use hadris_udf::descriptor::{
    ExtentDescriptor, LongAllocationDescriptor, ShortAllocationDescriptor,
};
use hadris_udf::write::{UdfWriteOptions, UdfWriter};
use hadris_udf::{FileType, SECTOR_SIZE as UDF_SECTOR_SIZE};

use crate::error::CdResult;
use crate::layout::{LayoutInfo, LayoutManager};
use crate::options::CdOptions;
use crate::tree::{Directory, FileData, FileTree};

/// Writer for creating hybrid ISO+UDF CD/DVD images
pub struct CdWriter<W: Read + Write + Seek> {
    writer: W,
    options: CdOptions,
}

io_transform! {

impl<W: Read + Write + Seek> CdWriter<W> {
    /// Create a new CD writer
    pub fn new(writer: W, options: CdOptions) -> Self {
        Self { writer, options }
    }

    /// Create a hybrid image from a file tree
    pub async fn write(mut self, mut tree: FileTree) -> CdResult<()> {
        // Sort the tree for consistent output
        tree.sort();

        // Phase 1: Layout - determine where all files will be placed
        let mut layout_manager = LayoutManager::new(self.options.sector_size);
        let layout_info = layout_manager.layout_files(&mut tree, &self.options)?;

        // Phase 2: Write file data to their assigned sectors
        self.write_file_data(&tree, &layout_info).await?;

        // Phase 3: Write ISO 9660 structures (if enabled)
        if self.options.iso.enabled {
            self.write_iso_structures(&tree, &layout_info).await?;
        }

        // Phase 4: Write UDF structures (if enabled)
        if self.options.udf.enabled {
            self.write_udf_structures(&tree, &layout_info).await?;
        }

        Ok(())
    }

    /// Write all file data to their pre-assigned sectors
    async fn write_file_data(&mut self, tree: &FileTree, _layout_info: &LayoutInfo) -> CdResult<()> {
        self.write_directory_file_data(&tree.root).await?;
        Ok(())
    }

    async fn write_directory_file_data(&mut self, dir: &Directory) -> CdResult<()> {
        for file in &dir.files {
            if file.extent.length == 0 {
                continue; // Skip zero-size files
            }

            // Seek to the file's assigned sector
            let offset = (file.extent.sector as u64) * self.options.sector_size as u64;
            self.writer.seek(SeekFrom::Start(offset)).await?;

            // Write the file data
            match &file.data {
                FileData::Buffer(data) => {
                    self.writer.write_all(data).await?;
                }
                FileData::Path(path) => {
                    let data = std::fs::read(path)?;
                    self.writer.write_all(&data).await?;
                }
            }

            // Pad to sector boundary
            let written = file.extent.length as usize;
            let padded = written.div_ceil(self.options.sector_size)
                * self.options.sector_size;
            if padded > written {
                let padding = vec![0u8; padded - written];
                self.writer.write_all(&padding).await?;
            }
        }

        // Recursively write subdirectory files
        for subdir in &dir.subdirs {
            self.write_directory_file_data(subdir).await?;
        }

        Ok(())
    }

    /// Write ISO 9660 structures
    async fn write_iso_structures(&mut self, tree: &FileTree, _layout_info: &LayoutInfo) -> CdResult<()> {
        use hadris_iso::write::options::{CreationFeatures, FormatOptions};
        use hadris_iso::write::{InputFiles, IsoImageWriter};

        // Convert our tree to ISO's InputFiles format
        let iso_files = self.tree_to_iso_files(&tree.root);

        let input_files = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: iso_files,
        };

        // Build ISO format options from our options
        let features = CreationFeatures {
            filenames: self.options.iso.level,
            long_filenames: self.options.iso.long_filenames,
            joliet: self.options.iso.joliet,
            rock_ridge: self.options.iso.rock_ridge,
            el_torito: self.options.boot.clone(),
            hybrid_boot: self.options.hybrid_boot.clone(),
        };

        let format_options = FormatOptions {
            volume_name: self.options.volume_id.clone(),
            system_id: None,
            volume_set_id: None,
            publisher_id: None,
            preparer_id: None,
            application_id: None,
            sector_size: self.options.sector_size,
            features,
            path_separator: PathSeparator::ForwardSlash,
        };

        // Reset position and write ISO
        self.writer.seek(SeekFrom::Start(0)).await?;
        IsoImageWriter::format_new(&mut self.writer, input_files, format_options)?;

        Ok(())
    }

    /// Convert our tree to ISO's file format
    fn tree_to_iso_files(&self, dir: &Directory) -> Vec<hadris_iso::write::File> {
        let mut files = Vec::new();

        for file in &dir.files {
            let data = match &file.data {
                FileData::Buffer(b) => b.clone(),
                FileData::Path(p) => std::fs::read(p).unwrap_or_default(),
            };
            files.push(hadris_iso::write::File::File {
                name: file.name.clone(),
                contents: data,
            });
        }

        for subdir in &dir.subdirs {
            files.push(hadris_iso::write::File::Directory {
                name: subdir.name.clone(),
                children: self.tree_to_iso_files(subdir),
            });
        }

        files
    }

    /// Write UDF structures
    async fn write_udf_structures(&mut self, tree: &FileTree, layout_info: &LayoutInfo) -> CdResult<()> {
        let udf_options = UdfWriteOptions {
            volume_id: self.options.volume_id.clone(),
            revision: self.options.udf.revision,
            partition_start: layout_info.udf_partition_start,
            partition_length: layout_info.udf_partition_length(),
        };

        let mut udf_writer = UdfWriter::new(&mut self.writer, udf_options);

        // Write Volume Recognition Sequence
        udf_writer.write_vrs()?;

        // VDS at sectors 257-262
        let vds_start = 257u32;
        let vds_length = 6u32; // 6 descriptors

        // Reserve VDS extent
        let reserve_vds_start = 263u32;

        // Write Anchor Volume Descriptor Pointer
        let main_vds = ExtentDescriptor {
            length: vds_length * UDF_SECTOR_SIZE as u32,
            location: vds_start,
        };
        let reserve_vds = ExtentDescriptor {
            length: vds_length * UDF_SECTOR_SIZE as u32,
            location: reserve_vds_start,
        };
        udf_writer.write_avdp(main_vds, reserve_vds)?;

        // File Set Descriptor location (first block in partition)
        let fsd_block = 0u32;
        let fsd_icb = LongAllocationDescriptor {
            extent_length: UDF_SECTOR_SIZE as u32,
            logical_block_num: fsd_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };

        // Root directory ICB location
        let root_icb_block = 1u32;
        let root_icb = LongAllocationDescriptor {
            extent_length: UDF_SECTOR_SIZE as u32,
            logical_block_num: root_icb_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };

        // LVID location
        let lvid_location = reserve_vds_start + vds_length;
        let integrity_extent = ExtentDescriptor {
            length: UDF_SECTOR_SIZE as u32,
            location: lvid_location,
        };

        // Write Volume Descriptor Sequence
        udf_writer.write_pvd(vds_start, 0)?;
        udf_writer.write_iuvd(vds_start + 1, 1)?;
        udf_writer.write_partition_descriptor(vds_start + 2, 2)?;
        udf_writer.write_lvd(vds_start + 3, 3, fsd_icb, integrity_extent)?;
        udf_writer.write_usd(vds_start + 4, 4)?;
        udf_writer.write_terminating_descriptor(vds_start + 5)?;

        // Write reserve VDS (copy of main VDS)
        udf_writer.write_pvd(reserve_vds_start, 0)?;
        udf_writer.write_iuvd(reserve_vds_start + 1, 1)?;
        udf_writer.write_partition_descriptor(reserve_vds_start + 2, 2)?;
        udf_writer.write_lvd(reserve_vds_start + 3, 3, fsd_icb, integrity_extent)?;
        udf_writer.write_usd(reserve_vds_start + 4, 4)?;
        udf_writer.write_terminating_descriptor(reserve_vds_start + 5)?;

        // Write Logical Volume Integrity Descriptor
        udf_writer.write_lvid(lvid_location, true)?;

        // Write File Set Descriptor
        udf_writer.write_fsd(fsd_block, root_icb)?;

        // Write root directory
        Self::write_udf_directory_static(&mut udf_writer, &tree.root, root_icb_block, layout_info)?;

        Ok(())
    }

    /// Write UDF directory structure (File Entry + FIDs) - static method to avoid borrow issues
    fn write_udf_directory_static<WR: Write + Seek>(
        udf_writer: &mut UdfWriter<WR>,
        dir: &Directory,
        icb_block: u32,
        layout_info: &LayoutInfo,
    ) -> CdResult<()> {
        // Collect child entries for FIDs
        let mut entries: Vec<(String, LongAllocationDescriptor, bool)> = Vec::new();
        let mut next_icb = icb_block + 2; // After this dir's File Entry and FIDs

        // Process files
        for file in &dir.files {
            let file_icb_block = next_icb;
            next_icb += 1;

            let file_icb = LongAllocationDescriptor {
                extent_length: UDF_SECTOR_SIZE as u32,
                logical_block_num: file_icb_block,
                partition_ref_num: 0,
                impl_use: [0; 6],
            };

            entries.push((file.name.to_string(), file_icb, false));
        }

        // Process subdirectories (we'll write them recursively)
        for subdir in &dir.subdirs {
            let subdir_icb_block = next_icb;
            // Reserve space for subdir's File Entry and FIDs
            let subdir_entries = subdir.files.len() + subdir.subdirs.len() + 1; // +1 for parent
            let fid_sectors = (subdir_entries * 50).div_ceil(UDF_SECTOR_SIZE);
            next_icb += 1 + fid_sectors as u32;

            let subdir_icb = LongAllocationDescriptor {
                extent_length: UDF_SECTOR_SIZE as u32,
                logical_block_num: subdir_icb_block,
                partition_ref_num: 0,
                impl_use: [0; 6],
            };

            entries.push((subdir.name.to_string(), subdir_icb, true));
        }

        // Calculate directory data size (FIDs)
        let total_entries = entries.len() + 1; // +1 for parent entry
        let estimated_fid_size = total_entries * 50; // Rough estimate
        let dir_data_sectors =
            estimated_fid_size.div_ceil(UDF_SECTOR_SIZE) as u32;
        let dir_data_size = (dir_data_sectors as usize) * UDF_SECTOR_SIZE;

        // Write directory File Entry
        let dir_alloc = vec![ShortAllocationDescriptor {
            extent_length: dir_data_size as u32,
            extent_position: icb_block + 1, // FIDs follow File Entry
        }];
        udf_writer.write_file_entry(
            icb_block,
            FileType::Directory,
            dir_data_size as u64,
            &dir_alloc,
            dir.unique_id,
        )?;

        // Write FIDs (parent + children)
        let parent_icb = LongAllocationDescriptor {
            extent_length: UDF_SECTOR_SIZE as u32,
            logical_block_num: icb_block, // Self for root, or actual parent
            partition_ref_num: 0,
            impl_use: [0; 6],
        };
        udf_writer.write_fids(icb_block + 1, parent_icb, &entries)?;

        // Write file File Entries
        let mut file_icb = icb_block + 2;
        for file in &dir.files {
            let file_alloc = if file.extent.length > 0 {
                // Convert absolute sector to logical block within partition
                let logical_block = file.extent.sector - layout_info.udf_partition_start;
                vec![ShortAllocationDescriptor {
                    extent_length: file.extent.length as u32,
                    extent_position: logical_block,
                }]
            } else {
                vec![] // Empty file
            };

            udf_writer.write_file_entry(
                file_icb,
                FileType::RegularFile,
                file.extent.length,
                &file_alloc,
                file.unique_id,
            )?;
            file_icb += 1;
        }

        // Recursively write subdirectories
        let mut subdir_icb = file_icb;
        for subdir in &dir.subdirs {
            Self::write_udf_directory_static(udf_writer, subdir, subdir_icb, layout_info)?;
            // Calculate next subdir's ICB position
            let subdir_entries = subdir.files.len() + subdir.subdirs.len() + 1;
            let fid_sectors = (subdir_entries * 50).div_ceil(UDF_SECTOR_SIZE);
            subdir_icb += 1 + fid_sectors as u32 + subdir.files.len() as u32;
        }

        Ok(())
    }
}

} // io_transform!

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::FileEntry;
    use std::io::Cursor;

    #[test]
    fn test_basic_writer() {
        let mut tree = FileTree::new();
        tree.add_file(FileEntry::from_buffer(
            "test.txt",
            b"Hello, World!".to_vec(),
        ));

        let buffer = vec![0u8; 1024 * 1024]; // 1MB buffer
        let cursor = Cursor::new(buffer);

        let options = CdOptions::with_volume_id("TEST");
        let writer = CdWriter::new(cursor, options);

        // This will test the basic flow
        // Note: Full verification would require mounting the resulting image
        let result = writer.write(tree);
        assert!(result.is_ok());
    }
}
