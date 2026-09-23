//! Main writer for creating hybrid ISO+UDF images
//!
//! The `OpticalImageWriter` orchestrates the creation of a hybrid image by:
//! 1. Building a shared file tree
//! 2. Laying out file data (shared between both filesystems)
//! 3. Writing ISO 9660 metadata
//! 4. Writing UDF metadata
//! 5. Finalizing the image

use super::super::{Read, Seek, SeekFrom, Write};

use hadris_udf::descriptor::{
    ExtentDescriptor, LongAllocationDescriptor, ShortAllocationDescriptor,
};
use hadris_udf::write::{UdfWriteOptions, UdfWriter};
use hadris_udf::{FileType, SECTOR_SIZE as UDF_SECTOR_SIZE};

use crate::error::Result;
use crate::layout::{LayoutInfo, LayoutManager, UdfDirectoryLayout};
use crate::options::OpticalImageOptions;
use crate::tree::{Directory, FileData, FileTree};

/// Writer for creating hybrid ISO+UDF CD/DVD images
pub struct OpticalImageWriter<W: Read + Write + Seek> {
    writer: W,
    options: OpticalImageOptions,
}

io_transform! {

impl<W: Read + Write + Seek> OpticalImageWriter<W> {
    /// Create a new CD writer
    pub fn new(writer: W, options: OpticalImageOptions) -> Self {
        Self { writer, options }
    }

    /// Creates an optical image and returns its output target.
    pub async fn create(writer: W, tree: FileTree, options: OpticalImageOptions) -> Result<W> {
        Self::new(writer, options).finish(tree).await
    }

    /// Returns the output target without writing an image.
    pub fn into_inner(self) -> W {
        self.writer
    }

    /// Finishes the image and returns its output target.
    pub async fn finish(mut self, mut tree: FileTree) -> Result<W> {
        if self.options.udf.enabled && self.options.sector_size != UDF_SECTOR_SIZE {
            return Err(crate::error::Error::InvalidConfig(format!(
                "UDF bridge images require {UDF_SECTOR_SIZE}-byte logical sectors"
            )));
        }

        // Sort the tree for consistent output
        tree.sort();

        // Phase 1: Layout - determine where all files will be placed
        let mut layout_manager = LayoutManager::new(self.options.sector_size);
        let mut layout_info = layout_manager.layout_files(&mut tree, &self.options)?;

        // ISO creation writes payloads as well as directory structures. Use the
        // payload extents its report gives for the UDF allocation descriptors:
        // the provisional layout does not account for ISO directory data placed
        // at the allocation floor before those payloads.
        if self.options.iso.enabled {
            let report = self.write_iso_structures(&tree, &layout_info).await?;
            self.apply_iso_report(&mut tree, &mut layout_info, &report)?;
        } else {
            self.write_file_data(&tree, &layout_info).await?;
        }

        // UDF metadata points at the already-written ISO payloads.
        if self.options.udf.enabled {
            self.write_udf_structures(&tree, &layout_info).await?;
        }

        Ok(self.writer)
    }

    fn apply_iso_report(
        &mut self,
        tree: &mut FileTree,
        layout: &mut LayoutInfo,
        report: &hadris_iso::Report,
    ) -> Result<()> {
        let mut paths = Vec::new();
        Self::collect_file_paths(&tree.root, "", &mut paths);
        let mut extents = std::collections::BTreeMap::new();
        for path in paths {
            let (sector, length) = match report.extent_of(&path) {
                Some(extent) => (
                    u32::try_from(extent.offset() / self.options.sector_size as u64).map_err(
                        |_| crate::error::Error::InvalidConfig("image is too large".into()),
                    )?,
                    extent.len(),
                ),
                None => (0, 0),
            };
            extents.insert(path, (sector, length));
        }
        Self::apply_iso_file_extents(&mut tree.root, "", &extents)?;

        let end = extents
            .values()
            .filter(|(_, len)| *len != 0)
            .map(|(sector, len)| *sector + len.div_ceil(self.options.sector_size as u64) as u32)
            .max()
            .unwrap_or(layout.file_data_start);
        layout.file_data_end = end;
        layout.total_sectors = end.saturating_add(100);
        Ok(())
    }

    fn collect_file_paths(dir: &Directory, prefix: &str, output: &mut Vec<String>) {
        for file in &dir.files {
            output.push(if prefix.is_empty() {
                file.name.to_string()
            } else {
                format!("{prefix}/{}", file.name)
            });
        }
        for child in &dir.subdirs {
            let child_prefix = if prefix.is_empty() {
                child.name.to_string()
            } else {
                format!("{prefix}/{}", child.name)
            };
            Self::collect_file_paths(child, &child_prefix, output);
        }
    }

    fn apply_iso_file_extents(
        dir: &mut Directory,
        prefix: &str,
        extents: &std::collections::BTreeMap<String, (u32, u64)>,
    ) -> Result<()> {
        for file in &mut dir.files {
            let path = if prefix.is_empty() {
                file.name.to_string()
            } else {
                format!("{prefix}/{}", file.name)
            };
            let &(sector, length) = extents.get(&path).ok_or_else(|| {
                crate::error::Error::InvalidPath(format!("missing ISO extent for {path}"))
            })?;
            file.extent.sector = sector;
            file.extent.length = length;
        }
        for child in &mut dir.subdirs {
            let child_prefix = if prefix.is_empty() {
                child.name.to_string()
            } else {
                format!("{prefix}/{}", child.name)
            };
            Self::apply_iso_file_extents(child, &child_prefix, extents)?;
        }
        Ok(())
    }

    /// Write all file data to their pre-assigned sectors
    async fn write_file_data(&mut self, tree: &FileTree, _layout_info: &LayoutInfo) -> Result<()> {
        self.write_directory_file_data(&tree.root).await?;
        Ok(())
    }

    async fn write_directory_file_data(&mut self, dir: &Directory) -> Result<()> {
        for file in &dir.files {
            if file.extent.length == 0 {
                continue; // Skip zero-size files
            }

            // Seek to the file's assigned sector
            let offset = (file.extent.sector as u64) * self.options.sector_size as u64;
            self.writer
                .seek(SeekFrom::Start(offset))
                .await
                ?;

            // Write the file data
            match &file.data {
                FileData::Buffer(data) => {
                    self.writer.write_all(data).await?;
                }
                FileData::Path(path) => {
                    let data = std::fs::read(path)
                        .map_err(hadris_io::legacy::Error::from)?;
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

    /// Write ISO 9660 structures and file data, returning where the ISO
    /// writer put each file.
    async fn write_iso_structures(&mut self, tree: &FileTree, layout_info: &LayoutInfo) -> Result<hadris_iso::Report> {
        let input = Self::iso_tree(&tree.root)?;
        let iso = &self.options.iso;
        let mut options = hadris_iso::IsoOptions::default()
            .with_volume(hadris_iso::VolumeIdentifiers::new(self.options.volume_id.clone()))
            .with_level(iso.level)
            .with_name_case(iso.name_case)
            .with_clock(hadris_fs::SystemClock);
        if iso.long_filenames {
            options = options.with_enhanced_tree();
        }
        if let Some(level) = iso.joliet {
            options = options.with_joliet(level);
        }
        if let Some(rock_ridge) = &iso.rock_ridge {
            options = options.with_rock_ridge(rock_ridge.clone());
        }
        if let Some(boot) = &self.options.boot {
            options = options.with_el_torito(boot.clone());
        }
        if let Some(hybrid) = &self.options.hybrid_boot {
            options = options.with_hybrid(hybrid.clone());
        }
        if self.options.udf.enabled {
            options = options.with_min_blocks(u64::from(layout_info.file_data_start));
        }
        let report = hadris_iso::sync::write(StreamBlocks(&mut self.writer), &input, &options)?;
        Ok(report)
    }

    /// Converts our tree to the shared writer input.
    fn iso_tree(root: &Directory) -> Result<hadris_fs::tree::Tree> {
        fn add(tree: &mut hadris_fs::tree::Tree, dir: &Directory, prefix: &str) -> Result<()> {
            for file in &dir.files {
                let path = format!("{prefix}/{}", file.name);
                let content = match &file.data {
                    FileData::Buffer(data) => hadris_fs::tree::Content::bytes(data.clone()),
                    FileData::Path(path) => hadris_fs::tree::Content::path(path),
                };
                tree.add_file(&path, content)
                    .map_err(|_| crate::error::Error::InvalidPath(path.clone()))?;
            }
            for subdir in &dir.subdirs {
                let path = format!("{prefix}/{}", subdir.name);
                tree.add_dir(&path)
                    .map_err(|_| crate::error::Error::InvalidPath(path.clone()))?;
                add(tree, subdir, &path)?;
            }
            Ok(())
        }
        let mut tree = hadris_fs::tree::Tree::new();
        add(&mut tree, root, "")?;
        Ok(tree)
    }

    /// Write UDF structures
    async fn write_udf_structures(&mut self, tree: &FileTree, layout_info: &LayoutInfo) -> Result<()> {
        let image_bytes = self
            .writer
            .seek(SeekFrom::End(0))
            .await
            ?;
        let image_sectors = image_bytes.div_ceil(self.options.sector_size as u64);
        let required_sectors = u64::from(layout_info.total_sectors)
            .checked_add(257)
            .ok_or_else(|| crate::error::Error::InvalidConfig("image is too large".into()))?;
        let final_sector_count = image_sectors.max(required_sectors);
        let last_sector = u32::try_from(final_sector_count - 1)
            .map_err(|_| crate::error::Error::InvalidConfig("image has too many sectors".into()))?;
        let trailing_anchor = last_sector - 256;
        let partition_length = trailing_anchor
            .checked_sub(layout_info.udf_partition_start)
            .ok_or_else(|| {
                crate::error::Error::InvalidConfig(
                    "UDF partition overlaps the trailing anchor".into(),
                )
            })?;

        let udf_options = UdfWriteOptions {
            volume_id: self.options.volume_id.clone(),
            revision: self.options.udf.revision,
            partition_start: layout_info.udf_partition_start,
            partition_length,
        };

        let mut udf_writer = UdfWriter::new(&mut self.writer, udf_options);

        // Keep the UDF VRS after ISO's descriptor terminator so both descriptor
        // streams remain independently parseable.
        udf_writer.write_vrs_at(layout_info.vds_end)?;

        // Each VDS extent occupies sixteen sectors. The six descriptors are
        // followed by reserved sectors within the declared extent.
        let vds_start = 257u32;
        let vds_length = 16u32;

        // Reserve VDS extent
        let reserve_vds_start = vds_start + vds_length;

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
        // The partition ends before this anchor, and the final 256 sectors are
        // reserved so its N-256 position cannot overlap ISO or UDF content.
        udf_writer.write_avdp_at(trailing_anchor, main_vds, reserve_vds)?;

        // File Set Descriptor location (first block in partition)
        let fsd_block = 0u32;
        let fsd_icb = LongAllocationDescriptor {
            extent_length: UDF_SECTOR_SIZE as u32,
            logical_block_num: fsd_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };

        // Root directory ICB location
        let root_icb_block = layout_info.udf_root.icb_block;
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
        Self::write_udf_directory_static(
            &mut udf_writer,
            &tree.root,
            &layout_info.udf_root,
            layout_info,
        )?;
        drop(udf_writer);

        if final_sector_count > image_sectors {
            self.writer
                .seek(SeekFrom::Start(
                    u64::from(last_sector) * self.options.sector_size as u64,
                ))
                .await
                ?;
            self.writer
                .write_all(&vec![0_u8; self.options.sector_size])
                .await?;
        }

        Ok(())
    }

    /// Write UDF directory structure (File Entry + FIDs) - static method to avoid borrow issues
    fn write_udf_directory_static<WR: Write + Seek>(
        udf_writer: &mut UdfWriter<WR>,
        dir: &Directory,
        plan: &UdfDirectoryLayout,
        layout_info: &LayoutInfo,
    ) -> Result<()> {
        let mut entries: Vec<(String, LongAllocationDescriptor, bool)> = Vec::new();
        for (file, &file_icb_block) in dir.files.iter().zip(&plan.file_icb_blocks) {
            let file_icb = LongAllocationDescriptor {
                extent_length: UDF_SECTOR_SIZE as u32,
                logical_block_num: file_icb_block,
                partition_ref_num: 0,
                impl_use: [0; 6],
            };

            entries.push((file.name.to_string(), file_icb, false));
        }
        for (subdir, subdir_plan) in dir.subdirs.iter().zip(&plan.subdirs) {
            let subdir_icb = LongAllocationDescriptor {
                extent_length: UDF_SECTOR_SIZE as u32,
                logical_block_num: subdir_plan.icb_block,
                partition_ref_num: 0,
                impl_use: [0; 6],
            };
            entries.push((subdir.name.to_string(), subdir_icb, true));
        }

        // Write directory File Entry
        let dir_alloc = vec![ShortAllocationDescriptor {
            extent_length: plan.fid_bytes as u32,
            extent_position: plan.fid_block,
        }];
        udf_writer.write_file_entry(
            plan.icb_block,
            FileType::Directory,
            plan.fid_bytes as u64,
            &dir_alloc,
            dir.unique_id,
        )?;

        // Write FIDs (parent + children)
        let parent_icb = LongAllocationDescriptor {
            extent_length: UDF_SECTOR_SIZE as u32,
            logical_block_num: plan.parent_icb_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };
        udf_writer.write_fids(plan.fid_block, parent_icb, &entries)?;

        for (file, &file_icb) in dir.files.iter().zip(&plan.file_icb_blocks) {
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
        }

        for (subdir, subdir_plan) in dir.subdirs.iter().zip(&plan.subdirs) {
            Self::write_udf_directory_static(udf_writer, subdir, subdir_plan, layout_info)?;
        }

        Ok(())
    }
}

} // io_transform!

/// A block device over the output stream, for the ISO writer.
struct StreamBlocks<'a, W>(&'a mut W);

impl<W> hadris_io::ErrorType for StreamBlocks<'_, W> {
    type Error = hadris_io::legacy::Error;
}

impl<W: Read + Write + Seek> hadris_storage::sync::BlockDevice for StreamBlocks<'_, W> {
    fn block_size(&self) -> hadris_storage::BlockSize {
        const { hadris_storage::BlockSize::new(2048).unwrap() }
    }

    fn block_count(&self) -> u64 {
        u64::MAX / 2048
    }

    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> core::result::Result<(), Self::Error> {
        self.0.seek(SeekFrom::Start(first.get() * 2048))?;
        self.0.read_exact(buf)
    }

    fn write_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> core::result::Result<(), hadris_storage::WriteError<Self::Error>> {
        self.0
            .seek(SeekFrom::Start(first.get() * 2048))
            .map_err(hadris_storage::WriteError::Device)?;
        self.0
            .write_all(buf)
            .map_err(hadris_storage::WriteError::Device)
    }

    fn flush(&mut self) -> core::result::Result<(), hadris_storage::WriteError<Self::Error>> {
        self.0.flush().map_err(hadris_storage::WriteError::Device)
    }
}

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
        let cursor = hadris_io::StdIo::new(Cursor::new(buffer));

        let options = OpticalImageOptions::default().volume_id("TEST");
        let writer = OpticalImageWriter::new(cursor, options);

        // This will test the basic flow
        // Note: Full verification would require mounting the resulting image
        let output = writer.finish(tree).unwrap();
        assert!(!output.get_ref().get_ref().is_empty());
    }
}
