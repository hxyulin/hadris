use core::fmt;

use super::{U16Le, U32Le};

/// The boot indicator of a bootable entry.
pub const BOOTABLE: u8 = 0x88;
/// The boot indicator of an entry that is not bootable.
pub const NOT_BOOTABLE: u8 = 0x00;
/// A section header followed by more headers.
pub const HEADER_MORE: u8 = 0x90;
/// The last section header.
pub const HEADER_FINAL: u8 = 0x91;

/// El Torito boot catalog validation entry.
///
/// @hadris-spec El-Torito:validation
/// @hadris-compliance partial
/// @hadris-note The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation.
/// @hadris-tests iso::boot::test_eltorito_boot_catalog_comparison
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootValidationEntry {
    /// 1.
    pub header_id: u8,
    /// The platform of the default entry.
    pub platform_id: u8,
    /// Reserved, zero.
    pub reserved: [u8; 2],
    /// The manufacturer or developer of the CD.
    pub id_string: [u8; 24],
    /// Makes the sum of the entry's 16-bit words zero.
    pub checksum: U16Le,
    /// `55 AA`.
    pub key: [u8; 2],
}

impl BootValidationEntry {
    /// A validation entry for `platform_id`, with its checksum.
    pub fn new(platform_id: u8) -> Self {
        let mut entry = Self {
            header_id: 1,
            platform_id,
            reserved: [0; 2],
            id_string: [0; 24],
            checksum: U16Le::new(0),
            key: [0x55, 0xAA],
        };
        entry.checksum = U16Le::new(entry.expected_checksum());
        entry
    }

    /// The checksum that makes the 16-bit words sum to zero.
    pub fn expected_checksum(&self) -> u16 {
        let mut bytes: [u8; 32] = bytemuck::cast(*self);
        bytes[28] = 0;
        bytes[29] = 0;
        let sum = bytes.chunks_exact(2).fold(0u16, |sum, pair| {
            sum.wrapping_add(u16::from_le_bytes([pair[0], pair[1]]))
        });
        sum.wrapping_neg()
    }

    /// Whether the header id, key and checksum are correct.
    pub fn is_valid(&self) -> bool {
        self.header_id == 1
            && self.key == [0x55, 0xAA]
            && self.checksum.get() == self.expected_checksum()
    }
}

impl fmt::Debug for BootValidationEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootValidationEntry")
            .field("header_id", &self.header_id)
            .field("platform_id", &self.platform_id)
            .field("checksum", &self.checksum)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// El Torito section header entry (0x90/0x91) introducing a platform's boot
/// entries.
///
/// @hadris-spec El-Torito:section-header
/// @hadris-compliance partial
/// @hadris-note The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation.
/// @hadris-tests iso::boot::test_hadris_multisection_boot_catalog
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootCatalogHeader {
    /// [`HEADER_MORE`] or [`HEADER_FINAL`].
    pub header_type: u8,
    /// The platform of the section's entries.
    pub platform_id: u8,
    /// The number of entries in the section.
    pub section_count: U16Le,
    /// Identifies the section.
    pub id_string: [u8; 28],
}

impl fmt::Debug for BootCatalogHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootCatalogHeader")
            .field("header_type", &self.header_type)
            .field("platform_id", &self.platform_id)
            .field("section_count", &self.section_count)
            .finish_non_exhaustive()
    }
}

/// El Torito initial/default or section entry: the bootable image and its
/// emulation media type.
///
/// @hadris-spec El-Torito:section-entry
/// @hadris-compliance partial
/// @hadris-note The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation.
/// @hadris-tests iso::boot::test_floppy_emulation_media_type_and_default_load_size
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootSectionEntry {
    /// [`BOOTABLE`] or [`NOT_BOOTABLE`].
    pub boot_indicator: u8,
    /// The emulation media type in the low nibble; flags above.
    pub boot_media_type: u8,
    /// The real-mode load segment, 0 for the default 0x7C0.
    pub load_segment: U16Le,
    /// The partition type byte of a hard-disk image.
    pub system_type: u8,
    /// Unused, zero.
    pub unused: u8,
    /// The number of 512-byte virtual sectors to load.
    pub sector_count: U16Le,
    /// The logical block of the image.
    pub load_rba: U32Le,
    /// Selection criteria type; zero for none.
    pub selection_criteria: u8,
    /// Vendor-unique selection criteria.
    pub vendor_unique: [u8; 19],
}

impl BootSectionEntry {
    /// A bootable entry.
    pub fn new(media_type: u8, load_segment: u16, sector_count: u16, load_rba: u32) -> Self {
        Self {
            boot_indicator: BOOTABLE,
            boot_media_type: media_type,
            load_segment: U16Le::new(load_segment),
            system_type: 0,
            unused: 0,
            sector_count: U16Le::new(sector_count),
            load_rba: U32Le::new(load_rba),
            selection_criteria: 0,
            vendor_unique: [0; 19],
        }
    }
}

impl fmt::Debug for BootSectionEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootSectionEntry")
            .field("boot_indicator", &self.boot_indicator)
            .field("boot_media_type", &self.boot_media_type)
            .field("load_segment", &self.load_segment)
            .field("system_type", &self.system_type)
            .field("sector_count", &self.sector_count)
            .field("load_rba", &self.load_rba)
            .finish_non_exhaustive()
    }
}

/// El Torito section entry extension (0x44).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootSectionEntryExtension {
    /// 0x44.
    pub extension_indicator: u8,
    /// Bit 5 set when more extensions follow.
    pub flags: u8,
    /// Vendor-unique selection criteria.
    pub vendor_unique: [u8; 30],
}

impl fmt::Debug for BootSectionEntryExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootSectionEntryExtension")
            .field("flags", &self.flags)
            .finish_non_exhaustive()
    }
}

/// The boot information table mkisofs writes at byte 8 of a no-emulation
/// boot image (`-boot-info-table`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootInfoTable {
    /// The logical block of the primary volume descriptor, 16.
    pub pvd_lba: U32Le,
    /// The logical block of the boot image.
    pub file_lba: U32Le,
    /// The length of the boot image in bytes.
    pub file_len: U32Le,
    /// The sum of the image's 32-bit little-endian words from byte 64.
    pub checksum: U32Le,
}

/// The GRUB 2 and ISOLINUX form of [`BootInfoTable`], with 40 reserved
/// bytes after it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Grub2BootInfoTable {
    /// The logical block of the primary volume descriptor, 16.
    pub pvd_lba: U32Le,
    /// The logical block of the boot image.
    pub file_lba: U32Le,
    /// The length of the boot image in bytes.
    pub file_len: U32Le,
    /// The sum of the image's 32-bit little-endian words from byte 64.
    pub checksum: U32Le,
    /// Reserved, zero.
    pub reserved: [u8; 40],
}

const _: () = {
    assert!(size_of::<BootValidationEntry>() == 32);
    assert!(size_of::<BootCatalogHeader>() == 32);
    assert!(size_of::<BootSectionEntry>() == 32);
    assert!(size_of::<BootSectionEntryExtension>() == 32);
    assert!(size_of::<BootInfoTable>() == 16);
    assert!(size_of::<Grub2BootInfoTable>() == 56);
};
