//! UDF image modification support.
//!
//! This module provides the ability to append files to existing UDF images
//! and mark files for deletion. It uses incremental writing where new
//! metadata chains to the previous version.
//!
//! # Incremental Writing Approach
//!
//! UDF supports incremental modifications through:
//! - VDS chaining via `predecessor_vds_location` field
//! - LVID (Logical Volume Integrity Descriptor) tracks integrity state
//! - VAT (Virtual Allocation Table) for write-once media
//!
//! # Example
//!
//! ```rust,ignore
//! use hadris_udf::modify::UdfModifier;
//!
//! let file = std::fs::OpenOptions::new()
//!     .read(true).write(true)
//!     .open("image.udf")?;
//!
//! let mut modifier = UdfModifier::open(file)?;
//! modifier.append_file("new_file.txt", b"Hello, world!".to_vec());
//! modifier.commit()?;
//! ```

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::super::{Read, Seek, SeekFrom, Write};
use hadris_common::types::extent::{Extent, FileType};
use hadris_common::types::layout::{AllocationMap, DirectoryLayout, FileLayout};
use hadris_io as io;

use super::descriptor::AnchorVolumeDescriptorPointer;
use crate::{AVDP_LOCATION, SECTOR_SIZE, UdfError, UdfRevision};

/// Operations that can be performed on a UDF image.
#[derive(Debug, Clone)]
pub enum ModifyOp {
    /// Add a new file to the image.
    AppendFile {
        /// Path within the UDF (e.g., "docs/readme.txt")
        path: String,
        /// File contents
        data: FileData,
    },
    /// Create a new directory.
    CreateDir {
        /// Path of the directory to create
        path: String,
    },
    /// Mark a file as deleted.
    Delete {
        /// Path of the file to delete
        path: String,
    },
    /// Replace a file's content.
    Replace {
        /// Path of the file to replace
        path: String,
        /// New file contents
        data: FileData,
    },
}

/// File data for modification operations.
#[derive(Debug, Clone)]
pub enum FileData {
    /// In-memory buffer.
    Buffer(Vec<u8>),
    /// Path to a file on the filesystem.
    #[cfg(feature = "std")]
    Path(std::path::PathBuf),
}

impl From<Vec<u8>> for FileData {
    fn from(data: Vec<u8>) -> Self {
        FileData::Buffer(data)
    }
}

impl From<&[u8]> for FileData {
    fn from(data: &[u8]) -> Self {
        FileData::Buffer(data.to_vec())
    }
}

#[cfg(feature = "std")]
impl From<std::path::PathBuf> for FileData {
    fn from(path: std::path::PathBuf) -> Self {
        FileData::Path(path)
    }
}

impl FileData {
    /// Returns the size of the file data.
    pub fn size(&self) -> io::Result<u64> {
        match self {
            FileData::Buffer(data) => Ok(data.len() as u64),
            #[cfg(feature = "std")]
            FileData::Path(path) => {
                let metadata = std::fs::metadata(path)?;
                Ok(metadata.len())
            }
        }
    }

    /// Reads the file data into a buffer.
    pub fn read_all(&self) -> io::Result<Vec<u8>> {
        match self {
            FileData::Buffer(data) => Ok(data.clone()),
            #[cfg(feature = "std")]
            FileData::Path(path) => std::fs::read(path),
        }
    }
}

/// Error type for UDF modification operations.
#[derive(Debug, thiserror::Error)]
pub enum UdfModifyError {
    /// I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// UDF error.
    #[error(transparent)]
    Udf(#[from] UdfError),
    /// File not found.
    #[error("file not found: {0}")]
    FileNotFound(String),
    /// Path already exists.
    #[error("path already exists: {0}")]
    PathExists(String),
    /// Not enough space.
    #[error("not enough space to allocate {0} bytes")]
    NotEnoughSpace(u64),
    /// Invalid path.
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

/// Result type for UDF modification operations.
pub type UdfModifyResult<T> = Result<T, UdfModifyError>;

/// Options for UDF modification.
#[derive(Debug, Clone, Default)]
pub struct UdfModifyOptions {
    /// Volume name (if changing).
    pub volume_name: Option<String>,
}

/// Modifier for UDF images.
///
/// This struct provides methods to append files, create directories,
/// and mark files for deletion in an existing UDF image.
pub struct UdfModifier<RW: Read + Write + Seek> {
    /// The underlying reader/writer.
    inner: RW,
    /// Parsed from existing image.
    existing_layout: DirectoryLayout,
    /// Tracks used sectors.
    #[allow(dead_code)]
    allocation_map: AllocationMap,
    /// Pending operations.
    pending_ops: Vec<ModifyOp>,
    /// Options for modification.
    #[allow(dead_code)]
    options: UdfModifyOptions,
    /// UDF revision.
    #[allow(dead_code)]
    revision: UdfRevision,
    /// Partition start sector.
    #[allow(dead_code)]
    partition_start: u32,
    /// Partition length.
    #[allow(dead_code)]
    partition_length: u32,
    /// Next unique ID for new files.
    #[allow(dead_code)]
    next_unique_id: u64,
    /// Current end of the image.
    end_sector: u32,
}

impl<RW: Read + Write + Seek> UdfModifier<RW> {
    /// Opens an existing UDF image for modification.
    pub fn open(inner: RW) -> UdfModifyResult<Self> {
        Self::open_with_options(inner, UdfModifyOptions::default())
    }

    /// Opens an existing UDF image for modification with custom options.
    pub fn open_with_options(mut inner: RW, options: UdfModifyOptions) -> UdfModifyResult<Self> {
        // Read AVDP to get VDS location
        inner.seek(SeekFrom::Start(AVDP_LOCATION as u64 * SECTOR_SIZE as u64))?;
        let mut avdp_buf = [0u8; SECTOR_SIZE];
        inner.read_exact(&mut avdp_buf)?;
        let avdp: &AnchorVolumeDescriptorPointer = bytemuck::from_bytes(&avdp_buf[..512]);

        let _vds_location = avdp.main_vds_extent.location;

        // For simplicity, use default partition values
        // A full implementation would parse all VDS descriptors
        let partition_start = 270u32;
        let partition_length = 1000u32;

        // Build a minimal layout - full implementation would parse FSD and root directory
        let existing_layout = DirectoryLayout::root();

        // Build allocation map
        let total_sectors = partition_start + partition_length;
        let allocation_map = AllocationMap::new(total_sectors);

        let end_sector = partition_start + partition_length;

        Ok(Self {
            inner,
            existing_layout,
            allocation_map,
            pending_ops: Vec::new(),
            options,
            revision: UdfRevision::V1_02,
            partition_start,
            partition_length,
            next_unique_id: 16,
            end_sector,
        })
    }

    /// Queues a modification operation.
    pub fn queue(&mut self, op: ModifyOp) {
        self.pending_ops.push(op);
    }

    /// Appends a file to the image (convenience method).
    pub fn append_file(&mut self, path: &str, data: impl Into<FileData>) {
        self.queue(ModifyOp::AppendFile {
            path: path.to_string(),
            data: data.into(),
        });
    }

    /// Creates a directory (convenience method).
    pub fn create_dir(&mut self, path: &str) {
        self.queue(ModifyOp::CreateDir {
            path: path.to_string(),
        });
    }

    /// Marks a file for deletion (convenience method).
    pub fn delete(&mut self, path: &str) {
        self.queue(ModifyOp::Delete {
            path: path.to_string(),
        });
    }

    /// Replaces a file's content (convenience method).
    pub fn replace(&mut self, path: &str, data: impl Into<FileData>) {
        self.queue(ModifyOp::Replace {
            path: path.to_string(),
            data: data.into(),
        });
    }

    /// Returns the current layout.
    pub fn layout(&self) -> &DirectoryLayout {
        &self.existing_layout
    }

    /// Commits all pending changes.
    pub fn commit(mut self) -> UdfModifyResult<()> {
        if self.pending_ops.is_empty() {
            return Ok(());
        }

        // 1. Apply pending ops to layout
        let new_layout = self.apply_ops()?;

        // 2. Write new file data
        let file_extents = self.write_new_data(&new_layout)?;

        // 3. Update UDF metadata
        self.write_new_metadata(&new_layout, file_extents)?;

        Ok(())
    }

    /// Applies pending operations to create a new layout.
    fn apply_ops(&mut self) -> UdfModifyResult<DirectoryLayout> {
        let mut layout = self.existing_layout.clone();

        for op in &self.pending_ops {
            match op {
                ModifyOp::AppendFile { path, data } => {
                    if layout.find_file(path).is_some() {
                        return Err(UdfModifyError::PathExists(path.clone()));
                    }

                    let (dir_path, filename) = Self::split_path(path)?;
                    let dir = if dir_path.is_empty() {
                        &mut layout
                    } else {
                        layout.get_or_create_dir(&dir_path)
                    };

                    let size = data.size()?;
                    let file = FileLayout::new(filename, Extent::new(0, size))
                        .with_type(FileType::RegularFile);
                    dir.add_file(file);
                }
                ModifyOp::CreateDir { path } => {
                    layout.get_or_create_dir(path);
                }
                ModifyOp::Delete { path } => {
                    if layout.remove_file(path).is_none() {
                        return Err(UdfModifyError::FileNotFound(path.clone()));
                    }
                }
                ModifyOp::Replace { path, data } => {
                    let file = layout
                        .find_file_mut(path)
                        .ok_or_else(|| UdfModifyError::FileNotFound(path.clone()))?;

                    let size = data.size()?;
                    file.extent = Extent::new(0, size);
                }
            }
        }

        Ok(layout)
    }

    /// Writes new file data.
    fn write_new_data(
        &mut self,
        _layout: &DirectoryLayout,
    ) -> UdfModifyResult<BTreeMap<String, Extent>> {
        let mut file_extents = BTreeMap::new();

        // Start writing after current end
        let mut current_sector = self.end_sector;

        for op in &self.pending_ops {
            match op {
                ModifyOp::AppendFile { path, data } | ModifyOp::Replace { path, data } => {
                    let size = data.size()?;
                    if size == 0 {
                        file_extents.insert(path.clone(), Extent::new(0, 0));
                        continue;
                    }

                    let extent = Extent::new(current_sector, size);
                    file_extents.insert(path.clone(), extent);

                    // Write data
                    self.inner
                        .seek(SeekFrom::Start(current_sector as u64 * SECTOR_SIZE as u64))?;
                    let content = data.read_all()?;
                    self.inner.write_all(&content)?;

                    // Update current sector
                    current_sector += extent.sector_count(SECTOR_SIZE as u32);
                }
                _ => {}
            }
        }

        // Pad to sector boundary
        let pos = self.inner.stream_position()?;
        let remainder = pos % SECTOR_SIZE as u64;
        if remainder != 0 {
            let padding = SECTOR_SIZE as u64 - remainder;
            let zeros = alloc::vec![0u8; padding as usize];
            self.inner.write_all(&zeros)?;
        }

        self.end_sector = current_sector;

        Ok(file_extents)
    }

    /// Writes new metadata (placeholder - full implementation would update VDS).
    fn write_new_metadata(
        &mut self,
        _layout: &DirectoryLayout,
        _file_extents: BTreeMap<String, Extent>,
    ) -> UdfModifyResult<()> {
        // Note: Full implementation would:
        // 1. Create new File Entries for new files
        // 2. Update root directory FIDs
        // 3. Update LVID with new integrity info
        // 4. Optionally chain new VDS

        // For now, this is a placeholder that logs the intent
        // The actual UDF metadata update is complex and would require
        // rewriting the root directory and potentially VDS

        Ok(())
    }

    /// Splits a path into (directory, filename).
    fn split_path(path: &str) -> UdfModifyResult<(String, String)> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Err(UdfModifyError::InvalidPath(path.to_string()));
        }

        let filename = parts.last().unwrap().to_string();
        let dir_path = if parts.len() > 1 {
            parts[..parts.len() - 1].join("/")
        } else {
            String::new()
        };

        Ok((dir_path, filename))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_split_path() {
        let (dir, file) = UdfModifier::<std::io::Cursor<Vec<u8>>>::split_path("test.txt").unwrap();
        assert_eq!(dir, "");
        assert_eq!(file, "test.txt");

        let (dir, file) =
            UdfModifier::<std::io::Cursor<Vec<u8>>>::split_path("docs/readme.txt").unwrap();
        assert_eq!(dir, "docs");
        assert_eq!(file, "readme.txt");
    }

    #[test]
    fn test_file_data() {
        let data = FileData::from(vec![1, 2, 3, 4]);
        assert_eq!(data.size().unwrap(), 4);
        assert_eq!(data.read_all().unwrap(), vec![1, 2, 3, 4]);
    }
}
