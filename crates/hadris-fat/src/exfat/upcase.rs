//! exFAT Up-case Table implementation.
//!
//! The up-case table is used for case-insensitive filename comparisons.
//! It maps Unicode code points to their uppercase equivalents.

use alloc::vec;
use alloc::vec::Vec;

use crate::error::Result;
use crate::io::{Read, Seek, SeekFrom};

use super::ExFatInfo;

/// Up-case table for case-insensitive filename matching.
///
/// The table contains 65536 entries (one for each BMP code point).
/// Compressed tables use a special encoding to save space.
pub struct UpcaseTable {
    /// The table data (65536 u16 entries for full table)
    data: Vec<u16>,
    /// Whether the table is valid
    valid: bool,
}

impl UpcaseTable {
    /// Marker for compressed range (0xFFFF followed by count)
    const COMPRESSION_MARKER: u16 = 0xFFFF;

    /// Create an empty (invalid) up-case table.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            valid: false,
        }
    }

    /// Load the up-case table from disk.
    ///
    /// # Arguments
    /// * `data` - The data source to read from
    /// * `info` - Filesystem info
    /// * `first_cluster` - First cluster of the up-case table
    /// * `size` - Size of the table in bytes
    /// * `is_contiguous` - Whether the table is stored contiguously
    pub fn load<DATA: Read + Seek>(
        &mut self,
        data: &mut DATA,
        info: &ExFatInfo,
        first_cluster: u32,
        size: u64,
        is_contiguous: bool,
    ) -> Result<()> {
        // Read the raw table data
        let mut raw_data = vec![0u8; size as usize];

        if is_contiguous {
            // Read contiguous data
            let offset = info.cluster_to_offset(first_cluster);
            data.seek(SeekFrom::Start(offset))?;
            data.read_exact(&mut raw_data)?;
        } else {
            // TODO: Handle fragmented table by following FAT chain
            // For now, assume contiguous
            let offset = info.cluster_to_offset(first_cluster);
            data.seek(SeekFrom::Start(offset))?;
            data.read_exact(&mut raw_data)?;
        }

        // Decompress the table
        self.decompress(&raw_data)?;
        self.valid = true;

        Ok(())
    }

    /// Decompress the up-case table from raw bytes.
    ///
    /// The table uses a simple compression scheme:
    /// - 0xFFFF followed by a count N means "next N code points map to themselves"
    fn decompress(&mut self, raw: &[u8]) -> Result<()> {
        self.data.clear();
        self.data.reserve(65536);

        let mut i = 0;
        let mut code_point: u16 = 0;

        while i + 1 < raw.len() && (code_point as usize) < 65536 {
            let value = u16::from_le_bytes([raw[i], raw[i + 1]]);
            i += 2;

            if value == Self::COMPRESSION_MARKER && i + 1 < raw.len() {
                // Compressed range: next count values map to themselves
                let count = u16::from_le_bytes([raw[i], raw[i + 1]]);
                i += 2;

                for _ in 0..count {
                    if (code_point as usize) >= 65536 {
                        break;
                    }
                    self.data.push(code_point);
                    code_point = code_point.wrapping_add(1);
                }
            } else {
                // Direct mapping
                self.data.push(value);
                code_point = code_point.wrapping_add(1);
            }
        }

        // Fill remaining entries with identity mapping
        while (code_point as usize) < 65536 {
            self.data.push(code_point);
            code_point = code_point.wrapping_add(1);
        }

        // Truncate to exactly 65536 entries
        self.data.truncate(65536);

        Ok(())
    }

    /// Convert a character to uppercase.
    pub fn to_upper(&self, c: u16) -> u16 {
        if self.valid && (c as usize) < self.data.len() {
            self.data[c as usize]
        } else {
            c
        }
    }

    /// Check if two names are equal (case-insensitive).
    pub fn names_equal(&self, name1: &str, name2: &str) -> bool {
        let chars1: Vec<u16> = name1.encode_utf16().collect();
        let chars2: Vec<u16> = name2.encode_utf16().collect();

        if chars1.len() != chars2.len() {
            return false;
        }

        for (&c1, &c2) in chars1.iter().zip(chars2.iter()) {
            if self.to_upper(c1) != self.to_upper(c2) {
                return false;
            }
        }

        true
    }

    /// Compute the name hash for a filename.
    ///
    /// The hash is used in Stream Extension entries for quick comparison.
    pub fn name_hash(&self, name: &str) -> u16 {
        let mut hash: u16 = 0;

        for code_unit in name.encode_utf16() {
            let upper = self.to_upper(code_unit);
            // Process low byte
            hash = hash.rotate_right(1).wrapping_add((upper & 0xFF) as u16);
            // Process high byte
            hash = hash.rotate_right(1).wrapping_add((upper >> 8) as u16);
        }

        hash
    }

    /// Check if the table is valid (has been loaded).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Create a default up-case table with ASCII-only case conversion.
    ///
    /// This can be used as a fallback if the table cannot be loaded.
    pub fn create_default() -> Self {
        let mut data = Vec::with_capacity(65536);

        for i in 0u16..=65535 {
            let upper = if i >= 0x61 && i <= 0x7A {
                // ASCII lowercase a-z -> A-Z
                i - 0x20
            } else {
                i
            };
            data.push(upper);
        }

        Self { data, valid: true }
    }
}

impl Default for UpcaseTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the checksum for an up-case table.
pub fn compute_upcase_checksum(data: &[u8]) -> u32 {
    let mut checksum: u32 = 0;

    for &byte in data {
        checksum = checksum.rotate_right(1).wrapping_add(byte as u32);
    }

    checksum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_upcase_ascii() {
        let table = UpcaseTable::create_default();

        assert_eq!(table.to_upper(b'a' as u16), b'A' as u16);
        assert_eq!(table.to_upper(b'z' as u16), b'Z' as u16);
        assert_eq!(table.to_upper(b'A' as u16), b'A' as u16);
        assert_eq!(table.to_upper(b'0' as u16), b'0' as u16);
    }

    #[test]
    fn test_names_equal() {
        let table = UpcaseTable::create_default();

        assert!(table.names_equal("hello", "HELLO"));
        assert!(table.names_equal("Test.txt", "test.TXT"));
        assert!(!table.names_equal("hello", "world"));
        assert!(!table.names_equal("hello", "hello!"));
    }
}
