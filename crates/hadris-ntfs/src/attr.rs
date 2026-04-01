//! NTFS attribute parsing, fixup processing, and data run decoding.
//!
//! All functions in this module operate on in-memory byte buffers — no I/O
//! is performed, making them usable from both sync and async code paths.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{NtfsError, Result};

// ---------------------------------------------------------------------------
// Well-known attribute type codes
// ---------------------------------------------------------------------------

pub const ATTR_STANDARD_INFORMATION: u32 = 0x10;
pub const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
pub const ATTR_FILE_NAME: u32 = 0x30;
pub const ATTR_OBJECT_ID: u32 = 0x40;
pub const ATTR_SECURITY_DESCRIPTOR: u32 = 0x50;
pub const ATTR_VOLUME_NAME: u32 = 0x60;
pub const ATTR_VOLUME_INFORMATION: u32 = 0x70;
pub const ATTR_DATA: u32 = 0x80;
pub const ATTR_INDEX_ROOT: u32 = 0x90;
pub const ATTR_INDEX_ALLOCATION: u32 = 0xA0;
pub const ATTR_BITMAP: u32 = 0xB0;
pub const ATTR_END: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// MFT record flags (at header offset 0x16)
// ---------------------------------------------------------------------------

pub const MFT_RECORD_IN_USE: u16 = 0x0001;
pub const MFT_RECORD_IS_DIRECTORY: u16 = 0x0002;

// ---------------------------------------------------------------------------
// Index entry flags
// ---------------------------------------------------------------------------

pub const INDEX_ENTRY_SUBNODE: u32 = 0x0001;
pub const INDEX_ENTRY_LAST: u32 = 0x0002;

// ---------------------------------------------------------------------------
// File name namespace values ($FILE_NAME offset 0x41)
// ---------------------------------------------------------------------------

pub const FILE_NAME_POSIX: u8 = 0;
pub const FILE_NAME_WIN32: u8 = 1;
pub const FILE_NAME_DOS: u8 = 2;
pub const FILE_NAME_WIN32_AND_DOS: u8 = 3;

// ---------------------------------------------------------------------------
// Well-known MFT record numbers
// ---------------------------------------------------------------------------

pub const MFT_RECORD_MFT: u64 = 0;
pub const MFT_RECORD_ROOT_DIR: u64 = 5;

// ---------------------------------------------------------------------------
// Attribute flags
// ---------------------------------------------------------------------------

pub const ATTR_FLAG_COMPRESSED: u16 = 0x0001;
pub const ATTR_FLAG_ENCRYPTED: u16 = 0x4000;
pub const ATTR_FLAG_SPARSE: u16 = 0x8000;

// ---------------------------------------------------------------------------
// $I30 index name (UTF-16LE for "$I30")
// ---------------------------------------------------------------------------

pub const I30_NAME: &[u8] = &[0x24, 0x00, 0x49, 0x00, 0x33, 0x00, 0x30, 0x00];

/// Check whether an attribute name matches the `$I30` directory index.
pub fn is_i30_name(name: Option<&[u8]>) -> bool {
    name.is_some_and(|n| n == I30_NAME)
}

// ---------------------------------------------------------------------------
// Decode the clusters-per-record field from the boot sector
// ---------------------------------------------------------------------------

/// Decode the MFT/index record size from the boot sector encoding.
///
/// Positive (i8): value × cluster_size.
/// Negative (i8): 2^|value| bytes.
pub fn decode_record_size(value: u8, cluster_size: usize) -> core::result::Result<usize, NtfsError> {
    let signed = value as i8;
    if signed > 0 {
        Ok(signed as usize * cluster_size)
    } else if signed < 0 {
        Ok(1usize << ((-signed) as u32))
    } else {
        Err(NtfsError::InvalidRecordSize)
    }
}

// ---------------------------------------------------------------------------
// Fixup (update sequence) processing
// ---------------------------------------------------------------------------

/// Apply the NTFS multi-sector fixup to a record buffer.
///
/// Works for both `FILE` (MFT) and `INDX` (index allocation) records.
/// The update sequence offset and size are read from bytes 4–7 of the
/// record header, which is the same position in both record types.
pub fn apply_fixups(record: &mut [u8]) -> Result<()> {
    if record.len() < 8 {
        return Err(NtfsError::InvalidFixup);
    }

    let uso = u16::from_le_bytes([record[4], record[5]]) as usize;
    let uss = u16::from_le_bytes([record[6], record[7]]) as usize;

    if uss <= 1 {
        return Ok(());
    }

    let array_end = uso + uss * 2;
    if uso < 8 || array_end > record.len() {
        return Err(NtfsError::InvalidFixup);
    }

    let usn = u16::from_le_bytes([record[uso], record[uso + 1]]);
    let num_fixups = uss - 1;

    // Derive the protection block size from the record and fixup count.
    // Typically 512 bytes (the NTFS multi-sector protection granularity).
    let block_size = record.len() / num_fixups;
    if block_size < 2 {
        return Err(NtfsError::InvalidFixup);
    }

    for i in 0..num_fixups {
        let sector_end = (i + 1) * block_size - 2;
        if sector_end + 1 >= record.len() {
            break;
        }

        let on_disk = u16::from_le_bytes([record[sector_end], record[sector_end + 1]]);
        if on_disk != usn {
            return Err(NtfsError::FixupMismatch {
                expected: usn,
                found: on_disk,
            });
        }

        let saved_offset = uso + 2 + i * 2;
        record[sector_end] = record[saved_offset];
        record[sector_end + 1] = record[saved_offset + 1];
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Attribute iteration
// ---------------------------------------------------------------------------

/// A parsed NTFS attribute (zero-copy over the record buffer).
#[derive(Debug)]
pub struct NtfsAttr<'a> {
    pub attr_type: u32,
    pub flags: u16,
    /// Raw UTF-16LE name bytes, if the attribute is named.
    pub name: Option<&'a [u8]>,
    pub body: AttrBody<'a>,
}

/// The body of an NTFS attribute.
#[derive(Debug)]
pub enum AttrBody<'a> {
    /// Resident: value data is inline in the MFT record.
    Resident(&'a [u8]),
    /// Non-resident: data is stored in clusters described by data runs.
    NonResident {
        start_vcn: u64,
        last_vcn: u64,
        /// Raw data-run bytes (decode with [`DataRunDecoder`]).
        data_runs: &'a [u8],
        data_size: u64,
        allocated_size: u64,
        initialized_size: u64,
    },
}

/// Iterator over the attributes inside an MFT record buffer.
pub struct AttrIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> AttrIter<'a> {
    /// Create a new attribute iterator from a fixup-applied MFT record.
    pub fn new(record: &'a [u8]) -> Result<Self> {
        if record.len() < 0x16 {
            return Err(NtfsError::InvalidAttribute);
        }
        let first_attr = u16::from_le_bytes([record[0x14], record[0x15]]) as usize;
        if first_attr >= record.len() {
            return Err(NtfsError::InvalidAttribute);
        }
        Ok(Self {
            data: record,
            offset: first_attr,
        })
    }

    /// Advance to the next attribute. Returns `None` at the end marker.
    pub fn next(&mut self) -> Option<NtfsAttr<'a>> {
        loop {
            if self.offset + 8 > self.data.len() {
                return None;
            }

            let attr_type = read_u32_le(self.data, self.offset);
            if attr_type == ATTR_END {
                return None;
            }

            let length = read_u32_le(self.data, self.offset + 4) as usize;
            if length < 16 || self.offset + length > self.data.len() {
                return None;
            }

            let non_resident = self.data[self.offset + 8];
            let name_length = self.data[self.offset + 9] as usize;
            let name_offset = read_u16_le(self.data, self.offset + 0x0A) as usize;
            let flags = read_u16_le(self.data, self.offset + 0x0C);

            let name = if name_length > 0 {
                let abs_start = self.offset + name_offset;
                let abs_end = abs_start + name_length * 2;
                if abs_end <= self.offset + length {
                    Some(&self.data[abs_start..abs_end])
                } else {
                    None
                }
            } else {
                None
            };

            let body = if non_resident == 0 {
                // Resident
                if self.offset + 0x18 > self.data.len() {
                    self.offset += length;
                    continue;
                }
                let value_length = read_u32_le(self.data, self.offset + 0x10) as usize;
                let value_offset = read_u16_le(self.data, self.offset + 0x14) as usize;
                let abs_start = self.offset + value_offset;
                let abs_end = abs_start + value_length;
                if abs_end > self.offset + length {
                    self.offset += length;
                    continue;
                }
                AttrBody::Resident(&self.data[abs_start..abs_end])
            } else {
                // Non-resident
                if self.offset + 0x40 > self.data.len() {
                    self.offset += length;
                    continue;
                }
                let start_vcn = read_u64_le(self.data, self.offset + 0x10);
                let last_vcn = read_u64_le(self.data, self.offset + 0x18);
                let runs_offset = read_u16_le(self.data, self.offset + 0x20) as usize;
                let allocated_size = read_u64_le(self.data, self.offset + 0x28);
                let data_size = read_u64_le(self.data, self.offset + 0x30);
                let initialized_size = read_u64_le(self.data, self.offset + 0x38);

                let abs_runs_start = self.offset + runs_offset;
                let abs_runs_end = self.offset + length;
                let data_runs = if abs_runs_start < abs_runs_end {
                    &self.data[abs_runs_start..abs_runs_end]
                } else {
                    &[]
                };

                AttrBody::NonResident {
                    start_vcn,
                    last_vcn,
                    data_runs,
                    data_size,
                    allocated_size,
                    initialized_size,
                }
            };

            self.offset += length;

            return Some(NtfsAttr {
                attr_type,
                flags,
                name,
                body,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Data run decoding
// ---------------------------------------------------------------------------

/// A single data run (contiguous range of clusters).
#[derive(Debug, Clone, Copy)]
pub struct DataRun {
    /// Logical Cluster Number. Negative means sparse (no physical storage).
    pub lcn: i64,
    /// Number of contiguous clusters in this run.
    pub length: u64,
}

/// Decodes the packed data-run bytes from a non-resident attribute.
///
/// Each entry encodes a (length, LCN-delta) pair. The LCN deltas are
/// cumulative — each is relative to the previous run's LCN.
pub struct DataRunDecoder<'a> {
    data: &'a [u8],
    offset: usize,
    prev_lcn: i64,
}

impl<'a> DataRunDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            prev_lcn: 0,
        }
    }
}

impl Iterator for DataRunDecoder<'_> {
    type Item = DataRun;

    fn next(&mut self) -> Option<DataRun> {
        if self.offset >= self.data.len() {
            return None;
        }

        let header = self.data[self.offset];
        if header == 0 {
            return None;
        }

        let length_size = (header & 0x0F) as usize;
        let offset_size = ((header >> 4) & 0x0F) as usize;

        self.offset += 1;

        if length_size == 0 || self.offset + length_size + offset_size > self.data.len() {
            return None;
        }

        // Run length (unsigned, little-endian)
        let mut length: u64 = 0;
        for i in 0..length_size {
            length |= (self.data[self.offset + i] as u64) << (i * 8);
        }
        self.offset += length_size;

        if offset_size == 0 {
            // Sparse run — no physical clusters
            return Some(DataRun { lcn: -1, length });
        }

        // LCN delta (signed, little-endian)
        let mut delta: i64 = 0;
        for i in 0..offset_size {
            delta |= (self.data[self.offset + i] as i64) << (i * 8);
        }
        // Sign-extend if the high bit is set
        if self.data[self.offset + offset_size - 1] & 0x80 != 0 {
            for i in offset_size..8 {
                delta |= (0xFF_i64) << (i * 8);
            }
        }
        self.offset += offset_size;

        self.prev_lcn += delta;

        Some(DataRun {
            lcn: self.prev_lcn,
            length,
        })
    }
}

// ---------------------------------------------------------------------------
// $FILE_NAME parsing
// ---------------------------------------------------------------------------

/// Parsed information from a `$FILE_NAME` attribute value.
#[derive(Debug, Clone)]
pub struct FileNameInfo {
    /// Parent directory MFT reference (lower 48 bits = record number).
    pub parent_ref: u64,
    /// Decoded UTF-8 filename.
    pub name: String,
    /// File name namespace (POSIX / Win32 / DOS / Win32+DOS).
    pub namespace: u8,
    /// Win32 file attribute flags from the $FILE_NAME.
    pub flags: u32,
    /// Data size as recorded in $FILE_NAME (may be stale).
    pub data_size: u64,
    /// Allocated size as recorded in $FILE_NAME.
    pub allocated_size: u64,
}

/// Parse a `$FILE_NAME` attribute value (the bytes after the attribute header).
pub fn parse_file_name(data: &[u8]) -> Result<FileNameInfo> {
    if data.len() < 0x42 {
        return Err(NtfsError::InvalidFileName);
    }

    let parent_ref = read_u64_le(data, 0x00);
    let allocated_size = read_u64_le(data, 0x28);
    let data_size = read_u64_le(data, 0x30);
    let flags = read_u32_le(data, 0x38);
    let name_length = data[0x40] as usize;
    let namespace = data[0x41];

    let name_bytes_end = 0x42 + name_length * 2;
    if name_bytes_end > data.len() {
        return Err(NtfsError::InvalidFileName);
    }
    let name_bytes = &data[0x42..name_bytes_end];
    let name = decode_utf16le(name_bytes)?;

    Ok(FileNameInfo {
        parent_ref,
        name,
        namespace,
        flags,
        data_size,
        allocated_size,
    })
}

/// Decode raw UTF-16LE bytes into a UTF-8 `String` using the `ucs2` crate.
pub fn decode_utf16le(raw: &[u8]) -> Result<String> {
    let char_count = raw.len() / 2;
    if char_count == 0 {
        return Ok(String::new());
    }

    // Convert LE bytes to native u16 values
    let mut u16_buf: Vec<u16> = Vec::with_capacity(char_count);
    for chunk in raw.chunks_exact(2) {
        u16_buf.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }

    // UCS-2 can expand to at most 3 UTF-8 bytes per character
    let mut utf8_buf = alloc::vec![0u8; char_count * 3];
    let len = ucs2::decode(&u16_buf, &mut utf8_buf).map_err(|_| NtfsError::InvalidFileName)?;
    utf8_buf.truncate(len);
    String::from_utf8(utf8_buf).map_err(|_| NtfsError::InvalidFileName)
}

// ---------------------------------------------------------------------------
// Index entry parsing helpers
// ---------------------------------------------------------------------------

/// Parsed information from a single index entry.
#[derive(Debug, Clone)]
pub struct IndexEntryInfo {
    /// MFT record number (lower 48 bits of the file reference).
    pub mft_index: u64,
    /// Sequence number (upper 16 bits of the file reference).
    pub mft_seq: u16,
    /// Parsed $FILE_NAME from the entry content.
    pub file_name: FileNameInfo,
}

/// Parse all non-sentinel index entries from an index node.
///
/// `data` is the full buffer (either INDEX_ROOT value or INDX record).
/// `node_header_offset` is the byte offset of the index node header
/// within `data` (0x10 for INDEX_ROOT values, 0x18 for INDX records).
pub fn parse_index_entries(data: &[u8], node_header_offset: usize) -> Result<Vec<IndexEntryInfo>> {
    if node_header_offset + 16 > data.len() {
        return Err(NtfsError::InvalidIndexEntry);
    }

    let entries_offset = read_u32_le(data, node_header_offset) as usize;
    let total_size = read_u32_le(data, node_header_offset + 4) as usize;

    let first_entry = node_header_offset + entries_offset;
    let entries_end = node_header_offset + total_size;

    if first_entry > data.len() || entries_end > data.len() {
        return Err(NtfsError::InvalidIndexEntry);
    }

    let mut results = Vec::new();
    let mut offset = first_entry;

    while offset + 16 <= entries_end {
        let entry_length = read_u16_le(data, offset + 8) as usize;
        let content_length = read_u16_le(data, offset + 10) as usize;
        let flags = read_u32_le(data, offset + 12);

        if entry_length < 16 || offset + entry_length > entries_end {
            break;
        }

        if flags & INDEX_ENTRY_LAST != 0 {
            break;
        }

        if content_length > 0 {
            let content_start = offset + 16;
            let content_end = content_start + content_length;
            if content_end <= offset + entry_length && content_end <= data.len() {
                let content = &data[content_start..content_end];
                if let Ok(file_name) = parse_file_name(content) {
                    // Skip DOS-only names to avoid duplicate entries
                    if file_name.namespace != FILE_NAME_DOS {
                        let mft_ref_raw = read_u64_le(data, offset);
                        let mft_index = mft_ref_raw & 0x0000_FFFF_FFFF_FFFF;
                        let mft_seq = (mft_ref_raw >> 48) as u16;

                        results.push(IndexEntryInfo {
                            mft_index,
                            mft_seq,
                            file_name,
                        });
                    }
                }
            }
        }

        offset += entry_length;
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Little-endian read helpers
// ---------------------------------------------------------------------------

#[inline]
fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

#[inline]
fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[inline]
fn read_u64_le(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}
