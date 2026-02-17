use core::ops::DerefMut;

use alloc::borrow::Cow;
use alloc::string::String;

use crate::{
    directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef},
    io::{self, IsoCursor, LogicalSector, Read, Seek, SeekFrom},
};
use hadris_io::ErrorKind;
use spin::Mutex;

use super::IsoImage;
use super::rrip::{self, RripMetadata, collect_su_entries};

// ── IsoDir ──

pub struct IsoDir<'a, T: Read + Seek> {
    pub(crate) image: &'a IsoImage<T>,
    pub(crate) directory: DirectoryRef,
}

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

// ── DirEntry ──

/// A directory entry that may be enriched with RRIP metadata.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub record: DirectoryRecord,
    pub rrip: Option<RripMetadata>,
}

impl DirEntry {
    #[inline]
    pub fn name(&self) -> &[u8] {
        self.record.name()
    }

    /// Returns the display name: RRIP alternate name if available, else decoded raw name.
    pub fn display_name(&self) -> Cow<'_, str> {
        if let Some(ref rrip) = self.rrip {
            if let Some(ref nm) = rrip.alternate_name {
                return Cow::Borrowed(nm.as_str());
            }
        }
        String::from_utf8_lossy(self.record.name())
    }

    #[inline]
    pub fn header(&self) -> &DirectoryRecordHeader {
        self.record.header()
    }

    /// Returns true if this entry represents a directory.
    /// CL-aware: a child link means the entry points to a directory.
    #[inline]
    pub fn is_directory(&self) -> bool {
        if let Some(ref rrip) = self.rrip {
            if rrip.child_link.is_some() {
                return true;
            }
        }
        self.record.is_directory()
    }

    #[inline]
    pub fn is_special(&self) -> bool {
        self.record.is_special()
    }

    #[inline]
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.record.size()
    }

    #[inline]
    pub fn system_use(&self) -> &[u8] {
        self.record.system_use()
    }

    /// Get a `DirectoryRef` for navigating into this directory.
    ///
    /// For CL entries, this follows the child link to the relocated directory.
    /// For regular directories, it uses the ISO 9660 extent/size.
    pub fn as_dir_ref<DATA: Read + Seek>(
        &self,
        image: &IsoImage<DATA>,
    ) -> io::Result<DirectoryRef> {
        if let Some(ref rrip) = self.rrip {
            if let Some(cl_sector) = rrip.child_link {
                return rrip::read_dir_size(image, LogicalSector(cl_sector as usize));
            }
        }
        self.record
            .as_dir_ref()
            .map_err(|_| io::Error::new(ErrorKind::Other, "not a directory"))
    }
}

// ── IsoDirIter (RRIP-aware) ──

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
}

impl<T: Read + Seek> IsoDirIter<'_, T> {
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Read the next raw DirectoryRecord from the directory data.
    fn next_raw_record(&mut self) -> Option<io::Result<DirectoryRecord>> {
        use hadris_io::try_io_result_option;
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
                }));
            } else {
                // No RRIP: return plain entry
                return Some(Ok(DirEntry {
                    record,
                    rrip: None,
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
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl<T: Read + Seek> Iterator for RawDirIter<'_, T> {
    type Item = io::Result<DirectoryRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        use hadris_io::try_io_result_option;
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
