//! Parsing of MFT records, attributes, mapping pairs, `$FILE_NAME` values and
//! index nodes. Nothing here does I/O, so both modes share it.

use hadris_fs::DateTime;

use crate::error::Detail;
use crate::raw;

/// The largest MFT or index record the reader handles.
pub(crate) const MAX_RECORD: usize = 4096;

/// The stride of the update sequence array, which is 512 bytes whatever the
/// sector size.
pub(crate) const FIXUP_STRIDE: usize = 512;

/// Reads a little-endian `u16` at `at`; the caller checked the bounds.
pub(crate) fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

/// Reads a little-endian `u32` at `at`; the caller checked the bounds.
pub(crate) fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Reads a little-endian `u64` at `at`; the caller checked the bounds.
pub(crate) fn u64_at(buf: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&buf[at..at + 8]);
    u64::from_le_bytes(bytes)
}

/// Decodes a clusters-per-record field of the boot sector: a positive value
/// counts clusters, a negative one is a power of two in bytes.
///
/// @hadris-spec NTFS:Boot-Sector
/// @hadris-compliance partial
/// @hadris-tests record::tests::record_sizes_decode_both_encodings
/// @hadris-fuzz ntfs_read
/// @hadris-note Sizes above 4096 bytes are refused as unsupported.
pub(crate) fn record_size(value: u8, cluster_size: u64) -> Result<u64, Detail> {
    let signed = value as i8;
    let size = if signed > 0 {
        (signed as u64).checked_mul(cluster_size)
    } else if signed < 0 {
        1u64.checked_shl(u32::from(signed.unsigned_abs()))
    } else {
        None
    };
    match size {
        Some(size) if size.is_power_of_two() && size >= FIXUP_STRIDE as u64 => Ok(size),
        _ => Err(Detail::Geometry),
    }
}

/// Applies the update sequence array of a `FILE` or `INDX` record in place.
///
/// @hadris-spec NTFS:Update-Sequence-Array
/// @hadris-compliance unknown
/// @hadris-tests record::tests::fixups_restore_each_stride, record::tests::fixups_reject_a_short_count, record::tests::fixups_use_512_byte_strides_on_4k_records
/// @hadris-fuzz ntfs_read
pub(crate) fn apply_fixups(record: &mut [u8]) -> Result<(), Detail> {
    if record.len() < 8 || record.len() % FIXUP_STRIDE != 0 {
        return Err(Detail::UpdateSequence);
    }
    let offset = usize::from(u16_at(record, 4));
    let count = usize::from(u16_at(record, 6));
    let strides = record.len() / FIXUP_STRIDE;
    if count != strides + 1 || offset < 8 || offset % 2 != 0 || offset + count * 2 > record.len() {
        return Err(Detail::UpdateSequence);
    }
    let usn = u16_at(record, offset);
    for i in 0..strides {
        let end = (i + 1) * FIXUP_STRIDE - 2;
        if u16_at(record, end) != usn {
            return Err(Detail::UpdateSequence);
        }
        let saved = offset + 2 + i * 2;
        record[end] = record[saved];
        record[end + 1] = record[saved + 1];
    }
    Ok(())
}

/// The header fields of an MFT record the reader uses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordHeader {
    pub(crate) sequence: u16,
    pub(crate) flags: u16,
}

impl RecordHeader {
    pub(crate) fn is_dir(&self) -> bool {
        self.flags & raw::MFT_RECORD_IS_DIRECTORY != 0
    }
}

/// Checks the magic of an MFT record, applies its fixups and reads its
/// header. A record that is not in use is refused.
pub(crate) fn file_record(record: &mut [u8]) -> Result<RecordHeader, Detail> {
    if record.len() < 0x30 || &record[..4] != b"FILE" {
        return Err(Detail::Record);
    }
    apply_fixups(record)?;
    let header = RecordHeader {
        sequence: u16_at(record, 0x10),
        flags: u16_at(record, 0x16),
    };
    if header.flags & raw::MFT_RECORD_IN_USE == 0 {
        return Err(Detail::Record);
    }
    Ok(header)
}

/// A non-resident attribute's header fields.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NonResident<'a> {
    pub(crate) start_vcn: u64,
    pub(crate) runs: &'a [u8],
    pub(crate) data_size: u64,
    pub(crate) initialized_size: u64,
}

/// Where an attribute's value is.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Body<'a> {
    Resident(&'a [u8]),
    NonResident(NonResident<'a>),
}

/// One attribute of an MFT record, borrowed from the record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Attr<'a> {
    pub(crate) kind: u32,
    pub(crate) flags: u16,
    /// The attribute's instance number, unique within its record.
    pub(crate) id: u16,
    /// The UTF-16LE name; empty for an unnamed attribute.
    pub(crate) name: &'a [u8],
    pub(crate) body: Body<'a>,
}

impl Attr<'_> {
    /// The logical length of the value.
    pub(crate) fn len(&self) -> u64 {
        match self.body {
            Body::Resident(value) => value.len() as u64,
            Body::NonResident(nr) => nr.data_size,
        }
    }
}

/// Iterates over the attributes of a fixed-up MFT record. It stops at the
/// end marker and yields at most one error.
///
/// @hadris-spec NTFS:Attribute-Record
/// @hadris-compliance partial
/// @hadris-tests record::tests::attributes_are_bounded_by_the_used_size, record::tests::attributes_stop_at_the_end_marker, record::tests::attributes_need_an_end_marker
/// @hadris-fuzz ntfs_read
/// @hadris-note Resident and non-resident headers are validated within one record; the attributes of extension records are reached through `$ATTRIBUTE_LIST`.
pub(crate) struct Attrs<'a> {
    data: &'a [u8],
    offset: usize,
    done: bool,
}

impl<'a> Attrs<'a> {
    pub(crate) fn new(record: &'a [u8]) -> Result<Self, Detail> {
        if record.len() < 0x1C {
            return Err(Detail::Attribute);
        }
        let first = usize::from(u16_at(record, 0x14));
        let used = u32_at(record, 0x18) as usize;
        if used > record.len() || first < 0x18 || first % 8 != 0 || first + 4 > used {
            return Err(Detail::Attribute);
        }
        Ok(Self {
            data: &record[..used],
            offset: first,
            done: false,
        })
    }

    fn fail(&mut self) -> Option<Result<Attr<'a>, Detail>> {
        self.done = true;
        Some(Err(Detail::Attribute))
    }
}

impl<'a> Iterator for Attrs<'a> {
    type Item = Result<Attr<'a>, Detail>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let data = self.data;
        let at = self.offset;
        if at + 4 > data.len() {
            return self.fail();
        }
        let kind = u32_at(data, at);
        if kind == raw::ATTR_END {
            self.done = true;
            return None;
        }
        if at + 0x18 > data.len() {
            return self.fail();
        }
        let length = u32_at(data, at + 4) as usize;
        let Some(end) = at.checked_add(length) else {
            return self.fail();
        };
        if length < 0x18 || length % 8 != 0 || end > data.len() {
            return self.fail();
        }
        let non_resident = data[at + 8];
        let name_len = usize::from(data[at + 9]) * 2;
        let name_offset = usize::from(u16_at(data, at + 0x0A));
        let flags = u16_at(data, at + 0x0C);
        let id = u16_at(data, at + 0x0E);
        let header = match non_resident {
            0 => 0x18,
            1 => 0x40,
            _ => return self.fail(),
        };
        if length < header {
            return self.fail();
        }
        let name = if name_len == 0 {
            &data[at..at]
        } else {
            if name_offset < header || name_offset + name_len > length {
                return self.fail();
            }
            &data[at + name_offset..at + name_offset + name_len]
        };
        let body = if non_resident == 0 {
            let value_len = u32_at(data, at + 0x10) as usize;
            let value_offset = usize::from(u16_at(data, at + 0x14));
            if value_offset < header || value_offset.saturating_add(value_len) > length {
                return self.fail();
            }
            Body::Resident(&data[at + value_offset..at + value_offset + value_len])
        } else {
            let start_vcn = u64_at(data, at + 0x10);
            let last_vcn = u64_at(data, at + 0x18);
            let runs_offset = usize::from(u16_at(data, at + 0x20));
            let data_size = u64_at(data, at + 0x30);
            let initialized_size = u64_at(data, at + 0x38);
            if runs_offset < header || runs_offset >= length || start_vcn > last_vcn.wrapping_add(1)
            {
                return self.fail();
            }
            Body::NonResident(NonResident {
                start_vcn,
                runs: &data[at + runs_offset..end],
                data_size,
                initialized_size,
            })
        };
        self.offset = end;
        Some(Ok(Attr {
            kind,
            flags,
            id,
            name,
            body,
        }))
    }
}

/// Finds the first attribute of `kind` whose name is `name` (UTF-16LE,
/// empty for unnamed).
pub(crate) fn find_attr<'a>(
    record: &'a [u8],
    kind: u32,
    name: &[u8],
) -> Result<Option<Attr<'a>>, Detail> {
    for attr in Attrs::new(record)? {
        let attr = attr?;
        if attr.kind == kind && attr.name == name {
            return Ok(Some(attr));
        }
    }
    Ok(None)
}

/// Finds the instance of an attribute that starts at `start_vcn`; a
/// resident attribute starts at 0.
pub(crate) fn find_instance<'a>(
    record: &'a [u8],
    kind: u32,
    name: &[u8],
    start_vcn: u64,
) -> Result<Option<Attr<'a>>, Detail> {
    for attr in Attrs::new(record)? {
        let attr = attr?;
        let start = match attr.body {
            Body::Resident(_) => 0,
            Body::NonResident(nr) => nr.start_vcn,
        };
        if attr.kind == kind && attr.name == name && start == start_vcn {
            return Ok(Some(attr));
        }
    }
    Ok(None)
}

/// Finds the attribute of `kind` with instance number `id`.
pub(crate) fn find_id(record: &[u8], kind: u32, id: u16) -> Result<Option<Attr<'_>>, Detail> {
    for attr in Attrs::new(record)? {
        let attr = attr?;
        if attr.kind == kind && attr.id == id {
            return Ok(Some(attr));
        }
    }
    Ok(None)
}

/// The largest `$ATTRIBUTE_LIST` entry: a header and a 255-unit name.
pub(crate) const MAX_LIST_ENTRY: usize = 0x20 + 510;

/// One entry of an `$ATTRIBUTE_LIST`: where an attribute instance lives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListEntry<'a> {
    pub(crate) kind: u32,
    /// UTF-16LE name; empty for an unnamed attribute.
    pub(crate) name: &'a [u8],
    pub(crate) start_vcn: u64,
    pub(crate) reference: u64,
    /// The instance number of the attribute in its record.
    pub(crate) id: u16,
    pub(crate) len: usize,
}

/// Parses the `$ATTRIBUTE_LIST` entry at the start of `buf`.
///
/// @hadris-spec NTFS:Attribute-List
/// @hadris-compliance partial
/// @hadris-tests record::tests::list_entries_are_bounded, crafted::attribute_lists_join_extension_records, crafted::attribute_list_gaps_fail, crafted::index_roots_in_extension_records_are_followed, read::attribute_lists_on_a_real_volume
/// @hadris-fuzz ntfs_read
/// @hadris-note Streams, names and indexes are followed into extension records; `$MFT` may have at most 32 extents.
pub(crate) fn list_entry(buf: &[u8]) -> Result<ListEntry<'_>, Detail> {
    if buf.len() < 0x1A {
        return Err(Detail::AttributeList);
    }
    let len = usize::from(u16_at(buf, 4));
    let name_len = usize::from(buf[6]) * 2;
    let name_offset = usize::from(buf[7]);
    if len < 0x1A || len % 8 != 0 || len > buf.len() || len > MAX_LIST_ENTRY {
        return Err(Detail::AttributeList);
    }
    let name = if name_len == 0 {
        &buf[..0]
    } else {
        if name_offset < 0x1A || name_offset + name_len > len {
            return Err(Detail::AttributeList);
        }
        &buf[name_offset..name_offset + name_len]
    };
    Ok(ListEntry {
        kind: u32_at(buf, 0),
        name,
        start_vcn: u64_at(buf, 8),
        reference: u64_at(buf, 0x10),
        id: u16_at(buf, 0x18),
        len,
    })
}

/// A run of clusters; `lcn` is `None` for a sparse run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) lcn: Option<u64>,
    pub(crate) len: u64,
}

/// Decodes mapping pairs. Each run's LCN is relative to the previous one.
/// It yields at most one error.
///
/// @hadris-spec NTFS:Mapping-Pairs
/// @hadris-compliance unknown
/// @hadris-tests record::tests::runs_decode_relative_and_sparse_extents, record::tests::runs_reject_malformed_encodings
/// @hadris-fuzz ntfs_read
pub(crate) struct Runs<'a> {
    data: &'a [u8],
    offset: usize,
    lcn: i64,
    done: bool,
}

impl<'a> Runs<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            lcn: 0,
            done: false,
        }
    }

    fn fail(&mut self) -> Option<Result<Run, Detail>> {
        self.done = true;
        Some(Err(Detail::DataRun))
    }
}

impl Iterator for Runs<'_> {
    type Item = Result<Run, Detail>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let Some(&header) = self.data.get(self.offset) else {
            return self.fail();
        };
        if header == 0 {
            self.done = true;
            return None;
        }
        let len_size = usize::from(header & 0x0F);
        let lcn_size = usize::from(header >> 4);
        let start = self.offset + 1;
        if len_size == 0
            || len_size > 8
            || lcn_size > 8
            || start + len_size + lcn_size > self.data.len()
        {
            return self.fail();
        }
        let mut len = 0u64;
        for (i, byte) in self.data[start..start + len_size].iter().enumerate() {
            len |= u64::from(*byte) << (i * 8);
        }
        if len == 0 {
            return self.fail();
        }
        self.offset = start + len_size + lcn_size;
        if lcn_size == 0 {
            return Some(Ok(Run { lcn: None, len }));
        }
        let bytes = &self.data[start + len_size..start + len_size + lcn_size];
        let mut delta = 0u64;
        for (i, byte) in bytes.iter().enumerate() {
            delta |= u64::from(*byte) << (i * 8);
        }
        if lcn_size < 8 && bytes[lcn_size - 1] & 0x80 != 0 {
            delta |= u64::MAX << (lcn_size * 8);
        }
        match self.lcn.checked_add(delta as i64) {
            Some(lcn) if lcn >= 0 => {
                self.lcn = lcn;
                Some(Ok(Run {
                    lcn: Some(lcn as u64),
                    len,
                }))
            }
            _ => self.fail(),
        }
    }
}

/// A parsed `$FILE_NAME` value, borrowed from its buffer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileName<'a> {
    pub(crate) parent: u64,
    pub(crate) namespace: u8,
    /// UTF-16LE name.
    pub(crate) name: &'a [u8],
}

/// Parses a `$FILE_NAME` value.
///
/// @hadris-spec NTFS:File-Name
/// @hadris-compliance partial
/// @hadris-tests record::tests::file_names_parse_and_bound_the_name
/// @hadris-fuzz ntfs_read
/// @hadris-note Parses the parent reference, namespace and full UTF-16 name; the flags, the copies of times and sizes and the reparse tag are not used.
pub(crate) fn file_name(value: &[u8]) -> Result<FileName<'_>, Detail> {
    if value.len() < 0x42 {
        return Err(Detail::FileName);
    }
    let len = usize::from(value[0x40]) * 2;
    let name = value.get(0x42..0x42 + len).ok_or(Detail::FileName)?;
    if name.is_empty() {
        return Err(Detail::FileName);
    }
    Ok(FileName {
        parent: u64_at(value, 0),
        namespace: value[0x41],
        name,
    })
}

/// The entry area of an index node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexNode {
    pub(crate) first: usize,
    pub(crate) end: usize,
}

/// Reads the node header at `header` of an `$INDEX_ROOT` value (0x10) or
/// an `INDX` record (0x18).
pub(crate) fn index_node(buf: &[u8], header: usize) -> Result<IndexNode, Detail> {
    if header.checked_add(16).is_none_or(|end| end > buf.len()) {
        return Err(Detail::Index);
    }
    let first = u32_at(buf, header) as usize;
    let total = u32_at(buf, header + 4) as usize;
    let first = header.checked_add(first).ok_or(Detail::Index)?;
    let end = header.checked_add(total).ok_or(Detail::Index)?;
    if first < header + 16 || first > end || end > buf.len() {
        return Err(Detail::Index);
    }
    Ok(IndexNode { first, end })
}

/// One entry of an index node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexEntry<'a> {
    /// The file reference, or `None` for the last entry of the node.
    pub(crate) entry: Option<(u64, FileName<'a>)>,
    pub(crate) next: usize,
}

/// Parses the index entry at `at` of `node`.
///
/// @hadris-spec NTFS:Index-Entry
/// @hadris-compliance partial
/// @hadris-tests record::tests::index_entries_are_bounded_by_the_node, read::large_directory_lists_every_entry
/// @hadris-fuzz ntfs_read
/// @hadris-note Every node is enumerated; child-node pointers are not followed for a keyed descent.
pub(crate) fn index_entry<'a>(
    buf: &'a [u8],
    node: IndexNode,
    at: usize,
) -> Result<IndexEntry<'a>, Detail> {
    if at < node.first || at.checked_add(16).is_none_or(|end| end > node.end) {
        return Err(Detail::Index);
    }
    let len = usize::from(u16_at(buf, at + 8));
    let content = usize::from(u16_at(buf, at + 10));
    let flags = u32_at(buf, at + 12);
    let next = at + len;
    if len < 16 || len % 8 != 0 || next > node.end {
        return Err(Detail::Index);
    }
    if flags & raw::INDEX_ENTRY_LAST != 0 {
        return Ok(IndexEntry { entry: None, next });
    }
    if content == 0 || 16 + content > len {
        return Err(Detail::Index);
    }
    let name = file_name(&buf[at + 16..at + 16 + content])?;
    Ok(IndexEntry {
        entry: Some((u64_at(buf, at), name)),
        next,
    })
}

/// The record number of a file reference.
pub(crate) const fn reference_record(reference: u64) -> u64 {
    reference & 0x0000_FFFF_FFFF_FFFF
}

/// The sequence number of a file reference.
pub(crate) const fn reference_sequence(reference: u64) -> u16 {
    (reference >> 48) as u16
}

/// The characters of UTF-16LE bytes, with unpaired surrogates replaced by
/// U+FFFD.
pub(crate) fn utf16_chars(bytes: &[u8]) -> impl Iterator<Item = char> + '_ {
    char::decode_utf16(
        bytes
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]])),
    )
    .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
}

/// Writes UTF-16LE bytes into `out` as UTF-8 and returns the length, or
/// `None` when it does not fit.
pub(crate) fn utf16_to_utf8(bytes: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut len = 0;
    for c in utf16_chars(bytes) {
        let end = len + c.len_utf8();
        c.encode_utf8(out.get_mut(len..end)?);
        len = end;
    }
    Some(len)
}

/// Converts an NTFS time, 100 ns units since 1601-01-01 UTC, or `None` for
/// zero.
pub(crate) fn nt_time(value: u64) -> Option<DateTime> {
    const UNITS: u64 = 10_000_000;
    const EPOCH_DELTA: i64 = 11_644_473_600;
    if value == 0 {
        return None;
    }
    let seconds = (value / UNITS) as i64 - EPOCH_DELTA;
    let nanos = (value % UNITS) as u32 * 100;
    DateTime::new(seconds, nanos)
        .ok()?
        .with_utc_offset_minutes(Some(0))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;
    use std::vec;
    use std::vec::Vec;

    #[test]
    fn record_sizes_decode_both_encodings() {
        assert_eq!(record_size((-10i8) as u8, 4096), Ok(1024));
        assert_eq!(record_size(1, 4096), Ok(4096));
        assert_eq!(record_size(0, 4096), Err(Detail::Geometry));
        assert_eq!(record_size((-128i8) as u8, 4096), Err(Detail::Geometry));
        assert_eq!(record_size((-8i8) as u8, 4096), Err(Detail::Geometry));
        assert_eq!(record_size(127, u64::MAX), Err(Detail::Geometry));
    }

    fn sealed(len: usize) -> Vec<u8> {
        let strides = len / FIXUP_STRIDE;
        let mut record = vec![0u8; len];
        record[4..6].copy_from_slice(&0x30u16.to_le_bytes());
        record[6..8].copy_from_slice(&(strides as u16 + 1).to_le_bytes());
        record[0x30..0x32].copy_from_slice(&0xA55Au16.to_le_bytes());
        for i in 0..strides {
            let saved = 0x32 + i * 2;
            record[saved..saved + 2].copy_from_slice(&(0x1100u16 + i as u16).to_le_bytes());
            let end = (i + 1) * FIXUP_STRIDE - 2;
            record[end..end + 2].copy_from_slice(&0xA55Au16.to_le_bytes());
        }
        record
    }

    #[test]
    fn fixups_restore_each_stride() {
        let mut record = sealed(1024);
        apply_fixups(&mut record).unwrap();
        assert_eq!(&record[510..512], &0x1100u16.to_le_bytes());
        assert_eq!(&record[1022..1024], &0x1101u16.to_le_bytes());
    }

    #[test]
    fn fixups_use_512_byte_strides_on_4k_records() {
        let mut record = sealed(4096);
        apply_fixups(&mut record).unwrap();
        for i in 0..8 {
            let end = (i + 1) * 512 - 2;
            assert_eq!(&record[end..end + 2], &(0x1100u16 + i as u16).to_le_bytes());
        }
    }

    #[test]
    fn fixups_reject_a_short_count() {
        let mut record = sealed(1024);
        record[6..8].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(apply_fixups(&mut record), Err(Detail::UpdateSequence));
        let mut record = sealed(1024);
        record[510] ^= 0xFF;
        assert_eq!(apply_fixups(&mut record), Err(Detail::UpdateSequence));
        assert_eq!(apply_fixups(&mut [0u8; 100]), Err(Detail::UpdateSequence));
    }

    #[test]
    fn runs_decode_relative_and_sparse_extents() {
        let runs: Result<Vec<_>, _> =
            Runs::new(&[0x11, 0x03, 0x20, 0x01, 0x02, 0x11, 0x01, 0xFE, 0x00]).collect();
        assert_eq!(
            runs.unwrap(),
            [
                Run {
                    lcn: Some(0x20),
                    len: 3
                },
                Run { lcn: None, len: 2 },
                Run {
                    lcn: Some(0x1E),
                    len: 1
                },
            ]
        );
    }

    #[test]
    fn runs_reject_malformed_encodings() {
        for invalid in [
            &[0x19, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0][..],
            &[0x11, 0, 1, 0][..],
            &[0x11, 1, 1][..],
            &[0x11, 1, 0xFF, 0][..],
            &[0x11, 1, 1][..],
            &[][..],
        ] {
            let mut runs = Runs::new(invalid);
            assert!(runs.any(|run| run == Err(Detail::DataRun)), "{invalid:?}");
            assert!(runs.next().is_none());
        }
    }

    fn record_with(attrs: &[u8], used: u32) -> Vec<u8> {
        let mut record = vec![0u8; 1024];
        record[0x14..0x16].copy_from_slice(&0x30u16.to_le_bytes());
        record[0x18..0x1C].copy_from_slice(&used.to_le_bytes());
        record[0x30..0x30 + attrs.len()].copy_from_slice(attrs);
        record
    }

    fn resident_data() -> Vec<u8> {
        let mut attr = vec![0u8; 0x18];
        attr[0..4].copy_from_slice(&raw::ATTR_DATA.to_le_bytes());
        attr[4..8].copy_from_slice(&0x18u32.to_le_bytes());
        attr[0x14..0x16].copy_from_slice(&0x18u16.to_le_bytes());
        attr
    }

    #[test]
    fn attributes_are_bounded_by_the_used_size() {
        let mut attr = resident_data();
        attr[4..8].copy_from_slice(&0x20u32.to_le_bytes());
        let record = record_with(&attr, 0x48);
        let mut attrs = Attrs::new(&record).unwrap();
        assert_eq!(attrs.next().unwrap().err(), Some(Detail::Attribute));
        assert!(attrs.next().is_none());
        assert_eq!(
            Attrs::new(&record_with(&[], 2048)).err(),
            Some(Detail::Attribute)
        );
    }

    #[test]
    fn attributes_stop_at_the_end_marker() {
        let mut attrs = resident_data();
        attrs.extend(raw::ATTR_END.to_le_bytes());
        let record = record_with(&attrs, 0x50);
        let mut iter = Attrs::new(&record).unwrap();
        assert_eq!(iter.next().unwrap().unwrap().kind, raw::ATTR_DATA);
        assert!(iter.next().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn attributes_need_an_end_marker() {
        let record = record_with(&resident_data(), 0x4C);
        let mut iter = Attrs::new(&record).unwrap();
        assert!(iter.next().unwrap().is_ok());
        assert_eq!(iter.next().unwrap().err(), Some(Detail::Attribute));
        assert!(iter.next().is_none());
    }

    fn file_name_value(name: &str) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut value = vec![0u8; 0x42 + units.len()];
        value[0..8].copy_from_slice(&5u64.to_le_bytes());
        value[0x40] = (units.len() / 2) as u8;
        value[0x41] = raw::FILE_NAME_WIN32;
        value[0x42..].copy_from_slice(&units);
        value
    }

    #[test]
    fn file_names_parse_and_bound_the_name() {
        let value = file_name_value("r\u{E9}sum\u{E9}");
        let parsed = file_name(&value).unwrap();
        assert_eq!(parsed.parent, 5);
        assert_eq!(
            utf16_chars(parsed.name).collect::<std::string::String>(),
            "r\u{E9}sum\u{E9}"
        );
        assert_eq!(
            file_name(&value[..value.len() - 1]).err(),
            Some(Detail::FileName)
        );
        assert_eq!(file_name(&[0u8; 0x41]).err(), Some(Detail::FileName));
    }

    #[test]
    fn names_decode_surrogate_pairs_and_replace_unpaired_ones() {
        let mut out = [0u8; 16];
        let len = utf16_to_utf8(&[0x3E, 0xD8, 0x80, 0xDD], &mut out).unwrap();
        assert_eq!(&out[..len], "\u{1F980}".as_bytes());
        let len = utf16_to_utf8(&[0x3E, 0xD8, 0x41, 0x00], &mut out).unwrap();
        assert_eq!(&out[..len], "\u{FFFD}A".as_bytes());
        let len = utf16_to_utf8(&[0x00, 0xDC], &mut out).unwrap();
        assert_eq!(&out[..len], "\u{FFFD}".as_bytes());
        assert_eq!(
            utf16_to_utf8(&[0x41, 0x00, 0x42, 0x00], &mut out[..1]),
            None
        );
        assert_eq!(utf16_chars(&[0x41]).count(), 0);
    }

    #[test]
    fn index_entries_are_bounded_by_the_node() {
        let mut buf = vec![0u8; 64];
        buf[0..4].copy_from_slice(&16u32.to_le_bytes());
        buf[4..8].copy_from_slice(&48u32.to_le_bytes());
        assert_eq!(index_node(&buf, usize::MAX).err(), Some(Detail::Index));
        assert_eq!(index_node(&buf, usize::MAX - 8).err(), Some(Detail::Index));
        let node = index_node(&buf, 0).unwrap();
        buf[16 + 8..16 + 10].copy_from_slice(&16u16.to_le_bytes());
        buf[16 + 12..16 + 16].copy_from_slice(&raw::INDEX_ENTRY_LAST.to_le_bytes());
        let last = index_entry(&buf, node, 16).unwrap();
        assert!(last.entry.is_none());
        assert_eq!(last.next, 32);
        buf[16 + 8..16 + 10].copy_from_slice(&64u16.to_le_bytes());
        assert_eq!(index_entry(&buf, node, 16).err(), Some(Detail::Index));
        assert_eq!(index_entry(&buf, node, 8).err(), Some(Detail::Index));
        assert_eq!(
            index_entry(&buf, node, usize::MAX).err(),
            Some(Detail::Index)
        );
    }

    #[test]
    fn list_entries_are_bounded() {
        let mut entry = [0u8; 0x28];
        entry[0..4].copy_from_slice(&raw::ATTR_DATA.to_le_bytes());
        entry[4..6].copy_from_slice(&0x28u16.to_le_bytes());
        entry[6] = 2;
        entry[7] = 0x1A;
        entry[8..16].copy_from_slice(&7u64.to_le_bytes());
        entry[0x10..0x18].copy_from_slice(&20u64.to_le_bytes());
        entry[0x1A..0x1E].copy_from_slice(&[b'a', 0, b'b', 0]);
        let parsed = list_entry(&entry).unwrap();
        assert_eq!(
            (parsed.kind, parsed.start_vcn, parsed.reference, parsed.len),
            (raw::ATTR_DATA, 7, 20, 0x28)
        );
        assert_eq!(parsed.name, &[b'a', 0, b'b', 0]);
        assert_eq!(
            list_entry(&entry[..0x20]).err(),
            Some(Detail::AttributeList)
        );
        entry[6] = 20;
        assert_eq!(list_entry(&entry).err(), Some(Detail::AttributeList));
        entry[6] = 2;
        entry[4..6].copy_from_slice(&0x21u16.to_le_bytes());
        assert_eq!(list_entry(&entry).err(), Some(Detail::AttributeList));
        entry[4..6].copy_from_slice(&0x10u16.to_le_bytes());
        assert_eq!(list_entry(&entry).err(), Some(Detail::AttributeList));
    }

    #[test]
    fn nt_times_convert_to_unix_time() {
        assert_eq!(nt_time(0), None);
        let unix = nt_time(116_444_736_000_000_000).unwrap();
        assert_eq!(unix.unix_seconds(), 0);
        assert_eq!(unix.utc_offset_minutes(), Some(0));
        let later = nt_time(116_444_736_000_000_000 + 12_345_678).unwrap();
        assert_eq!(
            (later.unix_seconds(), later.nanoseconds()),
            (1, 234_567_800)
        );
        let _ = nt_time(u64::MAX);
    }
}
