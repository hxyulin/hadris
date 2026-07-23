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
pub const MFT_RECORD_UPCASE: u64 = 10;

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
pub fn decode_record_size(
    value: u8,
    cluster_size: usize,
) -> core::result::Result<usize, NtfsError> {
    let signed = value as i8;
    if signed > 0 {
        (signed as usize)
            .checked_mul(cluster_size)
            .ok_or(NtfsError::InvalidRecordSize)
    } else if signed < 0 {
        1usize
            .checked_shl(signed.unsigned_abs() as u32)
            .ok_or(NtfsError::InvalidRecordSize)
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
pub fn apply_fixups(record: &mut [u8], sector_size: usize) -> Result<()> {
    if record.len() < 8 || sector_size < 2 {
        return Err(NtfsError::InvalidFixup);
    }

    let uso = u16::from_le_bytes([record[4], record[5]]) as usize;
    let uss = u16::from_le_bytes([record[6], record[7]]) as usize;

    let num_fixups = if record.len() < sector_size {
        0
    } else {
        if !record.len().is_multiple_of(sector_size) {
            return Err(NtfsError::InvalidFixup);
        }
        record.len() / sector_size
    };
    if uss != num_fixups + 1 {
        return Err(NtfsError::InvalidFixup);
    }

    let array_end = uss
        .checked_mul(2)
        .and_then(|size| uso.checked_add(size))
        .ok_or(NtfsError::InvalidFixup)?;
    if uso < 8 || array_end > record.len() {
        return Err(NtfsError::InvalidFixup);
    }

    let usn = u16::from_le_bytes([record[uso], record[uso + 1]]);

    for i in 0..num_fixups {
        let sector_end = (i + 1) * sector_size - 2;

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
    finished: bool,
}

impl<'a> AttrIter<'a> {
    /// Create a new attribute iterator from a fixup-applied MFT record.
    pub fn new(record: &'a [u8]) -> Result<Self> {
        if record.len() < 0x1C {
            return Err(NtfsError::InvalidAttribute);
        }
        let first_attr = u16::from_le_bytes([record[0x14], record[0x15]]) as usize;
        let used_size = read_u32_le(record, 0x18) as usize;
        if used_size > record.len()
            || first_attr < 0x18
            || first_attr % 8 != 0
            || first_attr
                .checked_add(size_of::<u32>())
                .is_none_or(|end| end > used_size)
        {
            return Err(NtfsError::InvalidAttribute);
        }
        Ok(Self {
            data: &record[..used_size],
            offset: first_attr,
            finished: false,
        })
    }

    fn invalid(&mut self) -> Option<Result<NtfsAttr<'a>>> {
        self.finished = true;
        Some(Err(NtfsError::InvalidAttribute))
    }
}

impl<'a> Iterator for AttrIter<'a> {
    type Item = Result<NtfsAttr<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.offset + size_of::<u32>() > self.data.len() {
            return self.invalid();
        }

        let attr_type = read_u32_le(self.data, self.offset);
        if attr_type == ATTR_END {
            self.finished = true;
            return None;
        }
        if self.offset + 0x10 > self.data.len() {
            return self.invalid();
        }

        let length = read_u32_le(self.data, self.offset + 4) as usize;
        let Some(attr_end) = self.offset.checked_add(length) else {
            return self.invalid();
        };
        if length < 0x18 || length % 8 != 0 || attr_end > self.data.len() {
            return self.invalid();
        }

        let non_resident = self.data[self.offset + 8];
        if non_resident > 1 {
            return self.invalid();
        }
        let name_length = self.data[self.offset + 9] as usize;
        let name_offset = read_u16_le(self.data, self.offset + 0x0A) as usize;
        let flags = read_u16_le(self.data, self.offset + 0x0C);
        let header_size = if non_resident == 0 { 0x18 } else { 0x40 };
        if length < header_size {
            return self.invalid();
        }

        let name = if name_length > 0 {
            let Some(name_size) = name_length.checked_mul(2) else {
                return self.invalid();
            };
            let Some(abs_start) = self.offset.checked_add(name_offset) else {
                return self.invalid();
            };
            let Some(abs_end) = abs_start.checked_add(name_size) else {
                return self.invalid();
            };
            if name_offset < header_size || abs_end > attr_end {
                return self.invalid();
            }
            Some(&self.data[abs_start..abs_end])
        } else {
            None
        };

        let body = if non_resident == 0 {
            let value_length = read_u32_le(self.data, self.offset + 0x10) as usize;
            let value_offset = read_u16_le(self.data, self.offset + 0x14) as usize;
            let Some(abs_start) = self.offset.checked_add(value_offset) else {
                return self.invalid();
            };
            let Some(abs_end) = abs_start.checked_add(value_length) else {
                return self.invalid();
            };
            if value_offset < header_size || abs_end > attr_end {
                return self.invalid();
            }
            AttrBody::Resident(&self.data[abs_start..abs_end])
        } else {
            let start_vcn = read_u64_le(self.data, self.offset + 0x10);
            let last_vcn = read_u64_le(self.data, self.offset + 0x18);
            let runs_offset = read_u16_le(self.data, self.offset + 0x20) as usize;
            let allocated_size = read_u64_le(self.data, self.offset + 0x28);
            let data_size = read_u64_le(self.data, self.offset + 0x30);
            let initialized_size = read_u64_le(self.data, self.offset + 0x38);

            let Some(abs_runs_start) = self.offset.checked_add(runs_offset) else {
                return self.invalid();
            };
            if runs_offset < header_size || abs_runs_start >= attr_end || start_vcn > last_vcn {
                return self.invalid();
            }

            AttrBody::NonResident {
                start_vcn,
                last_vcn,
                data_runs: &self.data[abs_runs_start..attr_end],
                data_size,
                allocated_size,
                initialized_size,
            }
        };

        self.offset = attr_end;
        Some(Ok(NtfsAttr {
            attr_type,
            flags,
            name,
            body,
        }))
    }
}

// ---------------------------------------------------------------------------
// Data run decoding
// ---------------------------------------------------------------------------

/// A single data run (contiguous range of clusters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    finished: bool,
}

impl<'a> DataRunDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            prev_lcn: 0,
            finished: false,
        }
    }
}

impl Iterator for DataRunDecoder<'_> {
    type Item = Result<DataRun>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.offset >= self.data.len() {
            self.finished = true;
            return Some(Err(NtfsError::InvalidDataRun));
        }

        let header = self.data[self.offset];
        if header == 0 {
            self.finished = true;
            return None;
        }

        let length_size = (header & 0x0F) as usize;
        let offset_size = ((header >> 4) & 0x0F) as usize;

        self.offset += 1;

        if length_size == 0
            || length_size > size_of::<u64>()
            || offset_size > size_of::<i64>()
            || self
                .offset
                .checked_add(length_size + offset_size)
                .is_none_or(|end| end > self.data.len())
        {
            self.finished = true;
            return Some(Err(NtfsError::InvalidDataRun));
        }

        // Run length (unsigned, little-endian)
        let mut length: u64 = 0;
        for i in 0..length_size {
            length |= (self.data[self.offset + i] as u64) << (i * 8);
        }
        self.offset += length_size;
        if length == 0 {
            self.finished = true;
            return Some(Err(NtfsError::InvalidDataRun));
        }

        if offset_size == 0 {
            // Sparse run — no physical clusters
            return Some(Ok(DataRun { lcn: -1, length }));
        }

        // LCN delta (signed, little-endian)
        let mut raw_delta: u64 = 0;
        for i in 0..offset_size {
            raw_delta |= (self.data[self.offset + i] as u64) << (i * 8);
        }
        // Sign-extend if the high bit is set
        if offset_size < size_of::<i64>() && self.data[self.offset + offset_size - 1] & 0x80 != 0 {
            raw_delta |= u64::MAX << (offset_size * 8);
        }
        self.offset += offset_size;

        let Some(lcn) = self.prev_lcn.checked_add(raw_delta as i64) else {
            self.finished = true;
            return Some(Err(NtfsError::InvalidDataRun));
        };
        if lcn < 0 {
            self.finished = true;
            return Some(Err(NtfsError::InvalidDataRun));
        }
        self.prev_lcn = lcn;

        Some(Ok(DataRun { lcn, length }))
    }
}

/// Decode and validate a complete non-resident attribute runlist.
pub fn decode_data_runs(data: &[u8]) -> Result<Vec<DataRun>> {
    DataRunDecoder::new(data).collect()
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

/// Decode raw UTF-16LE bytes into a UTF-8 `String`.
pub fn decode_utf16le(raw: &[u8]) -> Result<String> {
    if raw.len() % 2 != 0 {
        return Err(NtfsError::InvalidFileName);
    }

    let code_units = raw
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
    let mut name = String::with_capacity(raw.len());
    for decoded in char::decode_utf16(code_units) {
        name.push(decoded.map_err(|_| NtfsError::InvalidFileName)?);
    }
    Ok(name)
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

    let first_entry = node_header_offset
        .checked_add(entries_offset)
        .ok_or(NtfsError::InvalidIndexEntry)?;
    let entries_end = node_header_offset
        .checked_add(total_size)
        .ok_or(NtfsError::InvalidIndexEntry)?;

    if entries_offset < 16
        || total_size < entries_offset
        || first_entry > data.len()
        || entries_end > data.len()
    {
        return Err(NtfsError::InvalidIndexEntry);
    }

    let mut results = Vec::new();
    let mut offset = first_entry;

    while offset + 16 <= entries_end {
        let entry_length = read_u16_le(data, offset + 8) as usize;
        let content_length = read_u16_le(data, offset + 10) as usize;
        let flags = read_u32_le(data, offset + 12);

        if entry_length < 16 || offset + entry_length > entries_end {
            return Err(NtfsError::InvalidIndexEntry);
        }

        if flags & INDEX_ENTRY_LAST != 0 {
            break;
        }

        if content_length > 0 {
            let content_start = offset + 16;
            let content_end = content_start + content_length;
            if content_end > offset + entry_length || content_end > data.len() {
                return Err(NtfsError::InvalidIndexEntry);
            }

            let content = &data[content_start..content_end];
            let file_name = parse_file_name(content)?;
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
        } else {
            return Err(NtfsError::InvalidIndexEntry);
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
