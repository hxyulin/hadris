use crate::{
    boot::{BaseBootCatalog, BootSectionHeaderEntry},
    io::LogicalSector,
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
        todo!()
    }

    pub fn default_entry(&self) -> BootEntryInfo {
        let entry = &self.catalog.default_entry;
        assert!(entry.is_valid());
        BootEntryInfo {
            bootable: entry.boot_indicator == 0x88,
            meadia_type: entry.boot_media_type,
            load_segment: entry.load_segment.get(),
            sector_count: entry.sector_count.get(),
            load_rba: entry.load_rba.get(),
        }
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
    data: &'data Mutex<DATA>,
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
        loop {
            try_io!(data.read_exact(bytemuck::bytes_of_mut(&mut header)));

            match header.header_type {
                0x00 => panic!("should have stopped reading!"),
                0x90 => {
                    self.has_more = true;
                    return Some(Ok(header));
                }
                0x91 => {
                    self.has_more = false;
                    return Some(Ok(header));
                }
                // We skip past entries
                _ => continue,
            }
        }
    }
}
