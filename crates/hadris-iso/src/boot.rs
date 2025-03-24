//! The El Torito boot specification
//!
//! This is used for booting from CDs and DVDs

use std::{
    fmt::Debug,
    io::{Read, Write},
};

use crate::{
    types::{Endian, LittleEndian, U16, U32}, BootOptions, BootRecordVolumeDescriptor, FileData, FileInput, FormatOptions, PathTableRef, ReadWriteSeek
};

/// Types for El Torito boot catalogue
/// The boot catalogue consists of a series of boot catalogue entries:
/// First, the validation entry
/// Next, the initial/default entry
/// Section headers,
/// Section entries,
/// Section entry extensions

#[derive(Debug, Clone)]
pub struct BootCatalogue {
    validation: BootValidationEntry,
    default_entry: BootSectionEntry,
    sections: Vec<(BootSectionHeaderEntry, Vec<BootSectionEntry>)>,
}

impl Default for BootCatalogue {
    fn default() -> Self {
        Self::new(EmulationType::NoEmulation, 0, 0, 0)
    }
}

impl BootCatalogue {
    pub fn new(
        media_type: EmulationType,
        load_segment: u16,
        sector_count: u16,
        load_rba: u32,
    ) -> Self {
        Self {
            validation: BootValidationEntry::new(),
            default_entry: BootSectionEntry::new(media_type, load_segment, sector_count, load_rba),
            sections: Vec::new(),
        }
    }

    pub fn set_default_entry(&mut self, entry: BootSectionEntry) {
        self.default_entry = entry;
    }

    pub fn add_section(&mut self, platform_id: PlatformId, entries: Vec<BootSectionEntry>) {
        if let Some((header, _entry)) = self.sections.last_mut() {
            // No longer the last section
            header.header_type = 0x90;
        }

        let header = BootSectionHeaderEntry {
            header_type: 0x91,
            platform_id: platform_id.to_u8(),
            section_count: U16::new(1),
            section_ident: [0; 28],
        };

        self.sections.push((header, entries));
    }

    /// Parse the boot catalogue from the given reader,
    /// expects the reader to seek to the start of the catalogue
    pub fn parse<T: Read>(reader: &mut T) -> Result<Self, std::io::Error> {
        let validation = BootValidationEntry::parse(reader)?;
        if !validation.is_valid() {
            panic!("Invalid boot catalogue: Validation entry is invalid");
        }
        let default_entry = BootSectionEntry::parse(reader)?;
        if !default_entry.is_valid() {
            panic!("Invalid boot catalogue: Default boot entry is invalid");
        }

        let mut sections = Vec::new();
        let mut buffer = [0u8; 32];
        let mut has_more = false;
        let mut header = None;
        let mut entries = Vec::new();
        loop {
            reader.read_exact(&mut buffer)?;
            match buffer[0] {
                0x00 if !has_more => break,
                0x90 => {
                    has_more = true;
                    if let Some(header) = header.take() {
                        sections.push((header, entries));
                        entries = Vec::new();
                    }
                    header = Some(bytemuck::cast(buffer));
                }
                0x91 => {
                    has_more = false;
                    if let Some(header) = header.take() {
                        sections.push((header, entries));
                        entries = Vec::new();
                    }
                    header = Some(bytemuck::cast(buffer));
                }
                id => {
                    if header.is_none() {
                        panic!("Boot catalogue: expected header, got: {:#x}", id);
                    }
                    entries.push(bytemuck::cast(buffer));
                }
            }
        }

        assert!(!has_more, "Boot catalogue: expected more sections");
        if let Some(header) = header {
            sections.push((header, entries));
        }

        Ok(Self {
            validation,
            default_entry,
            sections,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        writer.write_all(bytemuck::bytes_of(&self.validation))?;
        writer.write_all(bytemuck::bytes_of(&self.default_entry))?;
        for (header, entries) in self.sections.iter() {
            writer.write_all(bytemuck::bytes_of(header))?;
            for entry in entries {
                writer.write_all(bytemuck::bytes_of(entry))?;
            }
        }
        // End of entries
        writer.write_all(&[0; 32])?;
        Ok(())
    }

    pub fn size(&self) -> usize {
        // 32 for the validation entry
        // 32 for the default entry
        // For each section:
        // 32 for header
        // and 32 for each entry
        64 + self
            .sections
            .iter()
            .map(|(_, entries)| entries.len() + 1)
            .sum::<usize>()
            * 32
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BootCatalogueEntry {
    Validation(BootValidationEntry),
    SectionHeader(BootSectionHeaderEntry),
    SectionEntry(BootSectionEntry),
    SectionEntryExtension(BootSectionEntryExtension),
}

impl BootCatalogueEntry {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            BootCatalogueEntry::Validation(entry) => bytemuck::bytes_of(entry),
            BootCatalogueEntry::SectionHeader(entry) => bytemuck::bytes_of(entry),
            BootCatalogueEntry::SectionEntry(entry) => bytemuck::bytes_of(entry),
            BootCatalogueEntry::SectionEntryExtension(entry) => bytemuck::bytes_of(entry),
        }
    }

    pub const fn size(&self) -> usize {
        32
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PlatformId {
    /// This is for X8086, X86, and X86_64 architectures.
    X80X86,
    PowerPC,
    Macintosh,
    UEFI,
    Unknown(u8),
}

impl PlatformId {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x00 => Self::X80X86,
            0x01 => Self::PowerPC,
            0x02 => Self::Macintosh,
            0xEF => Self::UEFI,
            value => Self::Unknown(value),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::X80X86 => 0x00,
            Self::PowerPC => 0x01,
            Self::Macintosh => 0x02,
            Self::UEFI => 0xEF,
            Self::Unknown(value) => value,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct BootValidationEntry {
    pub header_id: u8,
    pub platform_id: u8,
    pub reserved: [u8; 2],
    pub manufacturer: [u8; 24],
    pub checksum: U16<LittleEndian>,
    /// 0x55AA
    pub key: [u8; 2],
}

impl BootValidationEntry {
    pub fn new() -> Self {
        let mut entry = Self {
            header_id: 1,
            platform_id: 0,
            reserved: [0; 2],
            manufacturer: [0; 24],
            checksum: U16::new(0),
            key: [0x55, 0xAA],
        };
        entry.checksum.set(entry.calculate_checksum());
        entry
    }
}

impl Debug for BootValidationEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootValidationEntry")
            .field("header_id", &format!("{:#x}", self.header_id))
            .field("platform_id", &PlatformId::from_u8(self.platform_id))
            .field(
                "manufacturer",
                &core::str::from_utf8(&self.manufacturer).unwrap(),
            )
            .field("checksum", &self.checksum.get())
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl BootValidationEntry {
    pub fn parse<T: Read>(reader: &mut T) -> Result<Self, std::io::Error> {
        let mut buf: [u8; 32] = [0; 32];
        reader.read_exact(&mut buf)?;
        Ok(bytemuck::cast(buf))
    }

    pub fn is_valid(&self) -> bool {
        self.header_id == 0x01 && self.checksum.get() == self.calculate_checksum()
    }

    pub fn calculate_checksum(&self) -> u16 {
        let mut bytes = bytemuck::bytes_of(self).to_vec();
        bytes[28] = 0;
        bytes[29] = 0;
        let mut checksum = 0u16;
        for i in (0..32).step_by(2) {
            let value = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            checksum = checksum.wrapping_add(value);
        }
        (!checksum) + 1
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BootSectionHeaderEntry {
    /// 0x90 = Header, more headers follow
    /// 0x91 = Final header
    pub header_type: u8,
    pub platform_id: u8,
    pub section_count: U16<LittleEndian>,
    pub section_ident: [u8; 28],
}

impl Debug for BootSectionHeaderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootSectionHeaderEntry")
            .field("header_type", &format!("{:#x}", self.header_type))
            .field("platform_id", &PlatformId::from_u8(self.platform_id))
            .field("section_count", &self.section_count.get())
            .field(
                "section_ident",
                &core::str::from_utf8(&self.section_ident).unwrap(),
            )
            .finish_non_exhaustive()
    }
}

unsafe impl bytemuck::Zeroable for BootSectionHeaderEntry {}
unsafe impl bytemuck::Pod for BootSectionHeaderEntry {}

#[derive(Debug, Clone, Copy)]
pub enum EmulationType {
    /// 0x00 = No emulation
    NoEmulation,
    Unknown(u8),
}

impl EmulationType {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x00 => Self::NoEmulation,
            value => Self::Unknown(value),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::NoEmulation => 0x00,
            Self::Unknown(value) => value,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BootSectionEntry {
    /// 0x88 = Bootable, 0x00 = Not bootable
    pub boot_indicator: u8,
    pub boot_media_type: u8,
    pub load_segment: U16<LittleEndian>,
    pub system_type: u8,
    pub reserved0: u8,
    pub sector_count: U16<LittleEndian>,
    pub load_rba: U32<LittleEndian>,
    pub selection_criteria: u8,
    pub vendor_unique: [u8; 19],
}

impl BootSectionEntry {
    pub fn new(
        media_type: EmulationType,
        load_segment: u16,
        sector_count: u16,
        load_rba: u32,
    ) -> Self {
        Self {
            boot_indicator: 0x88,
            boot_media_type: media_type.to_u8(),
            load_segment: U16::new(load_segment),
            system_type: 0,
            reserved0: 0,
            sector_count: U16::new(sector_count),
            load_rba: U32::new(load_rba),
            selection_criteria: 0,
            vendor_unique: [0; 19],
        }
    }
}

impl Debug for BootSectionEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootSectionEntry")
            .field("boot_indicator", &format!("{:#x}", self.boot_indicator))
            .field(
                "boot_media_type",
                &EmulationType::from_u8(self.boot_media_type),
            )
            .field("load_segment", &self.load_segment.get())
            .field("system_type", &self.system_type)
            .field("sector_count", &self.sector_count.get())
            .field("load_rba", &self.load_rba.get())
            .field("selection_criteria", &self.selection_criteria)
            .finish_non_exhaustive()
    }
}

impl BootSectionEntry {
    pub fn parse<T: Read>(reader: &mut T) -> Result<Self, std::io::Error> {
        let mut buf: [u8; 32] = [0; 32];
        reader.read_exact(&mut buf)?;
        Ok(bytemuck::cast(buf))
    }

    pub fn is_valid(&self) -> bool {
        self.boot_indicator == 0x88
    }
}

unsafe impl bytemuck::Zeroable for BootSectionEntry {}
unsafe impl bytemuck::Pod for BootSectionEntry {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BootSectionEntryExtension {
    // Must be 0x44
    pub extension_indicator: u8,
    // Bit 5: 1 = more extensions follow, 0 = final extension
    pub flags: u8,
    pub vendor_unique: [u8; 30],
}

unsafe impl bytemuck::Zeroable for BootSectionEntryExtension {}
unsafe impl bytemuck::Pod for BootSectionEntryExtension {}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct BootInfoTable {
    pub iso_start: U32<LittleEndian>,
    pub file_lba: U32<LittleEndian>,
    pub file_len: U32<LittleEndian>,
    pub checksum: U32<LittleEndian>,
}

pub struct ElToritoWriter;

impl ElToritoWriter {
    pub fn create_descriptor(
        opts: &BootOptions,
        files: &mut FileInput,
    ) -> Result<BootRecordVolumeDescriptor, std::io::Error> {
        log::trace!("Adding boot record to volume descriptors");
        #[cfg(feature = "extra-checks")]
        for entry in opts.entries() {
            assert!(
                files.contains(&entry.boot_image_path),
                "Boot image path not found in files"
            );
        }

        if opts.write_boot_catalogue {
            log::trace!("Appending boot catalogue to file list");
            let size = 96 + opts.entries.len() * 64;
            let size = (size + 2047) & !2047;
            files.append(crate::file::File {
                path: "boot.catalog".to_string(),
                data: FileData::Data(vec![0; size]),
            });
        }
        Ok(BootRecordVolumeDescriptor::new(0))
    }

    /// Writes the boot catalogue and boot info table to the given writer
    pub fn write_catalog_and_table<W: ReadWriteSeek>(writer: &mut W, opts: &BootOptions, path_table: &PathTableRef) -> Result<(), std::io::Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_eq_size!(BootValidationEntry, [u8; 32]);
    static_assertions::assert_eq_size!(BootSectionHeaderEntry, [u8; 32]);
    static_assertions::assert_eq_size!(BootSectionEntry, [u8; 32]);

    static_assertions::assert_eq_align!(BootValidationEntry, u8);
    static_assertions::assert_eq_align!(BootSectionHeaderEntry, u8);
    static_assertions::assert_eq_align!(BootSectionEntry, u8);
}
