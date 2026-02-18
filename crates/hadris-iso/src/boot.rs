//! The El Torito boot specification
//!
//! This is used for booting from CDs and DVDs

use core::fmt::Debug;
use super::io::{self, Read, Seek, Write};

use crate::types::{Endian, LittleEndian, U16, U32};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[cfg(feature = "write")]
use super::{
    boot::options::BootOptions,
    volume::BootRecordVolumeDescriptor,
    write::{File, InputFiles},
};

/// Errors that can occur during boot catalog operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootError {
    /// I/O error occurred
    Io,
    /// Validation entry checksum is invalid
    InvalidValidationChecksum,
    /// Validation entry header ID is invalid (expected 0x01)
    InvalidValidationHeader,
    /// Validation entry key bytes are invalid (expected 0x55, 0xAA)
    InvalidValidationKey,
    /// Default boot entry is not marked as bootable
    InvalidDefaultEntry,
    /// Expected a section header but got something else
    ExpectedSectionHeader(u8),
    /// Boot catalog ended unexpectedly (more sections expected)
    UnexpectedEndOfCatalog,
}

impl core::fmt::Display for BootError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io => write!(f, "I/O error"),
            Self::InvalidValidationChecksum => {
                write!(f, "invalid boot catalog validation checksum")
            }
            Self::InvalidValidationHeader => {
                write!(f, "invalid boot catalog validation header (expected 0x01)")
            }
            Self::InvalidValidationKey => write!(
                f,
                "invalid boot catalog validation key (expected 0x55, 0xAA)"
            ),
            Self::InvalidDefaultEntry => write!(f, "default boot entry is not marked as bootable"),
            Self::ExpectedSectionHeader(id) => write!(f, "expected section header, got: {:#x}", id),
            Self::UnexpectedEndOfCatalog => write!(f, "boot catalog ended unexpectedly"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BootError {}

// Types for El Torito boot catalogue
// The boot catalogue consists of a series of boot catalogue entries:
// First, the validation entry
// Next, the initial/default entry
// Section headers,
// Section entries,
// Section entry extensions

/// The base of the boot catalog
/// This is the minimum required specified by the El-Torito specification
#[derive(Debug, Clone)]
pub struct BaseBootCatalog {
    pub validation: BootValidationEntry,
    pub default_entry: BootSectionEntry,
}

impl Default for BaseBootCatalog {
    fn default() -> Self {
        Self::new(EmulationType::NoEmulation, 0, 0, 0)
    }
}

impl BaseBootCatalog {
    pub fn new(
        media_type: EmulationType,
        load_segment: u16,
        sector_count: u16,
        load_rba: u32,
    ) -> Self {
        Self {
            validation: BootValidationEntry::new(),
            default_entry: BootSectionEntry::new(media_type, load_segment, sector_count, load_rba),
        }
    }

}

/// Boot catalogue (requires alloc feature for dynamic sections)
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BootCatalog {
    base: BaseBootCatalog,
    sections: Vec<(BootSectionHeaderEntry, Vec<BootSectionEntry>)>,
}

#[cfg(feature = "alloc")]
impl Default for BootCatalog {
    fn default() -> Self {
        Self::new(EmulationType::NoEmulation, 0, 0, 0)
    }
}

#[cfg(feature = "alloc")]
impl BootCatalog {
    pub fn new(
        media_type: EmulationType,
        load_segment: u16,
        sector_count: u16,
        load_rba: u32,
    ) -> Self {
        Self {
            base: BaseBootCatalog::new(media_type, load_segment, sector_count, load_rba),
            sections: Vec::new(),
        }
    }

    pub fn set_default_entry(&mut self, entry: BootSectionEntry) {
        self.base.default_entry = entry;
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

    /// Returns the total size of the boot catalog in bytes
    ///
    /// This includes:
    /// - 32 bytes for the validation entry
    /// - 32 bytes for the default/initial entry
    /// - 32 bytes for each section header
    /// - 32 bytes for each section entry
    /// - 32 bytes for the terminator entry
    pub fn size(&self) -> usize {
        // Base: validation (32) + default entry (32) = 64 bytes
        // Each section: header (32) + entries (32 each)
        // Terminator: 32 bytes
        let sections_size: usize = self
            .sections
            .iter()
            .map(|(_, entries)| (entries.len() + 1) * 32)
            .sum();
        64 + sections_size + 32 // +32 for terminator
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BootCatalogEntry {
    Validation(BootValidationEntry),
    SectionHeader(BootSectionHeaderEntry),
    SectionEntry(BootSectionEntry),
    SectionEntryExtension(BootSectionEntryExtension),
}

impl BootCatalogEntry {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            BootCatalogEntry::Validation(entry) => bytemuck::bytes_of(entry),
            BootCatalogEntry::SectionHeader(entry) => bytemuck::bytes_of(entry),
            BootCatalogEntry::SectionEntry(entry) => bytemuck::bytes_of(entry),
            BootCatalogEntry::SectionEntryExtension(entry) => bytemuck::bytes_of(entry),
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

impl Default for BootValidationEntry {
    fn default() -> Self {
        Self::new()
    }
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
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BootValidationEntry")
            .field("header_id", &format_args!("{:#x}", self.header_id))
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
    /// Check if the validation entry is valid (legacy method)
    pub fn is_valid(&self) -> bool {
        self.header_id == 0x01
            && self.key == [0x55, 0xAA]
            && self.checksum.get() == self.calculate_checksum()
    }

    /// Validate the entry and return a detailed error if invalid
    pub fn validate(&self) -> Result<(), BootError> {
        if self.header_id != 0x01 {
            return Err(BootError::InvalidValidationHeader);
        }
        if self.key != [0x55, 0xAA] {
            return Err(BootError::InvalidValidationKey);
        }
        if self.checksum.get() != self.calculate_checksum() {
            return Err(BootError::InvalidValidationChecksum);
        }
        Ok(())
    }

    /// Calculates the checksum of the boot catalogue
    ///
    /// The checksum works such that the checksum of the data (including checksum bytes) is 0.
    /// We can do this by finding the sum of the data without the checksum bytes, and negating it
    /// (using two's complement).
    pub fn calculate_checksum(&self) -> u16 {
        // We know the size of the struct, we we can just stack allocate a buffer and copy the data
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(bytemuck::bytes_of(self));
        // Zero out the checksum bytes (we are basically just ignoring them), since we need to find
        // what the data equal without them
        bytes[28] = 0;
        bytes[29] = 0;
        let mut checksum = 0u16;
        for i in (0..32).step_by(2) {
            let value = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            checksum = checksum.wrapping_add(value);
        }
        // We use two's complement to negate the checksum, so that the checksum + data = 0 (in 16-bit)
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
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BootSectionHeaderEntry")
            .field("header_type", &format_args!("{:#x}", self.header_type))
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
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BootSectionEntry")
            .field(
                "boot_indicator",
                &format_args!("{:#x}", self.boot_indicator),
            )
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
    /// Check if this entry is marked as bootable (0x88)
    pub fn is_bootable(&self) -> bool {
        self.boot_indicator == 0x88
    }

    /// Legacy alias for is_bootable
    pub fn is_valid(&self) -> bool {
        self.is_bootable()
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

/// Boot information table (16 bytes)
///
/// This table is located in the boot binary and contains information about the
/// ISO image and the boot binary. It is written at offset 8 in the boot image.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct BootInfoTable {
    /// The start LBA of the ISO image (This would 16 in most cases)
    pub iso_start: U32<LittleEndian>,
    /// The start LBA of the boot binary
    pub file_lba: U32<LittleEndian>,
    /// The length of the boot binary (in bytes)
    pub file_len: U32<LittleEndian>,
    /// The checksum of the boot binary
    pub checksum: U32<LittleEndian>,
}

/// GRUB2/ISOLINUX boot information table (56 bytes)
///
/// This is the extended boot info table format used by GRUB2 and ISOLINUX.
/// It is the same as BootInfoTable but includes 40 bytes of reserved space.
/// Written at offset 8 in the boot image.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct Grub2BootInfoTable {
    /// The start LBA of the primary volume descriptor (usually 16)
    pub pvd_lba: U32<LittleEndian>,
    /// The start LBA of the boot binary
    pub file_lba: U32<LittleEndian>,
    /// The length of the boot binary (in bytes)
    pub file_len: U32<LittleEndian>,
    /// The checksum of the boot binary (sum of 32-bit words from offset 64 to EOF)
    pub checksum: U32<LittleEndian>,
    /// Reserved bytes (must be zero)
    pub reserved: [u8; 40],
}

io_transform! {
impl BaseBootCatalog {
    /// Parse the base boot catalog from the reader
    ///
    /// # Errors
    /// Returns an error if:
    /// - I/O error occurs
    /// - Validation entry checksum is invalid
    /// - Default entry is not marked as bootable
    pub async fn parse<R: Read + Seek>(reader: &mut R) -> Result<Self, BootError> {
        let validation = BootValidationEntry::parse(reader).await.map_err(|_| BootError::Io)?;
        validation.validate()?;

        let default_entry = BootSectionEntry::parse(reader).await.map_err(|_| BootError::Io)?;
        if !default_entry.is_bootable() {
            return Err(BootError::InvalidDefaultEntry);
        }

        Ok(Self {
            validation,
            default_entry,
        })
    }

    pub async fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(bytemuck::bytes_of(&self.validation)).await?;
        writer.write_all(bytemuck::bytes_of(&self.default_entry)).await?;
        Ok(())
    }
}

impl BootValidationEntry {
    pub async fn parse<T: Read>(reader: &mut T) -> Result<Self, io::Error> {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf).await?;
        Ok(bytemuck::cast(buf))
    }
}

impl BootSectionEntry {
    pub async fn parse<T: Read>(reader: &mut T) -> Result<Self, io::Error> {
        let mut buf: [u8; 32] = [0; 32];
        reader.read_exact(&mut buf).await?;
        Ok(bytemuck::cast(buf))
    }
}

#[cfg(feature = "alloc")]
impl BootCatalog {
    /// Parse the boot catalogue from the given reader,
    /// expects the reader to seek to the start of the catalogue
    ///
    /// # Errors
    /// Returns an error if the boot catalog is malformed or an I/O error occurs.
    pub async fn parse<T: Read + Seek>(reader: &mut T) -> Result<Self, BootError> {
        let base = BaseBootCatalog::parse(reader).await?;
        let mut sections = Vec::new();
        let mut buffer = [0u8; 32];
        let mut has_more = false;
        let mut header = None;
        let mut entries = Vec::new();
        loop {
            reader.read_exact(&mut buffer).await.map_err(|_| BootError::Io)?;
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
                        return Err(BootError::ExpectedSectionHeader(id));
                    }
                    entries.push(bytemuck::cast(buffer));
                }
            }
        }

        if has_more {
            return Err(BootError::UnexpectedEndOfCatalog);
        }
        if let Some(header) = header {
            sections.push((header, entries));
        }

        Ok(Self { base, sections })
    }

    pub async fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.base.write(writer).await?;
        for (header, entries) in self.sections.iter() {
            writer.write_all(bytemuck::bytes_of(header)).await?;
            for entry in entries {
                writer.write_all(bytemuck::bytes_of(entry)).await?;
            }
        }
        // End of entries
        writer.write_all(&[0; 32]).await?;
        Ok(())
    }
}
} // io_transform!

#[cfg(feature = "write")]
pub struct ElToritoWriter;

#[cfg(feature = "write")]
impl ElToritoWriter {
    /// Creates the El-Torito ovlume descriptor based on the given options and files
    /// This will append a boot catalogue to the given files if the options require it
    /// When extra checks are enabled, this will also check that the boot entry paths are valid
    /// (included in the files)
    pub fn create_descriptor(
        opts: &BootOptions,
        files: &mut InputFiles,
    ) -> BootRecordVolumeDescriptor {
        if opts.write_boot_catalog {
            use alloc::string::ToString;
            use std::sync::Arc;

            let size = 96 + opts.entries.len() * 64;
            let size = (size + 2047) & !2047;
            let dir_pos = files
                .files
                .iter()
                .position(|f| matches!(f, File::Directory { .. }))
                .unwrap_or(0);
            files.files.insert(
                dir_pos.saturating_sub(1),
                File::File {
                    name: Arc::new("boot.catalog".to_string()),
                    contents: alloc::vec![0; size],
                },
            );
        }
        BootRecordVolumeDescriptor::new(0)
    }
}

#[cfg(feature = "write")]
pub mod options {
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::num::NonZeroU16;

    use super::{EmulationType, PlatformId};

    #[derive(Debug, Clone, Default)]
    pub struct BootOptions {
        pub write_boot_catalog: bool,
        pub default: BootEntryOptions,
        pub entries: Vec<(BootSectionOptions, BootEntryOptions)>,
    }

    impl BootOptions {
        pub fn sections(&self) -> Vec<(Option<BootSectionOptions>, BootEntryOptions)> {
            let mut sections = Vec::with_capacity(self.entries.len() + 1);
            sections.push((None, self.default.clone()));
            for (section_ops, ops) in &self.entries {
                sections.push((Some(section_ops.clone()), ops.clone()));
            }
            sections
        }
    }

    #[derive(Debug, Clone)]
    pub struct BootSectionOptions {
        pub platform: PlatformId,
    }

    #[derive(Debug, Clone)]
    pub struct BootEntryOptions {
        pub load_size: Option<NonZeroU16>,
        pub boot_image_path: String,
        pub boot_info_table: bool,
        pub grub2_boot_info: bool,
        pub emulation: EmulationType,
    }

    impl Default for BootEntryOptions {
        fn default() -> Self {
            Self {
                load_size: None,
                boot_image_path: String::new(),
                boot_info_table: false,
                grub2_boot_info: false,
                emulation: EmulationType::NoEmulation,
            }
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    static_assertions::assert_eq_size!(BootValidationEntry, [u8; 32]);
    static_assertions::assert_eq_size!(BootSectionHeaderEntry, [u8; 32]);
    static_assertions::assert_eq_size!(BootSectionEntry, [u8; 32]);

    static_assertions::assert_eq_align!(BootValidationEntry, u8);
    static_assertions::assert_eq_align!(BootSectionHeaderEntry, u8);
    static_assertions::assert_eq_align!(BootSectionEntry, u8);

    #[test]
    fn test_validation_entry_new() {
        let entry = BootValidationEntry::new();
        assert_eq!(entry.header_id, 0x01);
        assert_eq!(entry.key, [0x55, 0xAA]);
        assert!(entry.is_valid());
        assert!(entry.validate().is_ok());
    }

    #[test]
    fn test_validation_entry_checksum() {
        let entry = BootValidationEntry::new();
        // Checksum should be calculated so that sum of all 16-bit words = 0
        let bytes = bytemuck::bytes_of(&entry);
        let mut sum = 0u16;
        for i in (0..32).step_by(2) {
            let value = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            sum = sum.wrapping_add(value);
        }
        assert_eq!(sum, 0, "checksum should make total sum equal to 0");
    }

    #[test]
    fn test_validation_entry_invalid_header() {
        let mut entry = BootValidationEntry::new();
        entry.header_id = 0x00;
        assert!(!entry.is_valid());
        assert_eq!(entry.validate(), Err(BootError::InvalidValidationHeader));
    }

    #[test]
    fn test_validation_entry_invalid_key() {
        let mut entry = BootValidationEntry::new();
        entry.key = [0x00, 0x00];
        assert!(!entry.is_valid());
        assert_eq!(entry.validate(), Err(BootError::InvalidValidationKey));
    }

    #[test]
    fn test_validation_entry_invalid_checksum() {
        let mut entry = BootValidationEntry::new();
        entry.checksum.set(0x1234);
        assert!(!entry.is_valid());
        assert_eq!(entry.validate(), Err(BootError::InvalidValidationChecksum));
    }

    #[test]
    fn test_section_entry_new() {
        let entry = BootSectionEntry::new(EmulationType::NoEmulation, 0x07C0, 4, 20);
        assert!(entry.is_bootable());
        assert!(entry.is_valid());
        assert_eq!(entry.boot_indicator, 0x88);
        assert_eq!(entry.boot_media_type, 0x00);
        assert_eq!(entry.load_segment.get(), 0x07C0);
        assert_eq!(entry.sector_count.get(), 4);
        assert_eq!(entry.load_rba.get(), 20);
    }

    #[test]
    fn test_section_entry_not_bootable() {
        let mut entry = BootSectionEntry::new(EmulationType::NoEmulation, 0, 1, 20);
        entry.boot_indicator = 0x00;
        assert!(!entry.is_bootable());
    }

    #[test]
    fn test_base_boot_catalog_size() {
        let catalog = BaseBootCatalog::default();
        let mut buf = Vec::new();
        catalog.write(&mut buf).unwrap();
        assert_eq!(
            buf.len(),
            64,
            "base catalog should be 64 bytes (validation + default entry)"
        );
    }

    #[test]
    fn test_boot_catalog_size() {
        let catalog = BootCatalog::default();
        // Default catalog: validation (32) + default entry (32) + terminator (32) = 96
        assert_eq!(catalog.size(), 96);
    }

    #[test]
    fn test_boot_catalog_with_section() {
        let mut catalog = BootCatalog::default();
        catalog.add_section(
            PlatformId::X80X86,
            vec![BootSectionEntry::new(EmulationType::NoEmulation, 0, 1, 30)],
        );
        // validation (32) + default (32) + section header (32) + 1 entry (32) + terminator (32) = 160
        assert_eq!(catalog.size(), 160);
    }

    #[test]
    fn test_boot_catalog_roundtrip() {
        use std::io::Cursor;

        let mut catalog = BootCatalog::default();
        catalog.add_section(
            PlatformId::X80X86,
            vec![BootSectionEntry::new(EmulationType::NoEmulation, 0, 1, 30)],
        );

        let mut buf = Vec::new();
        catalog.write(&mut buf).unwrap();

        let mut cursor = Cursor::new(buf);
        let parsed = BootCatalog::parse(&mut cursor).unwrap();

        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections[0].1.len(), 1);
    }

    #[test]
    fn test_platform_id_roundtrip() {
        for id in [
            PlatformId::X80X86,
            PlatformId::PowerPC,
            PlatformId::Macintosh,
            PlatformId::UEFI,
        ] {
            let byte = id.to_u8();
            let recovered = PlatformId::from_u8(byte);
            assert_eq!(recovered.to_u8(), byte);
        }
    }

    #[test]
    fn test_emulation_type_roundtrip() {
        let no_emul = EmulationType::NoEmulation;
        assert_eq!(no_emul.to_u8(), 0x00);
        assert!(matches!(
            EmulationType::from_u8(0x00),
            EmulationType::NoEmulation
        ));

        let unknown = EmulationType::Unknown(0x42);
        assert_eq!(unknown.to_u8(), 0x42);
    }
}
