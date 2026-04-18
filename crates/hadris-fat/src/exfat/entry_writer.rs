//! exFAT Directory Entry Set Builder.
//!
//! This module provides utilities for building exFAT directory entry sets.
//! Each file or directory in exFAT is represented by an entry set consisting of:
//! - 1 File Directory Entry (primary)
//! - 1 Stream Extension Entry (secondary)
//! - 1-17 File Name Entries (secondary)
//!
//! Total: 3-19 entries per file/directory.

use alloc::string::String;
use alloc::vec::Vec;

use hadris_common::types::endian::{Endian, LittleEndian};
use hadris_common::types::number::{U16, U32, U64};

use super::entry::{
    FileAttributes, RawDirectoryEntry, RawFileDirectoryEntry, RawFileNameEntry,
    RawStreamExtensionEntry, compute_entry_set_checksum, entry_type,
};
use super::time::ExFatTimestamp;
use super::upcase::UpcaseTable;
use crate::error::{FatError, Result};

/// Maximum filename length in UTF-16 code units.
pub const MAX_NAME_LENGTH: usize = 255;

/// Characters per filename entry.
const CHARS_PER_NAME_ENTRY: usize = 15;

/// Builder for creating exFAT directory entry sets.
#[derive(Debug, Clone)]
pub struct EntrySetBuilder {
    /// File/directory name
    name: String,
    /// File attributes
    attributes: FileAttributes,
    /// First cluster (0 for empty files)
    first_cluster: u32,
    /// Valid data length
    valid_data_length: u64,
    /// Allocated data length
    data_length: u64,
    /// Whether the data is stored contiguously (NoFatChain flag)
    is_contiguous: bool,
    /// Creation timestamp
    created: ExFatTimestamp,
    /// Modification timestamp
    modified: ExFatTimestamp,
    /// Access timestamp
    accessed: ExFatTimestamp,
}

impl EntrySetBuilder {
    /// Create a builder for a new file entry.
    pub fn file(name: &str) -> Result<Self> {
        Self::new(name, FileAttributes::ARCHIVE)
    }

    /// Create a builder for a new directory entry.
    pub fn directory(name: &str) -> Result<Self> {
        Self::new(name, FileAttributes::DIRECTORY)
    }

    /// Create a new entry set builder.
    fn new(name: &str, attributes: FileAttributes) -> Result<Self> {
        // Validate name length
        let name_len: usize = name.encode_utf16().count();
        if name_len == 0 || name_len > MAX_NAME_LENGTH {
            return Err(FatError::InvalidFilename);
        }

        // Check for invalid characters
        for c in name.chars() {
            if is_invalid_char(c) {
                return Err(FatError::InvalidFilename);
            }
        }

        let now = ExFatTimestamp::now();

        Ok(Self {
            name: name.into(),
            attributes,
            first_cluster: 0,
            valid_data_length: 0,
            data_length: 0,
            is_contiguous: true, // Default to contiguous
            created: now.clone(),
            modified: now.clone(),
            accessed: now,
        })
    }

    /// Set the first cluster of the file/directory data.
    pub fn with_cluster(mut self, cluster: u32) -> Self {
        self.first_cluster = cluster;
        self
    }

    /// Set the data sizes.
    pub fn with_size(mut self, valid_length: u64, allocated_length: u64) -> Self {
        self.valid_data_length = valid_length;
        self.data_length = allocated_length;
        self
    }

    /// Set whether the data is stored contiguously.
    pub fn with_contiguous(mut self, contiguous: bool) -> Self {
        self.is_contiguous = contiguous;
        self
    }

    /// Set the timestamps.
    pub fn with_timestamps(
        mut self,
        created: ExFatTimestamp,
        modified: ExFatTimestamp,
        accessed: ExFatTimestamp,
    ) -> Self {
        self.created = created;
        self.modified = modified;
        self.accessed = accessed;
        self
    }

    /// Build the directory entry set.
    ///
    /// Returns a vector of raw directory entries that can be written to disk.
    pub fn build(&self, upcase: &UpcaseTable) -> Vec<RawDirectoryEntry> {
        // Calculate number of name entries needed
        let name_utf16: Vec<u16> = self.name.encode_utf16().collect();
        let name_entry_count = (name_utf16.len() + CHARS_PER_NAME_ENTRY - 1) / CHARS_PER_NAME_ENTRY;
        let secondary_count = 1 + name_entry_count; // Stream + name entries

        let mut entries = Vec::with_capacity(1 + secondary_count);

        // Build File Directory Entry (primary)
        let file_entry = self.build_file_entry(secondary_count as u8);
        entries.push(to_raw_entry(&file_entry));

        // Build Stream Extension Entry
        let name_hash = upcase.name_hash(&self.name);
        let stream_entry = self.build_stream_entry(name_utf16.len() as u8, name_hash);
        entries.push(to_raw_entry(&stream_entry));

        // Build File Name Entries
        for (i, chunk) in name_utf16.chunks(CHARS_PER_NAME_ENTRY).enumerate() {
            let name_entry = self.build_name_entry(chunk, i == 0);
            entries.push(to_raw_entry(&name_entry));
        }

        // Compute and set checksum
        let checksum = compute_entry_set_checksum(&entries);
        let file_entry_bytes = unsafe { &mut entries[0].file };
        file_entry_bytes.set_checksum = U16::<LittleEndian>::new(checksum);

        entries
    }

    /// Calculate the number of directory entries this entry set will use.
    pub fn entry_count(&self) -> usize {
        let name_utf16_len = self.name.encode_utf16().count();
        let name_entry_count = (name_utf16_len + CHARS_PER_NAME_ENTRY - 1) / CHARS_PER_NAME_ENTRY;
        1 + 1 + name_entry_count // File + Stream + Name entries
    }

    fn build_file_entry(&self, secondary_count: u8) -> RawFileDirectoryEntry {
        let (create_ts, create_10ms, create_utc) = self.created.to_raw();
        let (modify_ts, modify_10ms, modify_utc) = self.modified.to_raw();
        let (access_ts, _, access_utc) = self.accessed.to_raw();

        RawFileDirectoryEntry {
            entry_type: entry_type::FILE_DIRECTORY,
            secondary_count,
            set_checksum: U16::<LittleEndian>::new(0), // Filled in later
            file_attributes: U16::<LittleEndian>::new(self.attributes.bits()),
            reserved1: U16::<LittleEndian>::new(0),
            create_timestamp: U32::<LittleEndian>::new(create_ts),
            last_modified_timestamp: U32::<LittleEndian>::new(modify_ts),
            last_accessed_timestamp: U32::<LittleEndian>::new(access_ts),
            create_10ms_increment: create_10ms,
            last_modified_10ms_increment: modify_10ms,
            create_utc_offset: create_utc,
            last_modified_utc_offset: modify_utc,
            last_accessed_utc_offset: access_utc,
            reserved2: [0; 7],
        }
    }

    fn build_stream_entry(&self, name_length: u8, name_hash: u16) -> RawStreamExtensionEntry {
        // General secondary flags:
        // Bit 0: AllocationPossible (1 if a cluster allocation exists)
        // Bit 1: NoFatChain (1 if contiguous; requires AllocationPossible=1)
        // Per exFAT spec, both bits must be 0 when the file has no clusters.
        let has_allocation = self.data_length > 0 || self.first_cluster != 0;
        let flags = if has_allocation {
            0x01 | if self.is_contiguous { 0x02 } else { 0x00 }
        } else {
            0
        };

        RawStreamExtensionEntry {
            entry_type: entry_type::STREAM_EXTENSION,
            general_secondary_flags: flags,
            reserved1: 0,
            name_length,
            name_hash: U16::<LittleEndian>::new(name_hash),
            reserved2: U16::<LittleEndian>::new(0),
            valid_data_length: U64::<LittleEndian>::new(self.valid_data_length),
            reserved3: U32::<LittleEndian>::new(0),
            first_cluster: U32::<LittleEndian>::new(self.first_cluster),
            data_length: U64::<LittleEndian>::new(self.data_length),
        }
    }

    fn build_name_entry(&self, chars: &[u16], _is_first: bool) -> RawFileNameEntry {
        let mut file_name = [0u8; 30];

        // Copy UTF-16 characters into the entry
        for (i, &code_unit) in chars.iter().enumerate() {
            if i >= CHARS_PER_NAME_ENTRY {
                break;
            }
            let bytes = code_unit.to_le_bytes();
            file_name[i * 2] = bytes[0];
            file_name[i * 2 + 1] = bytes[1];
        }

        RawFileNameEntry {
            entry_type: entry_type::FILE_NAME,
            general_secondary_flags: 0, // No flags for name entries
            file_name,
        }
    }
}

/// Check if a character is invalid for exFAT filenames.
fn is_invalid_char(c: char) -> bool {
    matches!(
        c,
        '\0'..='\x1F' | '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|'
    )
}

/// Convert a typed entry to a raw directory entry union.
fn to_raw_entry<T: bytemuck::NoUninit>(entry: &T) -> RawDirectoryEntry {
    let mut raw = RawDirectoryEntry { bytes: [0; 32] };
    let bytes = bytemuck::bytes_of(entry);
    unsafe {
        raw.bytes[..bytes.len()].copy_from_slice(bytes);
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_set_builder() {
        let upcase = UpcaseTable::create_default();
        let builder = EntrySetBuilder::file("test.txt").unwrap();
        let entries = builder.build(&upcase);

        // Should have 3 entries: File + Stream + 1 Name
        assert_eq!(entries.len(), 3);

        // Check entry types
        assert_eq!(unsafe { entries[0].entry_type }, entry_type::FILE_DIRECTORY);
        assert_eq!(
            unsafe { entries[1].entry_type },
            entry_type::STREAM_EXTENSION
        );
        assert_eq!(unsafe { entries[2].entry_type }, entry_type::FILE_NAME);
    }

    #[test]
    fn test_long_name_multiple_entries() {
        let upcase = UpcaseTable::create_default();
        // Name with more than 15 characters
        let builder = EntrySetBuilder::file("this_is_a_very_long_filename.txt").unwrap();
        let entries = builder.build(&upcase);

        // Should have 5 entries: File + Stream + 3 Name (33 chars / 15 = 3)
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn test_invalid_filename() {
        // Names with invalid characters should fail
        assert!(EntrySetBuilder::file("test*.txt").is_err());
        assert!(EntrySetBuilder::file("test:file").is_err());
        assert!(EntrySetBuilder::file("").is_err());
    }
}
