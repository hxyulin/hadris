use crate::{
    boot::{BaseBootCatalog, BootSectionHeaderEntry},
    io::{IsoCursor, LogicalSector},
    read::IsoImage,
};
use bytemuck::Zeroable;
use hadris_common::types::endian::Endian;
use hadris_io::{self as io, Read, Seek, SeekFrom};
use spin::Mutex;

#[derive(Debug, Clone)]
pub struct BootInfo {
    /// The Base Boot Catalog
    /// This is used to provide basic information about booting
    pub(crate) catalog: BaseBootCatalog,
    /// The Start Sector of the Boot Catalog
    /// This is used to construct an iterator for querying further sections
    pub(crate) catalog_ptr: LogicalSector,
}

impl BootInfo {
    pub fn sections<'a, R: Read + Seek>(
        &'a self,
        image: &'a IsoImage<R>,
    ) -> BootSectionIter<'a, R> {
        // Boot catalog structure:
        // - Validation Entry (32 bytes) at sector start
        // - Default/Initial Entry (32 bytes)
        // - Section Headers and Entries follow (32 bytes each)
        // We start reading section headers after the first 64 bytes
        let catalog_byte_offset = (self.catalog_ptr.0 as u64) * 2048 + 64;
        BootSectionIter {
            data: &image.data,
            current_seek: catalog_byte_offset,
            has_more: true,
        }
    }

    /// Returns the default boot entry information.
    ///
    /// Returns `None` if the default entry is not valid (malformed boot catalog).
    pub fn default_entry(&self) -> Option<BootEntryInfo> {
        let entry = &self.catalog.default_entry;
        if !entry.is_valid() {
            return None;
        }
        Some(BootEntryInfo {
            bootable: entry.boot_indicator == 0x88,
            meadia_type: entry.boot_media_type,
            load_segment: entry.load_segment.get(),
            sector_count: entry.sector_count.get(),
            load_rba: entry.load_rba.get(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct BootEntryInfo {
    pub bootable: bool,
    pub meadia_type: u8,
    pub load_segment: u16,
    pub sector_count: u16,
    pub load_rba: u32,
}

pub struct BootSectionIter<'data, DATA: Read + Seek> {
    data: &'data Mutex<IsoCursor<DATA>>,
    current_seek: u64,
    has_more: bool,
}

impl<DATA: Read + Seek> Iterator for BootSectionIter<'_, DATA> {
    type Item = io::Result<BootSectionHeaderEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut data = self.data.lock();
        if !self.has_more {
            return None;
        }
        use hadris_io::try_io_result_option as try_io;
        try_io!(data.seek(SeekFrom::Start(self.current_seek)));
        let mut header = BootSectionHeaderEntry::zeroed();

        // Limit iterations to prevent infinite loop on malformed data
        const MAX_ENTRIES_PER_SECTOR: usize = 64; // 2048 / 32 = 64 entries max per sector
        for _ in 0..MAX_ENTRIES_PER_SECTOR {
            try_io!(data.read_exact(bytemuck::bytes_of_mut(&mut header)));
            self.current_seek += 32; // Each entry is 32 bytes

            match header.header_type {
                0x00 => {
                    // Terminator entry - no more sections
                    self.has_more = false;
                    return None;
                }
                0x90 => {
                    // Section header with more sections following
                    self.has_more = true;
                    return Some(Ok(header));
                }
                0x91 => {
                    // Final section header
                    self.has_more = false;
                    return Some(Ok(header));
                }
                // Skip past boot entries (not section headers)
                _ => continue,
            }
        }

        // Exceeded max iterations - malformed boot catalog
        self.has_more = false;
        None
    }
}
