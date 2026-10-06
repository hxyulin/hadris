//! On-disk layouts and codes of NTFS.
//!
//! The items here mirror the on-disk format byte for byte. The module may
//! gain items; the existing ones follow the format and stay exhaustive. The
//! crate root never re-exports them. MFT records, attributes and index
//! records are parsed field by field, because their update sequence arrays
//! are applied in place first, so only the boot sector is a layout.
//!
//! Every multi-byte field of [`BootSector`] is a little-endian byte array,
//! so the layout has alignment one and can be read from any buffer with
//! [`bytemuck::pod_read_unaligned`].

/// The NTFS boot sector, the first 512 bytes of the volume.
///
/// @hadris-spec NTFS:Boot-Sector
/// @hadris-compliance partial
/// @hadris-tests crafted::open_rejects_bad_boot_sectors, raw::tests::boot_sector_reads_its_fields
/// @hadris-fuzz ntfs_read
/// @hadris-note Geometry and locations are validated; the checksum and the backup boot sector are not used.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootSector {
    /// Jump instruction.
    pub jump: [u8; 3],
    /// OEM identifier, [`OEM_ID`] on an NTFS volume.
    pub oem_id: [u8; 8],
    /// Bytes per sector.
    pub bytes_per_sector: [u8; 2],
    /// Sectors per cluster, a power of two.
    pub sectors_per_cluster: u8,
    /// Reserved sectors, zero on NTFS.
    pub reserved_sectors: [u8; 2],
    /// Unused FAT fields.
    pub unused0: [u8; 5],
    /// Media descriptor.
    pub media_descriptor: u8,
    /// Unused FAT field.
    pub unused1: [u8; 2],
    /// Sectors per track.
    pub sectors_per_track: [u8; 2],
    /// Number of heads.
    pub heads: [u8; 2],
    /// Sectors before the volume.
    pub hidden_sectors: [u8; 4],
    /// Unused FAT fields.
    pub unused2: [u8; 8],
    /// Sectors in the volume.
    pub total_sectors: [u8; 8],
    /// Cluster of the start of `$MFT`.
    pub mft_lcn: [u8; 8],
    /// Cluster of the start of `$MFTMirr`.
    pub mft_mirror_lcn: [u8; 8],
    /// Size of an MFT record: clusters when positive, `2^-n` bytes when
    /// negative (as `i8`).
    pub clusters_per_mft_record: u8,
    /// Reserved.
    pub reserved0: [u8; 3],
    /// Size of an index record, encoded as `clusters_per_mft_record`.
    pub clusters_per_index_record: u8,
    /// Reserved.
    pub reserved1: [u8; 3],
    /// Volume serial number.
    pub volume_serial: [u8; 8],
    /// Checksum.
    pub checksum: [u8; 4],
    /// Bootstrap code.
    pub bootstrap: [u8; 426],
    /// End of sector marker, [`BOOT_SIGNATURE`].
    pub signature: [u8; 2],
}

const _: () = assert!(size_of::<BootSector>() == 512);

impl BootSector {
    /// `bytes_per_sector` as a number.
    pub const fn sector_size(&self) -> u16 {
        u16::from_le_bytes(self.bytes_per_sector)
    }

    /// `total_sectors` as a number.
    pub const fn sector_count(&self) -> u64 {
        u64::from_le_bytes(self.total_sectors)
    }

    /// `mft_lcn` as a number.
    pub const fn mft_cluster(&self) -> u64 {
        u64::from_le_bytes(self.mft_lcn)
    }

    /// `volume_serial` as a number.
    pub const fn serial(&self) -> u64 {
        u64::from_le_bytes(self.volume_serial)
    }

    /// `signature` as a number.
    pub const fn signature_value(&self) -> u16 {
        u16::from_le_bytes(self.signature)
    }
}

/// The OEM identifier of an NTFS boot sector.
pub const OEM_ID: [u8; 8] = *b"NTFS    ";
/// The end of sector marker of a boot sector.
pub const BOOT_SIGNATURE: u16 = 0xAA55;

/// `$STANDARD_INFORMATION` attribute type.
pub const ATTR_STANDARD_INFORMATION: u32 = 0x10;
/// `$ATTRIBUTE_LIST` attribute type.
pub const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
/// `$FILE_NAME` attribute type.
pub const ATTR_FILE_NAME: u32 = 0x30;
/// `$OBJECT_ID` attribute type.
pub const ATTR_OBJECT_ID: u32 = 0x40;
/// `$SECURITY_DESCRIPTOR` attribute type.
pub const ATTR_SECURITY_DESCRIPTOR: u32 = 0x50;
/// `$VOLUME_NAME` attribute type.
pub const ATTR_VOLUME_NAME: u32 = 0x60;
/// `$VOLUME_INFORMATION` attribute type.
pub const ATTR_VOLUME_INFORMATION: u32 = 0x70;
/// `$DATA` attribute type.
pub const ATTR_DATA: u32 = 0x80;
/// `$INDEX_ROOT` attribute type.
pub const ATTR_INDEX_ROOT: u32 = 0x90;
/// `$INDEX_ALLOCATION` attribute type.
pub const ATTR_INDEX_ALLOCATION: u32 = 0xA0;
/// `$BITMAP` attribute type.
pub const ATTR_BITMAP: u32 = 0xB0;
/// `$REPARSE_POINT` attribute type.
pub const ATTR_REPARSE_POINT: u32 = 0xC0;
/// The type that ends the attributes of an MFT record.
pub const ATTR_END: u32 = 0xFFFF_FFFF;

/// Attribute flag: the value is compressed.
pub const ATTR_FLAG_COMPRESSED: u16 = 0x0001;
/// Attribute flag: the value is encrypted.
pub const ATTR_FLAG_ENCRYPTED: u16 = 0x4000;
/// Attribute flag: the value is sparse.
pub const ATTR_FLAG_SPARSE: u16 = 0x8000;

/// MFT record flag: the record is in use.
pub const MFT_RECORD_IN_USE: u16 = 0x0001;
/// MFT record flag: the record is a directory.
pub const MFT_RECORD_IS_DIRECTORY: u16 = 0x0002;

/// Index entry flag: a child node pointer follows the entry.
pub const INDEX_ENTRY_SUBNODE: u32 = 0x0001;
/// Index entry flag: the last entry of a node, with no key.
pub const INDEX_ENTRY_LAST: u32 = 0x0002;

/// `$FILE_NAME` namespace: POSIX, case-sensitive.
pub const FILE_NAME_POSIX: u8 = 0;
/// `$FILE_NAME` namespace: Win32, case-insensitive.
pub const FILE_NAME_WIN32: u8 = 1;
/// `$FILE_NAME` namespace: DOS 8.3 alias.
pub const FILE_NAME_DOS: u8 = 2;
/// `$FILE_NAME` namespace: a name that is both the Win32 and the DOS name.
pub const FILE_NAME_WIN32_AND_DOS: u8 = 3;

/// File attribute: read-only.
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0001;
/// File attribute: hidden.
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0002;
/// File attribute: system.
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0004;
/// File attribute: archive.
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0020;
/// File attribute: sparse.
pub const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x0200;
/// File attribute: a reparse point.
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
/// File attribute: compressed.
pub const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x0800;
/// File attribute: encrypted.
pub const FILE_ATTRIBUTE_ENCRYPTED: u32 = 0x4000;
/// File attribute in `$FILE_NAME`: the file has a `$I30` index, it is a
/// directory.
pub const FILE_NAME_INDEX_PRESENT: u32 = 0x1000_0000;

/// MFT record of `$MFT`.
pub const RECORD_MFT: u64 = 0;
/// MFT record of `$Volume`.
pub const RECORD_VOLUME: u64 = 3;
/// MFT record of the root directory.
pub const RECORD_ROOT: u64 = 5;
/// MFT record of `$Bitmap`.
pub const RECORD_BITMAP: u64 = 6;
/// MFT record of `$UpCase`.
pub const RECORD_UPCASE: u64 = 10;
/// The first MFT record of an ordinary file. Records below it hold the
/// metadata files.
pub const RECORD_FIRST_USER: u64 = 16;

/// The UTF-16LE name of a directory's file name index, `$I30`.
pub const I30: [u8; 8] = [0x24, 0x00, 0x49, 0x00, 0x33, 0x00, 0x30, 0x00];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_sector_reads_its_fields() {
        let mut bytes = [0u8; 512];
        bytes[3..11].copy_from_slice(&OEM_ID);
        bytes[11..13].copy_from_slice(&4096u16.to_le_bytes());
        bytes[13] = 1;
        bytes[40..48].copy_from_slice(&1000u64.to_le_bytes());
        bytes[48..56].copy_from_slice(&4u64.to_le_bytes());
        bytes[72..80].copy_from_slice(&0x1234u64.to_le_bytes());
        bytes[510..512].copy_from_slice(&BOOT_SIGNATURE.to_le_bytes());
        let boot: BootSector = bytemuck::pod_read_unaligned(&bytes[..]);
        assert_eq!(boot.oem_id, OEM_ID);
        assert_eq!(boot.sector_size(), 4096);
        assert_eq!(boot.sector_count(), 1000);
        assert_eq!(boot.mft_cluster(), 4);
        assert_eq!(boot.serial(), 0x1234);
        assert_eq!(boot.signature_value(), BOOT_SIGNATURE);
        assert_eq!(bytemuck::bytes_of(&boot), &bytes[..]);
    }
}
