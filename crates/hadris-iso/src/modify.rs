//! ISO 9660 image modification support.
//!
//! This module provides the ability to append files to existing ISO images
//! and mark files for deletion. It uses a multi-session approach where
//! each modification creates a new session with updated metadata.
//!
//! # Multi-Session Approach
//!
//! ISO 9660 is fundamentally an immutable format. Modifications are achieved
//! through multi-session writing:
//!
//! - Each "session" has its own complete Volume Descriptor Set
//! - New sessions are appended to the end of the image
//! - The latest session's directory records reference all visible files
//! - "Deletion" means hiding files from the new session's directory listing
//!
//! # Example
//!
//! ```rust,ignore
//! use hadris_iso::modify::IsoModifier;
//!
//! let file = std::fs::OpenOptions::new()
//!     .read(true).write(true)
//!     .open("image.iso")?;
//!
//! let mut modifier = IsoModifier::open(file)?;
//! modifier.append_file("new_file.txt", b"Hello, world!".to_vec());
//! modifier.delete("old_file.txt");
//! modifier.commit()?;
//! ```

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::io::{self, Read, Seek, SeekFrom, Write};
use hadris_common::types::endian::Endian;
use hadris_common::types::extent::{Extent, FileType};
use hadris_common::types::layout::{AllocationMap, DirectoryLayout, FileLayout};

use super::directory::{DirectoryRecord, DirectoryRef, FileFlags};
use super::io::{IsoCursor, LogicalSector};
use super::path::PathTableRef;
use super::read::PathSeparator;
use super::volume::{
    PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor, VolumeDescriptorHeader,
    VolumeDescriptorList, VolumeDescriptorType,
};
use super::write::writer::{PathTableWriter, WrittenDirectory, WrittenFile, WrittenFiles};
use crate::file::EntryType;
use crate::joliet::JolietLevel;

/// Operations that can be performed on an ISO image.
#[derive(Debug, Clone)]
pub enum ModifyOp {
    /// Add a new file to the image.
    AppendFile {
        /// Path within the ISO (e.g., "docs/readme.txt")
        path: String,
        /// File contents
        data: FileData,
    },
    /// Create a new directory.
    CreateDir {
        /// Path of the directory to create
        path: String,
    },
    /// Mark a file as deleted (hidden from new session).
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

/// Error type for ISO modification operations.
#[derive(Debug, thiserror::Error)]
pub enum IsoModifyError {
    /// I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
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

/// Result type for ISO modification operations.
pub type IsoModifyResult<T> = Result<T, IsoModifyError>;

/// Modifier for ISO 9660 images.
///
/// This struct provides methods to append files, create directories,
/// and mark files for deletion in an existing ISO image. Changes are
/// committed as a new session at the end of the image.
pub struct IsoModifier<RW: Read + Write + Seek> {
    /// The underlying reader/writer.
    inner: IsoCursor<RW>,
    /// Parsed from existing image.
    existing_layout: DirectoryLayout,
    /// Tracks used sectors.
    #[allow(dead_code)]
    allocation_map: AllocationMap,
    /// Pending operations.
    pending_ops: Vec<ModifyOp>,
    /// Options for the new session.
    #[allow(dead_code)]
    options: IsoModifyOptions,
    /// Entry types from the existing image.
    entry_types: Vec<EntryType>,
    /// Sector size.
    sector_size: usize,
    /// Current end of the image.
    end_sector: LogicalSector,
}

/// Options for ISO modification.
#[derive(Debug, Clone)]
pub struct IsoModifyOptions {
    /// Path separator to use.
    pub path_separator: PathSeparator,
    /// Volume name (if changing).
    pub volume_name: Option<String>,
}

impl Default for IsoModifyOptions {
    fn default() -> Self {
        Self {
            path_separator: PathSeparator::ForwardSlash,
            volume_name: None,
        }
    }
}

io_transform! {
impl<RW: Read + Write + Seek> IsoModifier<RW> {
    /// Opens an existing ISO image for modification.
    pub async fn open(inner: RW) -> IsoModifyResult<Self> {
        Self::open_with_options(inner, IsoModifyOptions::default()).await
    }

    /// Opens an existing ISO image for modification with custom options.
    pub async fn open_with_options(mut inner: RW, options: IsoModifyOptions) -> IsoModifyResult<Self> {
        // Parse existing image
        let sector_size = 2048;
        let mut cursor = IsoCursor::new(&mut inner, sector_size);

        // Read volume descriptors to get image info
        cursor.seek_sector(LogicalSector(16)).await?;
        let volume_descriptors = VolumeDescriptorList::parse(&mut cursor).await?;

        // Get primary volume descriptor info
        let pvd = volume_descriptors.primary();
        let end_sector = pvd.volume_space_size.read() as usize;

        // Build entry types from volume descriptors
        let mut entry_types = Vec::new();
        entry_types.push(EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        });

        // Check for Joliet
        for svd in volume_descriptors.supplementary() {
            if svd.header.version == 1 {
                for &level in JolietLevel::all() {
                    if svd.escape_sequences == level.escape_sequence() {
                        entry_types.push(EntryType::Joliet {
                            level,
                            supports_rrip: false,
                        });
                    }
                }
            }
        }

        // Build directory layout from existing image
        let root_ref = DirectoryRef {
            extent: LogicalSector(pvd.dir_record.header.extent.read() as usize),
            size: pvd.dir_record.header.data_len.read() as usize,
        };

        let (existing_layout, used_extents) =
            Self::build_layout_from_directory(&mut cursor, root_ref, sector_size).await?;

        // Build allocation map
        let total_sectors = end_sector as u32;
        let allocation_map =
            AllocationMap::from_existing(&used_extents, total_sectors, sector_size as u32);

        // Create the cursor from the original inner
        let cursor = IsoCursor::new(inner, sector_size);

        Ok(Self {
            inner: cursor,
            existing_layout,
            allocation_map,
            pending_ops: Vec::new(),
            options,
            entry_types,
            sector_size,
            end_sector: LogicalSector(end_sector),
        })
    }

    /// Builds a DirectoryLayout from an existing ISO directory structure.
    async fn build_layout_from_directory(
        cursor: &mut IsoCursor<&mut RW>,
        root_ref: DirectoryRef,
        sector_size: usize,
    ) -> IsoModifyResult<(DirectoryLayout, Vec<Extent>)> {
        let mut layout = DirectoryLayout::root();
        let mut used_extents = Vec::new();

        // Reserve system area and volume descriptors (sectors 0-16)
        used_extents.push(Extent::new(0, 16 * sector_size as u64));

        Self::read_directory_recursive(
            cursor,
            root_ref,
            &mut layout,
            &mut used_extents,
            sector_size,
        ).await?;

        Ok((layout, used_extents))
    }

    /// Recursively reads a directory and its contents.
    #[allow(clippy::only_used_in_recursion)]
    async fn read_directory_recursive(
        cursor: &mut IsoCursor<&mut RW>,
        dir_ref: DirectoryRef,
        layout: &mut DirectoryLayout,
        used_extents: &mut Vec<Extent>,
        sector_size: usize,
    ) -> IsoModifyResult<()> {
        // Mark directory extent as used
        used_extents.push(Extent::new(dir_ref.extent.0 as u32, dir_ref.size as u64));

        cursor.seek_sector(dir_ref.extent).await?;
        let mut offset = 0;

        while offset < dir_ref.size {
            let record = DirectoryRecord::parse(&mut *cursor).await?;
            if record.header().len == 0 {
                break;
            }

            let name = record.name();

            // Skip . and .. entries
            if name == b"\x00" || name == b"\x01" {
                offset += record.header().len as usize;
                continue;
            }

            let header = record.header();
            let extent = Extent::new(header.extent.read(), header.data_len.read() as u64);

            // Decode name
            let name_str = String::from_utf8_lossy(name).to_string();
            // Remove version suffix (;1)
            let clean_name = if let Some(idx) = name_str.rfind(';') {
                name_str[..idx].to_string()
            } else {
                name_str
            };

            if record.is_directory() {
                // Recurse into subdirectory
                let sub_ref = DirectoryRef {
                    extent: LogicalSector(header.extent.read() as usize),
                    size: header.data_len.read() as usize,
                };

                let mut subdir = DirectoryLayout::new(&clean_name);
                subdir.extent = Some(extent);

                // Save current position
                let current_pos = cursor.stream_position().await?;

                Self::read_directory_recursive(
                    cursor,
                    sub_ref,
                    &mut subdir,
                    used_extents,
                    sector_size,
                ).await?;

                // Restore position
                cursor.seek(SeekFrom::Start(current_pos)).await?;

                layout.add_subdir(subdir);
            } else {
                // Mark file extent as used (if non-empty)
                if extent.length > 0 {
                    used_extents.push(extent);
                }

                let file = FileLayout::new(&clean_name, extent).with_type(FileType::RegularFile);
                layout.add_file(file);
            }

            offset += record.header().len as usize;
        }

        Ok(())
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

    /// Commits all pending changes by writing a new session.
    pub async fn commit(mut self) -> IsoModifyResult<()> {
        if self.pending_ops.is_empty() {
            return Ok(());
        }

        // 1. Apply pending ops to layout
        let new_layout = self.apply_ops()?;

        // 2. Write new file data and allocate sectors
        let written_files = self.write_new_data(&new_layout).await?;

        // 3. Write new session metadata
        self.write_new_session(&new_layout, written_files).await?;

        Ok(())
    }

    /// Applies pending operations to create a new layout.
    fn apply_ops(&mut self) -> IsoModifyResult<DirectoryLayout> {
        let mut layout = self.existing_layout.clone();

        for op in &self.pending_ops {
            match op {
                ModifyOp::AppendFile { path, data } => {
                    // Check if file already exists
                    if layout.find_file(path).is_some() {
                        return Err(IsoModifyError::PathExists(path.clone()));
                    }

                    // Split path into directory and filename
                    let (dir_path, filename) = Self::split_path(path)?;

                    // Get or create parent directory
                    let dir = if dir_path.is_empty() {
                        &mut layout
                    } else {
                        layout.get_or_create_dir(&dir_path)
                    };

                    // Create file layout with temporary extent (will be set during write)
                    let size = data.size()?;
                    let file = FileLayout::new(filename, Extent::new(0, size))
                        .with_type(FileType::RegularFile);
                    dir.add_file(file);
                }
                ModifyOp::CreateDir { path } => {
                    // Get or create the directory
                    layout.get_or_create_dir(path);
                }
                ModifyOp::Delete { path } => {
                    // Remove file from layout
                    if layout.remove_file(path).is_none() {
                        return Err(IsoModifyError::FileNotFound(path.clone()));
                    }
                }
                ModifyOp::Replace { path, data } => {
                    // Find the file and update its extent
                    let file = layout
                        .find_file_mut(path)
                        .ok_or_else(|| IsoModifyError::FileNotFound(path.clone()))?;

                    // Update size (extent sector will be set during write)
                    let size = data.size()?;
                    file.extent = Extent::new(0, size);
                }
            }
        }

        Ok(layout)
    }

    /// Writes new file data and returns a map of paths to extents.
    async fn write_new_data(
        &mut self,
        _layout: &DirectoryLayout,
    ) -> IsoModifyResult<BTreeMap<String, Extent>> {
        let mut file_extents = BTreeMap::new();
        let sector_size = self.sector_size as u32;

        // Start writing after current end
        let mut current_sector = self.end_sector.0 as u32;

        for op in &self.pending_ops {
            match op {
                ModifyOp::AppendFile { path, data } | ModifyOp::Replace { path, data } => {
                    let size = data.size()?;
                    if size == 0 {
                        // Empty files use sector 0
                        file_extents.insert(path.clone(), Extent::new(0, 0));
                        continue;
                    }

                    // Allocate space
                    let extent = Extent::new(current_sector, size);
                    file_extents.insert(path.clone(), extent);

                    // Write data
                    self.inner
                        .seek_sector(LogicalSector(current_sector as usize)).await?;
                    let content = data.read_all()?;
                    self.inner.write_all(&content).await?;

                    // Update current sector
                    current_sector += extent.sector_count(sector_size);
                }
                _ => {}
            }
        }

        // Pad to sector boundary
        self.inner.pad_align_sector().await?;
        self.end_sector = LogicalSector(current_sector as usize);

        Ok(file_extents)
    }

    /// Writes the new session metadata.
    async fn write_new_session(
        &mut self,
        layout: &DirectoryLayout,
        file_extents: BTreeMap<String, Extent>,
    ) -> IsoModifyResult<()> {
        // Build WrittenFiles structure from layout
        let mut written_files = WrittenFiles::new();
        self.build_written_files(layout, &file_extents, &mut written_files, "")?;

        // Write directory records for all entry types
        let mut root_dirs = BTreeMap::new();
        for &ty in &self.entry_types {
            let root_id = written_files.root_dir();
            let dir = written_files.get_mut(&root_id);
            Self::write_directory_static(&mut self.inner, ty, dir).await?;
            if let Some(dir_ref) = dir.entries.get(&ty) {
                root_dirs.insert(ty, *dir_ref);
            }
        }

        // Write path tables
        let mut path_tables = BTreeMap::new();
        let entry_types = self.entry_types.clone();
        for ty in entry_types {
            let l_ref = self.write_path_table(
                ty,
                hadris_common::types::endian::EndianType::LittleEndian,
                &mut written_files,
            ).await?;
            let m_ref = self.write_path_table(
                ty,
                hadris_common::types::endian::EndianType::BigEndian,
                &mut written_files,
            ).await?;
            path_tables.insert(
                ty,
                PathTableRef {
                    lpt: l_ref.extent,
                    mpt: m_ref.extent,
                    size: l_ref.size as u64,
                },
            );
        }

        // Update volume descriptors with new root directories and path tables
        let end_sector = self.inner.pad_align_sector().await?;
        self.update_volume_descriptors(&root_dirs, &path_tables, end_sector).await?;

        Ok(())
    }

    /// Builds WrittenFiles from DirectoryLayout.
    fn build_written_files(
        &self,
        layout: &DirectoryLayout,
        file_extents: &BTreeMap<String, Extent>,
        written_files: &mut WrittenFiles,
        path_prefix: &str,
    ) -> IsoModifyResult<()> {
        for file in &layout.files {
            let full_path = if path_prefix.is_empty() {
                file.name.clone()
            } else {
                alloc::format!("{}/{}", path_prefix, file.name)
            };

            // Look up extent from our written data or existing layout
            let extent = file_extents.get(&full_path).copied().unwrap_or(file.extent);

            let dir_ref = DirectoryRef {
                extent: LogicalSector(extent.sector as usize),
                size: extent.length as usize,
            };

            let root_id = written_files.root_dir();
            let dir = written_files.get_mut(&root_id);
            dir.files.push(WrittenFile {
                name: Arc::new(file.name.clone()),
                entry: dir_ref,
            });
        }

        for subdir in &layout.subdirs {
            let full_path = if path_prefix.is_empty() {
                subdir.name.clone()
            } else {
                alloc::format!("{}/{}", path_prefix, subdir.name)
            };

            // Add subdirectory to written files
            let root_id = written_files.root_dir();
            let dir = written_files.get_mut(&root_id);
            let _subdir_idx = dir.push_dir(Arc::new(subdir.name.clone()));

            // Recurse
            self.build_written_files(subdir, file_extents, written_files, &full_path)?;
        }

        Ok(())
    }

    /// Writes a directory (static version for use in new session).
    async fn write_directory_static<W: Read + Write + Seek>(
        data: &mut IsoCursor<W>,
        ty: EntryType,
        dir: &mut WrittenDirectory,
    ) -> io::Result<()> {
        let start = data.pad_align_sector().await?;

        // Current Directory Entry
        DirectoryRecord::new(b"\x00", &[], DirectoryRef::default(), FileFlags::DIRECTORY)
            .write(&mut *data).await?;

        // Parent Directory Entry
        DirectoryRecord::new(b"\x01", &[], DirectoryRef::default(), FileFlags::DIRECTORY)
            .write(&mut *data).await?;

        for directory in &dir.dirs {
            let WrittenDirectory { name, entries, .. } = directory;
            let flags = FileFlags::DIRECTORY;
            let converted_name = ty.convert_name(name);
            let record = DirectoryRecord::new(
                converted_name.as_bytes(),
                &[],
                *entries.get(&ty).unwrap_or(&DirectoryRef::default()),
                flags,
            );
            record.write(&mut *data).await?;
        }

        for file in &dir.files {
            let WrittenFile { name, entry } = file;
            let flags = FileFlags::empty();
            let converted_name = ty.convert_name(name);
            let record = DirectoryRecord::new(converted_name.as_bytes(), &[], *entry, flags);
            record.write(&mut *data).await?;
        }

        let end = data.pad_align_sector().await?;
        let size = (end.0 - start.0) * data.sector_size;
        dir.entries.insert(
            ty,
            DirectoryRef {
                extent: start,
                size,
            },
        );
        Ok(())
    }

    /// Writes a path table.
    async fn write_path_table(
        &mut self,
        ty: EntryType,
        endian: hadris_common::types::endian::EndianType,
        written_files: &mut WrittenFiles,
    ) -> io::Result<DirectoryRef> {
        let start = self.inner.pad_align_sector().await?;
        PathTableWriter {
            written_files,
            ty,
            endian,
        }
        .write(&mut self.inner).await?;
        let size = self.inner.stream_position().await? as usize - (start.0 * self.sector_size);
        let _end = self.inner.pad_align_sector().await?;
        Ok(DirectoryRef {
            extent: start,
            size,
        })
    }

    /// Updates volume descriptors with new root directories and path tables.
    async fn update_volume_descriptors(
        &mut self,
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        path_tables: &BTreeMap<EntryType, PathTableRef>,
        end_sector: LogicalSector,
    ) -> io::Result<()> {
        self.inner.seek_sector(LogicalSector(16)).await?;

        let mut buffer = [0u8; 2048];
        loop {
            self.inner.read_exact(&mut buffer).await?;
            let header = VolumeDescriptorHeader::from_bytes(&buffer[0..7]);
            let ty = VolumeDescriptorType::from_u8(header.descriptor_type);

            if let VolumeDescriptorType::VolumeSetTerminator = ty {
                break;
            }

            match ty {
                VolumeDescriptorType::PrimaryVolumeDescriptor => {
                    let base_type = self
                        .entry_types
                        .iter()
                        .find(|e| matches!(e, EntryType::Level1 { .. } | EntryType::Level2 { .. }))
                        .expect("no base level found");

                    if let Some(root_dir) = root_dirs.get(base_type)
                        && let Some(pt) = path_tables.get(base_type)
                    {
                        let pvd = bytemuck::from_bytes_mut::<PrimaryVolumeDescriptor>(&mut buffer);
                        pvd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                        pvd.dir_record.header.data_len.write(root_dir.size as u32);
                        pvd.type_l_path_table.set(pt.lpt.0 as u32);
                        pvd.type_m_path_table.set(pt.mpt.0 as u32);
                        pvd.path_table_size.write(pt.size as u32);
                        pvd.volume_space_size.write(end_sector.0 as u32);
                    }
                }
                VolumeDescriptorType::SupplementaryVolumeDescriptor => {
                    let svd =
                        bytemuck::from_bytes_mut::<SupplementaryVolumeDescriptor>(&mut buffer);
                    if svd.header.version == 1 {
                        // Joliet
                        for &level in JolietLevel::all() {
                            if svd.escape_sequences == level.escape_sequence() {
                                let joliet = self
                                    .entry_types
                                    .iter()
                                    .find(|e| matches!(e, EntryType::Joliet { level: jl, .. } if *jl == level));

                                if let Some(joliet) = joliet
                                    && let Some(root_dir) = root_dirs.get(joliet)
                                    && let Some(pt) = path_tables.get(joliet)
                                {
                                    svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                                    svd.dir_record.header.data_len.write(root_dir.size as u32);
                                    svd.type_l_path_table.set(pt.lpt.0 as u32);
                                    svd.type_m_path_table.set(pt.mpt.0 as u32);
                                    svd.path_table_size.write(pt.size as u32);
                                    svd.volume_space_size.write(end_sector.0 as u32);
                                }
                            }
                        }
                    }
                }
                _ => continue,
            }

            // Write back the modified descriptor
            self.inner.seek_relative(-(buffer.len() as i64)).await?;
            self.inner.write_all(&buffer).await?;
        }

        Ok(())
    }

    /// Splits a path into (directory, filename).
    fn split_path(path: &str) -> IsoModifyResult<(String, String)> {
        hadris_common::path::split_path(path)
            .ok_or_else(|| IsoModifyError::InvalidPath(path.to_string()))
    }
}
} // io_transform!

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_split_path() {
        let (dir, file) = IsoModifier::<std::io::Cursor<Vec<u8>>>::split_path("test.txt").unwrap();
        assert_eq!(dir, "");
        assert_eq!(file, "test.txt");

        let (dir, file) =
            IsoModifier::<std::io::Cursor<Vec<u8>>>::split_path("docs/readme.txt").unwrap();
        assert_eq!(dir, "docs");
        assert_eq!(file, "readme.txt");

        let (dir, file) =
            IsoModifier::<std::io::Cursor<Vec<u8>>>::split_path("a/b/c/d.txt").unwrap();
        assert_eq!(dir, "a/b/c");
        assert_eq!(file, "d.txt");
    }

    #[test]
    fn test_file_data() {
        let data = FileData::from(vec![1, 2, 3, 4]);
        assert_eq!(data.size().unwrap(), 4);
        assert_eq!(data.read_all().unwrap(), vec![1, 2, 3, 4]);

        let data = FileData::from(&[5, 6, 7][..]);
        assert_eq!(data.size().unwrap(), 3);
    }
}
