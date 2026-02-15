//! exFAT Directory iteration.
//!
//! Provides directory traversal for exFAT filesystems.

use alloc::vec::Vec;
use core::mem::size_of;

use crate::error::{FatError, Result};
use crate::io::{Read, Seek};

use super::entry::{ExFatFileEntry, RawDirectoryEntry, entry_type, parse_entry_set};
use super::fs::ExFatFs;

/// A directory in an exFAT filesystem.
pub struct ExFatDir<'a, DATA: Read + Seek> {
    /// Reference to the filesystem
    pub(crate) fs: &'a ExFatFs<DATA>,
    /// First cluster of the directory
    pub(crate) first_cluster: u32,
    /// Whether the directory is stored contiguously
    pub(crate) is_contiguous: bool,
    /// Size of the directory in bytes (for contiguous dirs)
    pub(crate) size: u64,
}

impl<'a, DATA: Read + Seek> ExFatDir<'a, DATA> {
    /// Create an iterator over directory entries.
    pub fn entries(&self) -> ExFatDirIter<'a, DATA> {
        ExFatDirIter {
            fs: self.fs,
            first_cluster: self.first_cluster,
            is_contiguous: self.is_contiguous,
            dir_size: self.size,
            current_cluster: self.first_cluster,
            cluster_offset: 0,
            dir_offset: 0,
        }
    }

    /// Find an entry by name.
    ///
    /// Performs case-insensitive comparison using the up-case table.
    pub fn find(&self, name: &str) -> Result<Option<ExFatFileEntry>> {
        for entry in self.entries() {
            let entry = entry?;

            // Perform case-insensitive comparison
            if self.fs.names_equal(&entry.name, name)? {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    /// Open a subdirectory by name.
    pub fn open_dir(&self, name: &str) -> Result<ExFatDir<'a, DATA>> {
        let entry = self.find(name)?.ok_or(FatError::EntryNotFound)?;

        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }

        Ok(ExFatDir {
            fs: self.fs,
            first_cluster: entry.first_cluster,
            is_contiguous: entry.no_fat_chain,
            size: entry.data_length,
        })
    }
}

/// Iterator over exFAT directory entries.
pub struct ExFatDirIter<'a, DATA: Read + Seek> {
    /// Reference to the filesystem
    fs: &'a ExFatFs<DATA>,
    /// First cluster of the directory
    first_cluster: u32,
    /// Whether the directory is stored contiguously
    is_contiguous: bool,
    /// Total directory size (for contiguous dirs, or 0 for unknown)
    dir_size: u64,
    /// Current cluster being read
    current_cluster: u32,
    /// Byte offset within current cluster
    cluster_offset: usize,
    /// Total byte offset from start of directory
    dir_offset: u64,
}

impl<DATA: Read + Seek> Iterator for ExFatDirIter<'_, DATA> {
    type Item = Result<ExFatFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Try to read the next entry
            match self.read_next_entry() {
                Ok(Some(entry)) => return Some(Ok(entry)),
                Ok(None) => return None, // End of directory
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

impl<DATA: Read + Seek> ExFatDirIter<'_, DATA> {
    /// Read the next file entry from the directory.
    fn read_next_entry(&mut self) -> Result<Option<ExFatFileEntry>> {
        let info = self.fs.info();
        let entry_size = size_of::<RawDirectoryEntry>();
        let cluster_size = info.bytes_per_cluster;

        loop {
            // Check if we've reached a size limit (for contiguous directories)
            if self.dir_size > 0 && self.dir_offset >= self.dir_size {
                return Ok(None);
            }

            // Check if we need to move to the next cluster
            if self.cluster_offset >= cluster_size {
                if self.is_contiguous {
                    // Contiguous: just increment cluster
                    self.current_cluster += 1;
                    if !info.is_valid_cluster(self.current_cluster) {
                        return Ok(None);
                    }
                } else {
                    // Follow FAT chain
                    match self.fs.next_cluster(self.current_cluster)? {
                        Some(next) => self.current_cluster = next,
                        None => return Ok(None),
                    }
                }
                self.cluster_offset = 0;
            }

            // Read a single entry
            let offset = info.cluster_to_offset(self.current_cluster) + self.cluster_offset as u64;
            let raw_entry = self.fs.read_entry_at(offset)?;

            let entry_type = unsafe { raw_entry.entry_type };

            // Check for end of directory
            if entry_type == entry_type::END_OF_DIRECTORY {
                return Ok(None);
            }

            self.cluster_offset += entry_size;
            self.dir_offset += entry_size as u64;

            // Skip deleted entries and non-file entries
            if entry_type == entry_type::DELETED_FILE {
                continue;
            }

            // Skip critical system entries (bitmap, upcase, volume label)
            if entry_type == entry_type::ALLOCATION_BITMAP
                || entry_type == entry_type::UPCASE_TABLE
                || entry_type == entry_type::VOLUME_LABEL
                || entry_type == entry_type::VOLUME_GUID
                || entry_type == entry_type::TEXFAT_PADDING
                || entry_type == entry_type::ACCESS_CONTROL
            {
                continue;
            }

            // File directory entry - need to read the complete entry set
            if entry_type == entry_type::FILE_DIRECTORY {
                let entry_offset = self.dir_offset - entry_size as u64;
                return self.read_entry_set(raw_entry, entry_offset);
            }

            // Skip any other entry types
        }
    }

    /// Read a complete file entry set starting from a File Directory Entry.
    fn read_entry_set(
        &mut self,
        primary: RawDirectoryEntry,
        entry_offset: u64,
    ) -> Result<Option<ExFatFileEntry>> {
        let info = self.fs.info();
        let entry_size = size_of::<RawDirectoryEntry>();
        let cluster_size = info.bytes_per_cluster;

        let primary_entry = unsafe { &primary.file };
        let secondary_count = primary_entry.secondary_count as usize;

        if secondary_count < 2 || secondary_count > 18 {
            // Invalid entry, skip
            return Ok(None);
        }

        // Read all entries in the set
        let mut entries = Vec::with_capacity(1 + secondary_count);
        entries.push(primary);

        for _ in 0..secondary_count {
            // Check if we've reached the directory size limit
            if self.dir_size > 0 && self.dir_offset >= self.dir_size {
                return Ok(None);
            }

            // Check if we need to move to the next cluster
            if self.cluster_offset >= cluster_size {
                if self.is_contiguous {
                    self.current_cluster += 1;
                    if !info.is_valid_cluster(self.current_cluster) {
                        return Ok(None);
                    }
                } else {
                    match self.fs.next_cluster(self.current_cluster)? {
                        Some(next) => self.current_cluster = next,
                        None => return Ok(None),
                    }
                }
                self.cluster_offset = 0;
            }

            let offset = info.cluster_to_offset(self.current_cluster) + self.cluster_offset as u64;
            let entry = self.fs.read_entry_at(offset)?;
            entries.push(entry);

            self.cluster_offset += entry_size;
            self.dir_offset += entry_size as u64;
        }

        // Parse the entry set
        if let Some((mut file_entry, _)) = parse_entry_set(&entries) {
            file_entry.parent_cluster = self.first_cluster;
            file_entry.entry_offset = entry_offset;
            Ok(Some(file_entry))
        } else {
            // Failed to parse, skip
            Ok(None)
        }
    }
}
