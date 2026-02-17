use core::mem::size_of;
use core::ops::DerefMut;

use hadris_common::types::endian::Endian;

use crate::error::{FatError, Result};
use crate::file::ShortFileName;
#[cfg(feature = "lfn")]
use crate::file::{LfnBuilder, LongFileName};
use crate::fs::FatFs;
#[cfg(not(feature = "alloc"))]
use crate::io::ReadExt;
use crate::io::{Cluster, ClusterLike, Read, Seek, SeekFrom};
use crate::raw::{DirEntryAttrFlags, RawDirectoryEntry};
use crate::read::FileReader;

pub struct FatDir<'a, DATA: Read + Seek> {
    pub(crate) data: &'a FatFs<DATA>,
    /// Cluster for subdirectories, or 0 (sentinel) for FAT12/16 fixed root
    pub(crate) cluster: Cluster,
    /// For FAT12/16 root: (start_byte, size_bytes), None for cluster-based dirs
    pub(crate) fixed_root: Option<(usize, usize)>,
}

impl<'a, DATA: Read + Seek> FatDir<'a, DATA> {
    #[cfg(feature = "lfn")]
    pub fn entries(&self) -> FatDirIter<'a, DATA> {
        FatDirIter {
            data: self.data,
            cluster: self.cluster,
            offset: 0,
            fixed_root_remaining: self.fixed_root.map(|(_, size)| size),
            fixed_root_start: self.fixed_root.map(|(start, _)| start),
            lfn_builder: LfnBuilder::new(),
            #[cfg(feature = "alloc")]
            cluster_buffer: None,
            #[cfg(feature = "alloc")]
            buffer_valid: false,
        }
    }

    #[cfg(not(feature = "lfn"))]
    pub fn entries(&self) -> FatDirIter<'a, DATA> {
        FatDirIter {
            data: self.data,
            cluster: self.cluster,
            offset: 0,
            fixed_root_remaining: self.fixed_root.map(|(_, size)| size),
            fixed_root_start: self.fixed_root.map(|(start, _)| start),
            #[cfg(feature = "alloc")]
            cluster_buffer: None,
            #[cfg(feature = "alloc")]
            buffer_valid: false,
        }
    }

    /// Open a subdirectory from a file entry.
    ///
    /// The entry must be a directory.
    pub fn open_entry(&self, entry: &FileEntry) -> Result<FatDir<'a, DATA>> {
        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }
        Ok(FatDir {
            data: self.data,
            cluster: entry.cluster(),
            fixed_root: None, // Subdirectories are never fixed root
        })
    }

    /// Find an entry by name.
    ///
    /// When the `lfn` feature is enabled, this performs a case-sensitive match
    /// against long file names first, then falls back to case-insensitive short
    /// name matching.
    ///
    /// Without the `lfn` feature, only case-insensitive short name matching is used.
    pub fn find(&self, name: &str) -> Result<Option<FileEntry>> {
        for entry in self.entries() {
            let DirectoryEntry::Entry(file_entry) = entry?;

            // Check LFN match (case-sensitive)
            #[cfg(feature = "lfn")]
            if let Some(lfn) = file_entry.long_name()
                && lfn.as_str() == name
            {
                return Ok(Some(file_entry));
            }
            // Check short name match (case-insensitive, handles 8.3 padding)
            if file_entry.short_name().matches(name) {
                return Ok(Some(file_entry));
            }
        }
        Ok(None)
    }

    /// Open a subdirectory by name.
    ///
    /// Returns an error if the entry is not found or is not a directory.
    pub fn open_dir(&self, name: &str) -> Result<FatDir<'a, DATA>> {
        let entry = self.find(name)?.ok_or(FatError::EntryNotFound)?;

        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }

        // Subdirectories always use cluster chains, never fixed root
        Ok(FatDir {
            data: self.data,
            cluster: entry.cluster(),
            fixed_root: None,
        })
    }

    /// Open a file for reading by name.
    ///
    /// Returns an error if the entry is not found or is a directory.
    pub fn open_file(&self, name: &str) -> Result<FileReader<'a, DATA>> {
        let entry = self.find(name)?.ok_or(FatError::EntryNotFound)?;
        FileReader::new(self.data, &entry)
    }
}

pub struct FatDirIter<'a, DATA: Read + Seek> {
    data: &'a FatFs<DATA>,
    /// Current cluster (or 0 for fixed root directory)
    cluster: Cluster,
    /// Offset within current cluster (or within fixed root dir)
    offset: usize,
    /// For fixed root directory: remaining bytes to read (None for cluster-based)
    fixed_root_remaining: Option<usize>,
    /// For fixed root directory: start byte offset
    fixed_root_start: Option<usize>,
    #[cfg(feature = "lfn")]
    lfn_builder: LfnBuilder,
    /// Buffered cluster data (reduces seeks by reading entire cluster at once)
    #[cfg(feature = "alloc")]
    cluster_buffer: Option<alloc::vec::Vec<u8>>,
    /// Whether the buffer is valid for the current cluster
    #[cfg(feature = "alloc")]
    buffer_valid: bool,
}

impl<DATA: Read + Seek> Iterator for FatDirIter<'_, DATA> {
    type Item = Result<DirectoryEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut data = self.data.data.lock();
        let entry_size = size_of::<RawDirectoryEntry>();
        let cluster_size = data.cluster_size;

        loop {
            // Check bounds and handle cluster transitions
            if let Some(ref mut remaining) = self.fixed_root_remaining {
                // Fixed root directory (FAT12/16)
                if *remaining < entry_size {
                    return None; // End of fixed root directory
                }
            } else {
                // Cluster-based directory (FAT32 or subdirectory)
                // Check if we need to move to the next cluster
                if self.offset >= cluster_size {
                    let next = match self.data.fat.next_cluster(data.deref_mut(), self.cluster.0) {
                        Ok(n) => n,
                        Err(e) => return Some(Err(e)),
                    };
                    match next {
                        Some(cluster) => {
                            self.cluster.0 = cluster as usize;
                            self.offset = 0;
                            #[cfg(feature = "alloc")]
                            {
                                self.buffer_valid = false;
                            }
                        }
                        None => return None, // End of directory
                    }
                }
            }

            // Read the entry - use buffering when alloc is available
            #[cfg(feature = "alloc")]
            let raw_entry = {
                // Ensure buffer is filled
                if !self.buffer_valid || self.cluster_buffer.is_none() {
                    let buffer_size = if let Some(remaining) = self.fixed_root_remaining {
                        // For fixed root, buffer the remaining bytes (up to a reasonable size)
                        remaining.min(4096)
                    } else {
                        cluster_size
                    };

                    let seek_pos = if self.fixed_root_remaining.is_some() {
                        let start = self.fixed_root_start.unwrap();
                        start as u64
                    } else {
                        self.cluster
                            .to_bytes(self.data.info.data_start, cluster_size)
                            as u64
                    };

                    if let Err(e) = data.seek(SeekFrom::Start(seek_pos)) {
                        return Some(Err(FatError::Io(e)));
                    }

                    let mut buffer = alloc::vec![0u8; buffer_size];
                    if let Err(e) = data.read_exact(&mut buffer) {
                        return Some(Err(FatError::Io(e)));
                    }

                    self.cluster_buffer = Some(buffer);
                    self.buffer_valid = true;
                }

                // Read entry from buffer
                let buffer = self.cluster_buffer.as_ref().unwrap();
                let offset = self.offset;

                if offset + entry_size > buffer.len() {
                    // Buffer exhausted, need to handle this case
                    // For fixed root: we're done
                    // For cluster-based: handled by cluster transition above
                    if self.fixed_root_remaining.is_some() {
                        return None;
                    }
                    continue;
                }

                let entry_bytes: [u8; 32] = buffer[offset..offset + entry_size].try_into().unwrap();

                // Safety: RawDirectoryEntry is a union of properly aligned types
                // and entry_bytes has the correct size
                unsafe { core::mem::transmute::<[u8; 32], RawDirectoryEntry>(entry_bytes) }
            };

            #[cfg(not(feature = "alloc"))]
            let raw_entry = {
                // Calculate seek position
                let seek_pos = if self.fixed_root_remaining.is_some() {
                    let start = self.fixed_root_start.unwrap();
                    (start + self.offset) as u64
                } else {
                    self.cluster
                        .to_bytes(self.data.info.data_start, cluster_size)
                        as u64
                        + self.offset as u64
                };

                if let Err(e) = data.seek(SeekFrom::Start(seek_pos)) {
                    return Some(Err(FatError::Io(e)));
                }

                // Read the directory entry
                match data.read_struct::<RawDirectoryEntry>() {
                    Ok(e) => e,
                    Err(e) => return Some(Err(FatError::Io(e))),
                }
            };

            let entry_bytes = unsafe { raw_entry.bytes };

            // Check for end of directory
            if entry_bytes[0] == 0 {
                #[cfg(feature = "lfn")]
                self.lfn_builder.reset();
                return None;
            }

            // Check for deleted entry
            if entry_bytes[0] == 0xE5 {
                self.offset += entry_size;
                if let Some(ref mut remaining) = self.fixed_root_remaining {
                    *remaining = remaining.saturating_sub(entry_size);
                }
                #[cfg(feature = "lfn")]
                self.lfn_builder.reset(); // Deleted entry breaks LFN sequence
                continue;
            }

            self.offset += entry_size;
            if let Some(ref mut remaining) = self.fixed_root_remaining {
                *remaining = remaining.saturating_sub(entry_size);
            }

            // Check if this is an LFN entry (attributes == LONG_NAME)
            #[cfg(feature = "lfn")]
            {
                let entry_attr = unsafe { raw_entry.file }.attributes;
                if entry_attr == DirEntryAttrFlags::LONG_NAME.bits() {
                    // This is an LFN entry
                    let lfn = unsafe { raw_entry.lfn };
                    let seq = lfn.sequence_number;

                    // Check if this is the start of a new LFN sequence (has 0x40 bit set)
                    if seq & LfnBuilder::LAST_ENTRY_MASK != 0 {
                        self.lfn_builder.start(seq, lfn.checksum);
                    }

                    if self.lfn_builder.building {
                        self.lfn_builder.add_entry(
                            seq,
                            lfn.checksum,
                            &lfn.name1,
                            &lfn.name2,
                            &lfn.name3,
                        );
                    }
                    continue;
                }
            }

            // This is a regular file/directory entry
            let file_entry = unsafe { raw_entry.file };

            // Convert 0x05 back to 0xE5 for kanji compatibility
            let mut name_bytes = file_entry.name;
            if name_bytes[0] == 0x05 {
                name_bytes[0] = 0xE5;
            }

            let short_name = match ShortFileName::new(name_bytes) {
                Ok(n) => n,
                Err(_) => return Some(Err(FatError::InvalidShortFilename)),
            };

            // Try to get the LFN if we've been building one
            #[cfg(feature = "lfn")]
            let long_name = self.lfn_builder.finish(&short_name);

            // For FAT12/16 with fixed root dir, parent_clus is 0 (sentinel)
            // For cluster-based dirs, parent_clus is the actual cluster
            return Some(Ok(DirectoryEntry::Entry(FileEntry {
                short_name,
                #[cfg(feature = "lfn")]
                long_name,
                attr: DirEntryAttrFlags::from_bits_retain(file_entry.attributes),
                size: file_entry.size.get() as usize,
                parent_clus: self.cluster,
                offset_within_cluster: self.offset - entry_size,
                cluster: Cluster::from_parts(
                    file_entry.first_cluster_high.get(),
                    file_entry.first_cluster_low.get(),
                ),
            })));
        }
    }
}

#[derive(Debug)]
pub enum DirectoryEntry {
    /// A file or directory entry
    Entry(FileEntry),
}

impl DirectoryEntry {
    /// Get the display name of the entry.
    /// Returns the long filename if available, otherwise the short name.
    pub fn name(&self) -> &str {
        match self {
            Self::Entry(ent) => ent.name(),
        }
    }

    /// Get the file entry if this is an Entry variant
    pub fn as_entry(&self) -> Option<&FileEntry> {
        match self {
            Self::Entry(ent) => Some(ent),
        }
    }
}

#[derive(Debug)]
pub struct ParseInfo<T> {
    pub data: T,
    pub warnings: FileSystemWarnings,
    pub errors: FileSystemErrors,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct FileSystemWarnings: u64 {

    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct FileSystemErrors: u64 {

    }
}

#[derive(Debug)]
pub struct FileEntry {
    pub(crate) short_name: ShortFileName,
    #[cfg(feature = "lfn")]
    pub(crate) long_name: Option<LongFileName>,
    pub(crate) attr: DirEntryAttrFlags,
    pub(crate) size: usize,
    /// Parent directory cluster (used for write operations)
    #[cfg_attr(not(feature = "write"), allow(dead_code))]
    pub(crate) parent_clus: Cluster<usize>,
    /// Offset of this entry within the parent cluster (used for write operations)
    #[cfg_attr(not(feature = "write"), allow(dead_code))]
    pub(crate) offset_within_cluster: usize,
    pub(crate) cluster: Cluster<usize>,
}

impl FileEntry {
    /// Get the file's display name.
    /// Returns the long filename if available, otherwise the short name.
    pub fn name(&self) -> &str {
        #[cfg(feature = "lfn")]
        if let Some(ref lfn) = self.long_name {
            return lfn.as_str();
        }
        self.short_name.as_str()
    }

    /// Get the short (8.3) filename
    pub fn short_name(&self) -> &ShortFileName {
        &self.short_name
    }

    /// Get the long filename, if available
    #[cfg(feature = "lfn")]
    pub fn long_name(&self) -> Option<&LongFileName> {
        self.long_name.as_ref()
    }

    /// Get the file attributes
    pub fn attributes(&self) -> DirEntryAttrFlags {
        self.attr
    }

    /// Check if this entry is a directory
    pub fn is_directory(&self) -> bool {
        self.attr.contains(DirEntryAttrFlags::DIRECTORY)
    }

    /// Check if this entry is a regular file
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }

    /// Get the file size in bytes (0 for directories)
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the first cluster of the file data
    pub fn cluster(&self) -> Cluster<usize> {
        self.cluster
    }
}
