use super::super::boot::{BaseBootCatalog, BootSectionHeaderEntry};
use super::super::io::LogicalSector;
use hadris_common::types::endian::Endian;

use super::super::io::{self, IsoCursor, Read, Seek, SeekFrom};
use super::IsoImage;
use bytemuck::Zeroable;
use spin::Mutex;

#[derive(Debug, Clone)]
/// Represents BootInfo.
pub struct BootInfo {
    /// The Base Boot Catalog
    /// This is used to provide basic information about booting
    pub(crate) catalog: BaseBootCatalog,
    /// The Start Sector of the Boot Catalog
    /// This is used to construct an iterator for querying further sections
    #[allow(dead_code)]
    pub(crate) catalog_ptr: LogicalSector,
}

impl BootInfo {
    /// Performs the `sections` operation.
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
            media_type: entry.boot_media_type,
            load_segment: entry.load_segment.get(),
            sector_count: entry.sector_count.get(),
            load_rba: entry.load_rba.get(),
        })
    }
}

#[derive(Debug, Clone)]
/// Represents BootEntryInfo.
pub struct BootEntryInfo {
    /// Whether the catalog marks this entry bootable.
    pub bootable: bool,
    /// El-Torito media/emulation type byte.
    pub media_type: u8,
    /// Segment where the boot image is loaded.
    pub load_segment: u16,
    /// Number of virtual sectors to load.
    pub sector_count: u16,
    /// Starting logical block of the boot image.
    pub load_rba: u32,
}

io_transform! {

/// Represents BootSectionIter.
pub struct BootSectionIter<'data, DATA: Read + Seek> {
    data: &'data Mutex<IsoCursor<DATA>>,
    current_seek: u64,
    has_more: bool,
}

impl<DATA: Read + Seek> BootSectionIter<'_, DATA> {
    /// Reads the next boot section header.
    pub async fn next_section(&mut self) -> io::Result<Option<BootSectionHeaderEntry>> {
        let mut data = self.data.lock();
        if !self.has_more {
            return Ok(None);
        }
        data
            .seek(SeekFrom::Start(self.current_seek))
            .await
            .map_err(io::Error::erase)?;
        let mut header = BootSectionHeaderEntry::zeroed();

        // Limit iterations to prevent infinite loop on malformed data
        const MAX_ENTRIES_PER_SECTOR: usize = 64; // 2048 / 32 = 64 entries max per sector
        for _ in 0..MAX_ENTRIES_PER_SECTOR {
            data.read_exact(bytemuck::bytes_of_mut(&mut header)).await?;
            self.current_seek += 32; // Each entry is 32 bytes

            match header.header_type {
                0x00 => {
                    // Terminator entry - no more sections
                    self.has_more = false;
                    return Ok(None);
                }
                0x90 => {
                    // Section header with more sections following
                    self.has_more = true;
                    return Ok(Some(header));
                }
                0x91 => {
                    // Final section header
                    self.has_more = false;
                    return Ok(Some(header));
                }
                // Skip past boot entries (not section headers)
                _ => continue,
            }
        }

        // Exceeded max iterations - malformed boot catalog
        self.has_more = false;
        Ok(None)
    }
}

} // io_transform!

sync_only! {
impl<DATA: Read + Seek> Iterator for BootSectionIter<'_, DATA> {
    type Item = io::Result<BootSectionHeaderEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_section().transpose()
    }
}
} // sync_only!
