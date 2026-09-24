//! Raw exFAT on-disk layouts and constants.
//!
//! The items mirror the exFAT 1.00 specification. The module may gain items;
//! the existing ones follow the specification and stay exhaustive.

use hadris_common::types::{
    endian::LittleEndian,
    number::{U16, U32, U64},
};

/// `FileSystemName` of the boot sector.
pub const FILE_SYSTEM_NAME: [u8; 8] = *b"EXFAT   ";
/// `JumpBoot` of the boot sector.
pub const JUMP_BOOT: [u8; 3] = [0xEB, 0x76, 0x90];
/// `BootSignature` of the boot sector.
pub const BOOT_SIGNATURE: u16 = 0xAA55;
/// `ExtendedBootSignature`, the last four bytes of each extended boot
/// sector.
pub const EXTENDED_BOOT_SIGNATURE: u32 = 0xAA55_0000;
/// Sectors in each boot region: the boot sector, eight extended boot
/// sectors, the OEM parameters, a reserved sector and the checksum sector.
pub const BOOT_REGION_SECTORS: u64 = 12;
/// Byte offsets in the boot sector that the boot checksum skips:
/// `VolumeFlags` and `PercentInUse`.
pub const CHECKSUM_SKIPPED: [usize; 3] = [106, 107, 112];

/// `VolumeFlags` bit: the second FAT and bitmap are active (TexFAT only).
pub const VOLUME_ACTIVE_FAT: u16 = 0x0001;
/// `VolumeFlags` bit: the volume is probably inconsistent.
pub const VOLUME_DIRTY: u16 = 0x0002;
/// `VolumeFlags` bit: the media has reported failures.
pub const VOLUME_MEDIA_FAILURE: u16 = 0x0004;
/// `VolumeFlags` bit: clear before modifying the volume.
pub const VOLUME_CLEAR_TO_ZERO: u16 = 0x0008;

/// FAT entry 0, the media type.
pub const FAT_MEDIA: u32 = 0xFFFF_FFF8;
/// A FAT entry marking a bad cluster.
pub const FAT_BAD: u32 = 0xFFFF_FFF7;
/// A FAT entry ending a cluster chain.
pub const FAT_END: u32 = 0xFFFF_FFFF;
/// The first cluster of the cluster heap.
pub const FIRST_CLUSTER: u32 = 2;
/// The largest cluster count a volume may have.
pub const MAX_CLUSTER_COUNT: u32 = 0xFFFF_FFF5;

/// Size of every directory entry.
pub const ENTRY_SIZE: usize = 32;
/// The `InUse` bit of `EntryType`.
pub const IN_USE: u8 = 0x80;
/// End of directory: this entry and every later one are unused.
pub const ENTRY_END: u8 = 0x00;
/// Allocation Bitmap entry.
pub const ENTRY_BITMAP: u8 = 0x81;
/// Up-case Table entry.
pub const ENTRY_UPCASE: u8 = 0x82;
/// Volume Label entry.
pub const ENTRY_LABEL: u8 = 0x83;
/// File entry.
pub const ENTRY_FILE: u8 = 0x85;
/// Volume GUID entry.
pub const ENTRY_GUID: u8 = 0xA0;
/// TexFAT Padding entry.
pub const ENTRY_PADDING: u8 = 0xA1;
/// Stream Extension entry.
pub const ENTRY_STREAM: u8 = 0xC0;
/// File Name entry.
pub const ENTRY_NAME: u8 = 0xC1;
/// Vendor Extension entry.
pub const ENTRY_VENDOR_EXTENSION: u8 = 0xE0;
/// Vendor Allocation entry.
pub const ENTRY_VENDOR_ALLOCATION: u8 = 0xE1;
/// The `TypeCategory` bit of `EntryType`: set for secondary entries.
pub const CATEGORY_SECONDARY: u8 = 0x40;
/// The `TypeImportance` bit of `EntryType`: set for benign entries.
pub const IMPORTANCE_BENIGN: u8 = 0x20;

/// `FileAttributes` bit: read-only.
pub const ATTR_READ_ONLY: u16 = 0x0001;
/// `FileAttributes` bit: hidden.
pub const ATTR_HIDDEN: u16 = 0x0002;
/// `FileAttributes` bit: system.
pub const ATTR_SYSTEM: u16 = 0x0004;
/// `FileAttributes` bit: a directory.
pub const ATTR_DIRECTORY: u16 = 0x0010;
/// `FileAttributes` bit: archive.
pub const ATTR_ARCHIVE: u16 = 0x0020;

/// `GeneralSecondaryFlags` bit: a cluster allocation is possible.
pub const ALLOCATION_POSSIBLE: u8 = 0x01;
/// `GeneralSecondaryFlags` bit: the allocation is contiguous and the FAT
/// does not describe it.
pub const NO_FAT_CHAIN: u8 = 0x02;

/// UTF-16 code units in one File Name entry.
pub const NAME_UNITS_PER_ENTRY: usize = 15;
/// The longest file name, in UTF-16 code units.
pub const MAX_NAME_UNITS: usize = 255;
/// The longest volume label, in UTF-16 code units.
pub const MAX_LABEL_UNITS: usize = 11;
/// The largest directory, in bytes.
pub const MAX_DIRECTORY_SIZE: u64 = 256 << 20;
/// `OffsetValid` bit of a `UtcOffset` field.
pub const UTC_OFFSET_VALID: u8 = 0x80;

/// The `TableChecksum` of [`RECOMMENDED_UPCASE_TABLE`].
pub const RECOMMENDED_UPCASE_CHECKSUM: u32 = 0xE619_D30D;

/// The recommended up-case table of section 7.2.5.1 in compressed form: a
/// `0xFFFF` unit followed by a count stands for that many identity
/// mappings. The last unit maps `0xFFFF` to itself.
pub const RECOMMENDED_UPCASE_TABLE: &[u8; 5836] = include_bytes!("upcase.bin");

/// The Main and Backup Boot Sector.
///
/// @hadris-spec EXFAT:3.1
/// @hadris-compliance partial
/// @hadris-note Every field is checked at mount except `PartitionOffset` and `DriveSelect`; the backup boot region is compared by `check` but never used to mount.
/// @hadris-tests exfat_read::mount_rejects_bad_boot_sectors, exfat_format::formats_every_sector_size
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BootSector {
    /// `JumpBoot`
    pub jump_boot: [u8; 3],
    /// `FileSystemName`, `"EXFAT   "`.
    pub file_system_name: [u8; 8],
    /// `MustBeZero`
    pub must_be_zero: [u8; 53],
    /// `PartitionOffset`, in sectors from the start of the media.
    pub partition_offset: U64<LittleEndian>,
    /// `VolumeLength`, in sectors.
    pub volume_length: U64<LittleEndian>,
    /// `FatOffset`, in sectors from the start of the volume.
    pub fat_offset: U32<LittleEndian>,
    /// `FatLength`, in sectors.
    pub fat_length: U32<LittleEndian>,
    /// `ClusterHeapOffset`, in sectors from the start of the volume.
    pub cluster_heap_offset: U32<LittleEndian>,
    /// `ClusterCount`
    pub cluster_count: U32<LittleEndian>,
    /// `FirstClusterOfRootDirectory`
    pub first_cluster_of_root_directory: U32<LittleEndian>,
    /// `VolumeSerialNumber`
    pub volume_serial_number: U32<LittleEndian>,
    /// `FileSystemRevision`, major version in the high byte.
    pub file_system_revision: U16<LittleEndian>,
    /// `VolumeFlags`
    pub volume_flags: U16<LittleEndian>,
    /// `BytesPerSectorShift`
    pub bytes_per_sector_shift: u8,
    /// `SectorsPerClusterShift`
    pub sectors_per_cluster_shift: u8,
    /// `NumberOfFats`
    pub number_of_fats: u8,
    /// `DriveSelect`
    pub drive_select: u8,
    /// `PercentInUse`, or `0xFF` when unknown.
    pub percent_in_use: u8,
    /// `Reserved`
    pub reserved: [u8; 7],
    /// `BootCode`
    pub boot_code: [u8; 390],
    /// `BootSignature`
    pub boot_signature: U16<LittleEndian>,
}

/// The File directory entry, the primary entry of a file or directory.
///
/// @hadris-spec EXFAT:7.4
/// @hadris-compliance partial
/// @hadris-note Attributes and the three timestamps with their 10 ms increments and UTC offsets are read and written; entry sets with benign secondary entries are read and kept.
/// @hadris-tests exfat_write::times_and_attributes_round_trip, exfat_read::entry_sets_cross_clusters
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FileEntry {
    /// `EntryType`, [`ENTRY_FILE`].
    pub entry_type: u8,
    /// `SecondaryCount`, 2 to 18.
    pub secondary_count: u8,
    /// `SetChecksum`
    pub set_checksum: U16<LittleEndian>,
    /// `FileAttributes`
    pub file_attributes: U16<LittleEndian>,
    /// `Reserved1`
    pub reserved1: U16<LittleEndian>,
    /// `CreateTimestamp`
    pub create_timestamp: U32<LittleEndian>,
    /// `LastModifiedTimestamp`
    pub last_modified_timestamp: U32<LittleEndian>,
    /// `LastAccessedTimestamp`
    pub last_accessed_timestamp: U32<LittleEndian>,
    /// `Create10msIncrement`
    pub create_10ms_increment: u8,
    /// `LastModified10msIncrement`
    pub last_modified_10ms_increment: u8,
    /// `CreateUtcOffset`
    pub create_utc_offset: u8,
    /// `LastModifiedUtcOffset`
    pub last_modified_utc_offset: u8,
    /// `LastAccessedUtcOffset`
    pub last_accessed_utc_offset: u8,
    /// `Reserved2`
    pub reserved2: [u8; 7],
}

/// The Stream Extension directory entry: the allocation and sizes of a
/// file or directory.
///
/// @hadris-spec EXFAT:7.6
/// @hadris-compliance full
/// @hadris-tests exfat_read::valid_data_length_reads_zeros, exfat_write::contiguous_files_grow_into_chains
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StreamEntry {
    /// `EntryType`, [`ENTRY_STREAM`].
    pub entry_type: u8,
    /// `GeneralSecondaryFlags`
    pub general_secondary_flags: u8,
    /// `Reserved1`
    pub reserved1: u8,
    /// `NameLength`, in UTF-16 code units.
    pub name_length: u8,
    /// `NameHash`
    pub name_hash: U16<LittleEndian>,
    /// `Reserved2`
    pub reserved2: U16<LittleEndian>,
    /// `ValidDataLength`
    pub valid_data_length: U64<LittleEndian>,
    /// `Reserved3`
    pub reserved3: U32<LittleEndian>,
    /// `FirstCluster`
    pub first_cluster: U32<LittleEndian>,
    /// `DataLength`
    pub data_length: U64<LittleEndian>,
}

/// The File Name directory entry: 15 UTF-16 code units of a name.
///
/// @hadris-spec EXFAT:7.7
/// @hadris-compliance full
/// @hadris-tests exfat_write::long_names_up_to_255_units, codec::tests::names_are_checked
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NameEntry {
    /// `EntryType`, [`ENTRY_NAME`].
    pub entry_type: u8,
    /// `GeneralSecondaryFlags`
    pub general_secondary_flags: u8,
    /// `FileName`
    pub file_name: [U16<LittleEndian>; 15],
}

/// The Allocation Bitmap directory entry of the root directory.
///
/// @hadris-spec EXFAT:7.1
/// @hadris-compliance partial
/// @hadris-note The first bitmap is read and written through its FAT chain; the second bitmap of TexFAT volumes is not supported.
/// @hadris-tests exfat_read::fragmented_bitmap_and_upcase_table, exfat_check::bitmap_mismatches_are_found
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BitmapEntry {
    /// `EntryType`, [`ENTRY_BITMAP`].
    pub entry_type: u8,
    /// `BitmapFlags`; bit 0 selects the second bitmap.
    pub bitmap_flags: u8,
    /// `Reserved`
    pub reserved: [u8; 18],
    /// `FirstCluster`
    pub first_cluster: U32<LittleEndian>,
    /// `DataLength`, in bytes.
    pub data_length: U64<LittleEndian>,
}

/// The Up-case Table directory entry of the root directory.
///
/// @hadris-spec EXFAT:7.2
/// @hadris-compliance full
/// @hadris-tests exfat_read::fragmented_bitmap_and_upcase_table, exfat_check::upcase_checksum_is_checked
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UpcaseEntry {
    /// `EntryType`, [`ENTRY_UPCASE`].
    pub entry_type: u8,
    /// `Reserved1`
    pub reserved1: [u8; 3],
    /// `TableChecksum`
    pub table_checksum: U32<LittleEndian>,
    /// `Reserved2`
    pub reserved2: [u8; 12],
    /// `FirstCluster`
    pub first_cluster: U32<LittleEndian>,
    /// `DataLength`, in bytes.
    pub data_length: U64<LittleEndian>,
}

/// The Volume Label directory entry of the root directory.
///
/// @hadris-spec EXFAT:7.3
/// @hadris-compliance full
/// @hadris-tests exfat_write::labels_are_set_and_removed
/// @hadris-fuzz exfat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LabelEntry {
    /// `EntryType`, [`ENTRY_LABEL`].
    pub entry_type: u8,
    /// `CharacterCount`, 0 to 11.
    pub character_count: u8,
    /// `VolumeLabel`
    pub volume_label: [U16<LittleEndian>; 11],
    /// `Reserved`
    pub reserved: [u8; 8],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_have_their_sizes() {
        assert_eq!(size_of::<BootSector>(), 512);
        for size in [
            size_of::<FileEntry>(),
            size_of::<StreamEntry>(),
            size_of::<NameEntry>(),
            size_of::<BitmapEntry>(),
            size_of::<UpcaseEntry>(),
            size_of::<LabelEntry>(),
        ] {
            assert_eq!(size, ENTRY_SIZE);
        }
    }
}
