use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use super::super::directory::FileFlags;
use super::super::directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef};
use super::super::io::{self, LogicalSector, Read, Seek};

use super::IsoImage;
use super::rrip::{self, RripMetadata};

sync_only! {
use core::ops::DerefMut;
use super::super::io::{IsoCursor, SeekFrom};
use super::rrip::collect_su_entries;
use spin::Mutex;
}

/// A single extent of a file (sector location + byte length).
#[derive(Debug, Clone, Copy)]
pub struct Extent {
    /// The `sector` field.
    pub sector: LogicalSector,
    /// The `length` field.
    pub length: u32,
}

// ── IsoDir ──

#[allow(dead_code)]
/// Represents IsoDir.
pub struct IsoDir<'a, T: Read + Seek> {
    pub(crate) image: &'a IsoImage<T>,
    pub(crate) directory: DirectoryRef,
}

sync_only! {
impl<'a, T: Read + Seek> IsoDir<'a, T> {
    /// Iterate directory entries with automatic RRIP enrichment when detected.
    pub fn entries(&self) -> IsoDirIter<'_, T> {
        let rrip_skip = if self.image.info.susp_info.rrip_detected {
            Some(self.image.info.susp_info.bytes_skipped)
        } else {
            None
        };
        IsoDirIter {
            image: self.image,
            directory: self.directory,
            offset: 0,
            rrip_skip,
            pending_associated: None,
        }
    }

    /// Iterate directory entries as raw `DirectoryRecord` without RRIP processing.
    pub fn raw_entries(&self) -> RawDirIter<'_, T> {
        RawDirIter {
            reader: &self.image.data,
            directory: self.directory,
            offset: 0,
        }
    }
}
} // sync_only!

// ── DirEntry ──

/// A directory entry that may be enriched with RRIP metadata.
///
/// For multi-extent files (using the `NOT_FINAL` flag for files >4 GiB),
/// `additional_extents` contains the extents beyond the first one stored
/// in the primary `record`.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// The `record` field.
    pub record: DirectoryRecord,
    /// The `rrip` field.
    pub rrip: Option<RripMetadata>,
    /// Additional extents for multi-extent files. Empty for single-extent files.
    pub additional_extents: Vec<Extent>,
    /// Associated file record, if one precedes this entry (ASSOCIATED_FILE flag).
    ///
    /// ISO 9660 allows an "associated file" record with the same identifier
    /// to appear before the primary record. This is commonly used for
    /// resource forks or metadata streams.
    pub associated_file: Option<Extent>,
}

impl DirEntry {
    #[inline]
    /// Performs the `name` operation.
    pub fn name(&self) -> &[u8] {
        self.record.name()
    }

    /// Returns the display name: RRIP alternate name if available, else decoded raw name.
    pub fn display_name(&self) -> Cow<'_, str> {
        if let Some(ref rrip) = self.rrip
            && let Some(ref nm) = rrip.alternate_name
        {
            return Cow::Borrowed(nm.as_str());
        }
        String::from_utf8_lossy(self.record.name())
    }

    /// Returns whether this entry matches a path component.
    ///
    /// Rock Ridge alternate names are matched exactly. Plain ISO 9660 names
    /// are matched case-insensitively after removing trailing NUL bytes and a
    /// numeric version suffix such as `;1`.
    pub fn matches_name(&self, name: &str) -> bool {
        if let Some(rrip_name) = self
            .rrip
            .as_ref()
            .and_then(|metadata| metadata.alternate_name.as_deref())
        {
            return rrip_name == name;
        }

        let raw_name = self.record.name();
        if raw_name.len().is_multiple_of(2) {
            let units: Vec<u16> = raw_name
                .chunks_exact(2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect();
            if let Ok(joliet_name) = String::from_utf16(&units) {
                let joliet_name = match joliet_name.rsplit_once(';') {
                    Some((base, version))
                        if !version.is_empty()
                            && version.bytes().all(|byte| byte.is_ascii_digit()) =>
                    {
                        base
                    }
                    _ => &joliet_name,
                };
                if joliet_name == name {
                    return true;
                }
            }
        }

        let display_name: String = self
            .display_name()
            .chars()
            .filter(|character| *character != '\0')
            .collect();
        let display_name = match display_name.rsplit_once(';') {
            Some((base, version))
                if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) =>
            {
                base
            }
            _ => &display_name,
        };
        display_name.eq_ignore_ascii_case(name)
    }

    #[inline]
    /// Performs the `header` operation.
    pub fn header(&self) -> &DirectoryRecordHeader {
        self.record.header()
    }

    /// Returns true if this entry represents a directory.
    /// CL-aware: a child link means the entry points to a directory.
    #[inline]
    pub fn is_directory(&self) -> bool {
        if let Some(ref rrip) = self.rrip
            && rrip.child_link.is_some()
        {
            return true;
        }
        self.record.is_directory()
    }

    #[inline]
    /// Performs the `is_special` operation.
    pub fn is_special(&self) -> bool {
        self.record.is_special()
    }

    #[inline]
    /// Performs the `is_file` operation.
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }

    /// Returns the record size (directory record length), NOT the file data size.
    /// For the file's total data size, use [`total_size`](Self::total_size).
    #[inline]
    pub fn size(&self) -> usize {
        self.record.size()
    }

    /// Returns the total file data size across all extents.
    ///
    /// For single-extent files this is the same as `header().data_len.read()`.
    /// For multi-extent files this sums all extent lengths.
    pub fn total_size(&self) -> u64 {
        let first = self.record.header().data_len.read() as u64;
        let rest: u64 = self
            .additional_extents
            .iter()
            .map(|e| e.length as u64)
            .sum();
        first + rest
    }

    /// Returns true if this is a multi-extent file.
    pub fn is_multi_extent(&self) -> bool {
        !self.additional_extents.is_empty()
    }

    /// Returns an iterator over all extents of this file.
    ///
    /// The first extent comes from the primary record; additional extents
    /// follow in order.
    pub fn extents(&self) -> impl Iterator<Item = Extent> + '_ {
        let header = self.record.header();
        let first = Extent {
            sector: LogicalSector(header.extent.read() as usize),
            length: header.data_len.read(),
        };
        core::iter::once(first).chain(self.additional_extents.iter().copied())
    }

    /// Returns true if this entry has an associated file (e.g., resource fork).
    pub fn has_associated_file(&self) -> bool {
        self.associated_file.is_some()
    }

    /// Returns the length of the extended attribute record in logical sectors,
    /// or `None` if no extended attributes are present.
    ///
    /// When present (`extended_attr_record > 0`), the XA record is located
    /// at the start of the file's extent (before the file data) and occupies
    /// this many logical sectors.
    pub fn extended_attr_len(&self) -> Option<u8> {
        let len = self.record.header().extended_attr_record;
        if len > 0 { Some(len) } else { None }
    }

    #[inline]
    /// Performs the `system_use` operation.
    pub fn system_use(&self) -> &[u8] {
        self.record.system_use()
    }
}

io_transform! {
impl DirEntry {
    /// Get a `DirectoryRef` for navigating into this directory.
    ///
    /// For CL entries, this follows the child link to the relocated directory.
    /// For regular directories, it uses the ISO 9660 extent/size.
    pub async fn as_dir_ref<DATA: Read + Seek>(
        &self,
        image: &IsoImage<DATA>,
    ) -> io::Result<DirectoryRef> {
        if let Some(ref rrip) = self.rrip
            && let Some(cl_sector) = rrip.child_link
        {
            return rrip::read_dir_size(image, LogicalSector(cl_sector as usize)).await;
        }
        self.record
            .as_dir_ref()
            .map_err(|_| io::Error::other("not a directory"))
    }
}
} // io_transform!

io_transform! {
impl<T: Read + Seek> IsoDir<'_, T> {
    /// Reads all entries in this directory.
    ///
    /// Unlike the synchronous `entries` iterator, this collection-oriented operation is
    /// available with both synchronous and asynchronous I/O.
    pub async fn read_entries(&self) -> io::Result<Vec<DirEntry>> {
        const SECTOR_SIZE: usize = 2048;
        let mut bytes = alloc::vec![0_u8; self.directory.size];
        self.image
            .read_bytes_at(self.directory.extent.0 as u64 * SECTOR_SIZE as u64, &mut bytes)
            .await?;
        let mut cursor = hadris_io::Cursor::new(bytes.as_slice());
        let mut offset = 0_usize;
        let mut entries = Vec::new();
        let mut associated_file = None;

        while offset < bytes.len() {
            if bytes[offset] == 0 {
                offset = (offset / SECTOR_SIZE + 1) * SECTOR_SIZE;
                continue;
            }
            cursor.seek(io::SeekFrom::Start(offset as u64)).await?;
            let record = DirectoryRecord::parse(&mut cursor).await?;
            offset += record.size();
            let flags = FileFlags::from_bits_retain(record.header().flags);
            if flags.contains(FileFlags::ASSOCIATED_FILE) {
                associated_file = Some(Extent {
                    sector: LogicalSector(record.header().extent.read() as usize),
                    length: record.header().data_len.read(),
                });
                continue;
            }

            let mut additional_extents = Vec::new();
            if flags.contains(FileFlags::NOT_FINAL) {
                const MAX_EXTENTS: usize = 4096;
                while additional_extents.len() < MAX_EXTENTS && offset < bytes.len() {
                    if bytes[offset] == 0 {
                        offset = (offset / SECTOR_SIZE + 1) * SECTOR_SIZE;
                        continue;
                    }
                    cursor.seek(io::SeekFrom::Start(offset as u64)).await?;
                    let continuation = DirectoryRecord::parse(&mut cursor).await?;
                    offset += continuation.size();
                    let header = continuation.header();
                    additional_extents.push(Extent {
                        sector: LogicalSector(header.extent.read() as usize),
                        length: header.data_len.read(),
                    });
                    if !FileFlags::from_bits_retain(header.flags)
                        .contains(FileFlags::NOT_FINAL)
                    {
                        break;
                    }
                }
            }

            let rrip = if self.image.info.susp_info.rrip_detected {
                let fields = rrip::collect_su_entries(
                    &record,
                    self.image,
                    self.image.info.susp_info.bytes_skipped,
                )
                .await?;
                let metadata = RripMetadata::from_fields(&fields);
                if metadata.is_relocated {
                    continue;
                }
                Some(metadata)
            } else {
                None
            };
            entries.push(DirEntry {
                record,
                rrip,
                additional_extents,
                associated_file: associated_file.take(),
            });
        }
        Ok(entries)
    }

    /// Finds an entry in this directory by its displayed name.
    pub async fn find(&self, name: &str) -> io::Result<Option<DirEntry>> {
        Ok(self
            .read_entries()
            .await?
            .into_iter()
            .find(|entry| entry.matches_name(name)))
    }
}
} // io_transform!

// ── IsoDirIter (RRIP-aware) ──

sync_only! {

/// Iterator over directory entries with automatic RRIP enrichment.
///
/// When RRIP is detected (`rrip_skip` is `Some`), each entry is enriched with
/// parsed RRIP metadata and RE-marked entries are skipped.
/// When RRIP is not detected, entries have `rrip: None` (zero overhead).
pub struct IsoDirIter<'a, T: Read + Seek> {
    image: &'a IsoImage<T>,
    directory: DirectoryRef,
    offset: usize,
    rrip_skip: Option<u8>,
    /// Pending associated file from a previous record with ASSOCIATED_FILE flag.
    pending_associated: Option<Extent>,
}

impl<T: Read + Seek> IsoDirIter<'_, T> {
    /// Performs the `offset` operation.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// After reading a record with `NOT_FINAL` set, consume subsequent
    /// continuation records (same file identifier) until the final extent.
    /// Returns the additional extents (not including the first/primary record).
    fn collect_additional_extents(&mut self) -> io::Result<Vec<Extent>> {
        let mut extents = Vec::new();
        // Depth limit to prevent infinite loops on malformed images
        const MAX_EXTENTS: usize = 4096;

        loop {
            if extents.len() >= MAX_EXTENTS {
                break;
            }

            let record = match self.next_raw_record() {
                Some(Ok(r)) => r,
                Some(Err(e)) => return Err(e),
                None => break,
            };

            let header = record.header();
            extents.push(Extent {
                sector: LogicalSector(header.extent.read() as usize),
                length: header.data_len.read(),
            });

            // If this record does NOT have NOT_FINAL, it's the last extent
            if !FileFlags::from_bits_retain(header.flags).contains(FileFlags::NOT_FINAL) {
                break;
            }
        }

        Ok(extents)
    }

    /// Read the next raw DirectoryRecord from the directory data.
    fn next_raw_record(&mut self) -> Option<io::Result<DirectoryRecord>> {
        use super::super::io::try_io_result_option;
        let reader = &self.image.data;
        let mut reader = reader.lock();

        const SECTOR_SIZE: usize = 2048;

        loop {
            if self.offset >= self.directory.size {
                return None;
            }

            try_io_result_option!(reader.seek(SeekFrom::Start(
                (self.directory.extent.0 as u64) * SECTOR_SIZE as u64 + (self.offset as u64),
            )));

            let mut len_byte = [0u8; 1];
            try_io_result_option!(reader.read_exact(&mut len_byte));

            if len_byte[0] == 0 {
                let current_sector_offset = self.offset % SECTOR_SIZE;
                if current_sector_offset == 0 {
                    return None;
                }
                let bytes_to_skip = SECTOR_SIZE - current_sector_offset;
                self.offset += bytes_to_skip;
                continue;
            }

            try_io_result_option!(reader.seek(SeekFrom::Start(
                (self.directory.extent.0 as u64) * SECTOR_SIZE as u64 + (self.offset as u64),
            )));

            let record = try_io_result_option!(DirectoryRecord::parse(reader.deref_mut()));
            self.offset += record.size();

            return Some(Ok(record));
        }
    }
}

impl<T: Read + Seek> Iterator for IsoDirIter<'_, T> {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let record = match self.next_raw_record()? {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };

            let flags = FileFlags::from_bits_retain(record.header().flags);

            // If this is an associated file record, save it and continue
            // to the next record (the primary file entry).
            if flags.contains(FileFlags::ASSOCIATED_FILE) {
                let header = record.header();
                self.pending_associated = Some(Extent {
                    sector: LogicalSector(header.extent.read() as usize),
                    length: header.data_len.read(),
                });
                continue;
            }

            // Check for multi-extent: if NOT_FINAL is set, collect additional extents
            let additional_extents = if flags.contains(FileFlags::NOT_FINAL) {
                match self.collect_additional_extents() {
                    Ok(extents) => extents,
                    Err(e) => return Some(Err(e)),
                }
            } else {
                Vec::new()
            };

            let associated_file = self.pending_associated.take();

            if let Some(bytes_to_skip) = self.rrip_skip {
                // RRIP mode: enrich with metadata, skip RE entries
                let fields = match collect_su_entries(&record, self.image, bytes_to_skip) {
                    Ok(f) => f,
                    Err(e) => return Some(Err(e)),
                };
                let rrip = RripMetadata::from_fields(&fields);

                // Skip RE-marked entries (relocated directory placeholders)
                if rrip.is_relocated {
                    continue;
                }

                return Some(Ok(DirEntry {
                    record,
                    rrip: Some(rrip),
                    additional_extents,
                    associated_file,
                }));
            } else {
                // No RRIP: return plain entry
                return Some(Ok(DirEntry {
                    record,
                    rrip: None,
                    additional_extents,
                    associated_file,
                }));
            }
        }
    }
}

// ── RawDirIter ──

/// Iterator over raw directory records without RRIP processing.
pub struct RawDirIter<'a, T: Read + Seek> {
    pub(crate) reader: &'a Mutex<IsoCursor<T>>,
    pub(crate) directory: DirectoryRef,
    pub(crate) offset: usize,
}

impl<T: Read + Seek> RawDirIter<'_, T> {
    /// Performs the `offset` operation.
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl<T: Read + Seek> Iterator for RawDirIter<'_, T> {
    type Item = io::Result<DirectoryRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        use super::super::io::try_io_result_option;
        let mut reader = self.reader.lock();

        const SECTOR_SIZE: usize = 2048;

        loop {
            if self.offset >= self.directory.size {
                return None;
            }

            try_io_result_option!(reader.seek(SeekFrom::Start(
                (self.directory.extent.0 as u64) * SECTOR_SIZE as u64 + (self.offset as u64),
            )));

            let mut len_byte = [0u8; 1];
            try_io_result_option!(reader.read_exact(&mut len_byte));

            if len_byte[0] == 0 {
                let current_sector_offset = self.offset % SECTOR_SIZE;
                if current_sector_offset == 0 {
                    return None;
                }
                let bytes_to_skip = SECTOR_SIZE - current_sector_offset;
                self.offset += bytes_to_skip;
                continue;
            }

            try_io_result_option!(reader.seek(SeekFrom::Start(
                (self.directory.extent.0 as u64) * SECTOR_SIZE as u64 + (self.offset as u64),
            )));

            let record = try_io_result_option!(DirectoryRecord::parse(reader.deref_mut()));
            self.offset += record.size();

            return Some(Ok(record));
        }
    }
}

} // sync_only!
