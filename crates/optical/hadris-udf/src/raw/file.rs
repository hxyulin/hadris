use super::{CharSpec, EntityId, LbAddr, LongAd, Tag, Timestamp, U16Le, U32Le, U64Le};

/// File Set Descriptor (ECMA-167 4/14.1).
///
/// @hadris-spec ECMA-167:4/14.1
/// @hadris-compliance partial
/// @hadris-note The first descriptor of the file set sequence gives the root directory; later file sets and the system stream directory are not read.
/// @hadris-tests roundtrip::every_tree_reads_back, errors::malformed_volumes_are_refused
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FileSetDescriptor {
    /// The tag; identifier [`tag::FILE_SET`](super::tag::FILE_SET).
    pub tag: Tag,
    /// The recording date and time.
    pub recorded: Timestamp,
    /// The interchange level.
    pub interchange_level: U16Le,
    /// The maximum interchange level.
    pub max_interchange_level: U16Le,
    /// The character set list.
    pub character_set_list: U32Le,
    /// The maximum character set list.
    pub max_character_set_list: U32Le,
    /// The file set number.
    pub file_set_number: U32Le,
    /// The file set descriptor number.
    pub file_set_descriptor_number: U32Le,
    /// The logical volume identifier character set.
    pub logical_volume_character_set: CharSpec,
    /// The logical volume identifier, a 128-byte d-string.
    pub logical_volume_identifier: [u8; 128],
    /// The file set character set.
    pub file_set_character_set: CharSpec,
    /// The file set identifier, a 32-byte d-string.
    pub file_set_identifier: [u8; 32],
    /// The copyright file identifier.
    pub copyright_file: [u8; 32],
    /// The abstract file identifier.
    pub abstract_file: [u8; 32],
    /// The root directory ICB.
    pub root: LongAd,
    /// The domain identifier.
    pub domain: EntityId,
    /// The next extent of the file set descriptor sequence.
    pub next: LongAd,
    /// The system stream directory ICB.
    pub system_stream_directory: LongAd,
    /// Reserved.
    pub reserved: [u8; 32],
}

/// ICB tag (ECMA-167 4/14.6).
///
/// @hadris-spec ECMA-167:4/14.6
/// @hadris-compliance partial
/// @hadris-note Strategy 4 entries are read and written; the file type, allocation type and setuid, setgid and sticky flags are used, and indirect entries of strategy 4096 are not followed.
/// @hadris-tests roundtrip::every_tree_reads_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct IcbTag {
    /// The prior recorded number of direct entries.
    pub prior_entries: U32Le,
    /// The strategy type; 4 for UDF.
    pub strategy: U16Le,
    /// Strategy parameters.
    pub strategy_parameter: [u8; 2],
    /// The maximum number of entries.
    pub max_entries: U16Le,
    /// Reserved.
    pub reserved: u8,
    /// The file type, one of [`file_type`](super::file_type).
    pub file_type: u8,
    /// The parent ICB location.
    pub parent: LbAddr,
    /// [`IcbFlags`] bits.
    pub flags: U16Le,
}

bitflags::bitflags! {
    /// ICB tag flags (ECMA-167 4/14.6.8). The low three bits hold the
    /// allocation type, one of [`allocation`](super::allocation).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct IcbFlags: u16 {
        /// The allocation type bits.
        const ALLOCATION = 0x0007;
        /// Directory entries are sorted.
        const SORTED = 0x0008;
        /// The file may not be relocated.
        const NON_RELOCATABLE = 0x0010;
        /// The archive bit.
        const ARCHIVE = 0x0020;
        /// Set user id.
        const SETUID = 0x0040;
        /// Set group id.
        const SETGID = 0x0080;
        /// The sticky bit.
        const STICKY = 0x0100;
        /// The extents are contiguous.
        const CONTIGUOUS = 0x0200;
        /// A system file.
        const SYSTEM = 0x0400;
        /// The data is transformed.
        const TRANSFORMED = 0x0800;
        /// Multiple versions.
        const MULTI_VERSIONS = 0x1000;
        /// The file is a stream.
        const STREAM = 0x2000;
    }
}

bitflags::bitflags! {
    /// File permissions (ECMA-167 4/14.9.5): execute, write, read, change
    /// attributes and delete for others, the group and the owner.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Permissions: u32 {
        /// Others may execute.
        const OTHER_EXECUTE = 1 << 0;
        /// Others may write.
        const OTHER_WRITE = 1 << 1;
        /// Others may read.
        const OTHER_READ = 1 << 2;
        /// Others may change attributes.
        const OTHER_CHANGE_ATTRIBUTES = 1 << 3;
        /// Others may delete.
        const OTHER_DELETE = 1 << 4;
        /// The group may execute.
        const GROUP_EXECUTE = 1 << 5;
        /// The group may write.
        const GROUP_WRITE = 1 << 6;
        /// The group may read.
        const GROUP_READ = 1 << 7;
        /// The group may change attributes.
        const GROUP_CHANGE_ATTRIBUTES = 1 << 8;
        /// The group may delete.
        const GROUP_DELETE = 1 << 9;
        /// The owner may execute.
        const OWNER_EXECUTE = 1 << 10;
        /// The owner may write.
        const OWNER_WRITE = 1 << 11;
        /// The owner may read.
        const OWNER_READ = 1 << 12;
        /// The owner may change attributes.
        const OWNER_CHANGE_ATTRIBUTES = 1 << 13;
        /// The owner may delete.
        const OWNER_DELETE = 1 << 14;
    }
}

/// File Entry (ECMA-167 4/14.9); extended attributes and allocation
/// descriptors follow it.
///
/// @hadris-spec ECMA-167:4/14.9
/// @hadris-compliance partial
/// @hadris-note Read with the file type, size, owner, permissions, link count and times; written with short allocation descriptors of at most 1 GiB each.
/// @hadris-tests roundtrip::every_tree_reads_back, roundtrip::metadata_reads_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FileEntry {
    /// The tag; identifier [`tag::FILE_ENTRY`](super::tag::FILE_ENTRY).
    pub tag: Tag,
    /// The ICB tag.
    pub icb_tag: IcbTag,
    /// The owner's user id; `u32::MAX` when not specified.
    pub uid: U32Le,
    /// The owner's group id; `u32::MAX` when not specified.
    pub gid: U32Le,
    /// [`Permissions`] bits.
    pub permissions: U32Le,
    /// The number of identifiers naming the file.
    pub link_count: U16Le,
    /// The record format.
    pub record_format: u8,
    /// The record display attributes.
    pub record_display_attributes: u8,
    /// The record length.
    pub record_length: U32Le,
    /// The file size in bytes.
    pub information_length: U64Le,
    /// The number of recorded logical blocks.
    pub blocks_recorded: U64Le,
    /// The last access time.
    pub accessed: Timestamp,
    /// The last modification time.
    pub modified: Timestamp,
    /// The last attribute change time.
    pub attributes_changed: Timestamp,
    /// The checkpoint.
    pub checkpoint: U32Le,
    /// The extended attribute ICB.
    pub extended_attribute_icb: LongAd,
    /// The implementation identifier.
    pub implementation: EntityId,
    /// The unique id.
    pub unique_id: U64Le,
    /// The length of the extended attributes that follow.
    pub extended_attributes_length: U32Le,
    /// The length of the allocation descriptors after them.
    pub allocation_descriptors_length: U32Le,
}

/// Extended File Entry (ECMA-167 4/14.17); extended attributes and
/// allocation descriptors follow it.
///
/// @hadris-spec ECMA-167:4/14.17
/// @hadris-compliance partial
/// @hadris-note Read like a file entry, with the creation time; the stream directory is not read.
/// @hadris-tests read::extended_file_entries_read
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExtendedFileEntry {
    /// The tag; identifier [`tag::EXTENDED_FILE_ENTRY`](super::tag::EXTENDED_FILE_ENTRY).
    pub tag: Tag,
    /// The ICB tag.
    pub icb_tag: IcbTag,
    /// The owner's user id.
    pub uid: U32Le,
    /// The owner's group id.
    pub gid: U32Le,
    /// [`Permissions`] bits.
    pub permissions: U32Le,
    /// The number of identifiers naming the file.
    pub link_count: U16Le,
    /// The record format.
    pub record_format: u8,
    /// The record display attributes.
    pub record_display_attributes: u8,
    /// The record length.
    pub record_length: U32Le,
    /// The file size in bytes.
    pub information_length: U64Le,
    /// The size of all streams in bytes.
    pub object_size: U64Le,
    /// The number of recorded logical blocks.
    pub blocks_recorded: U64Le,
    /// The last access time.
    pub accessed: Timestamp,
    /// The last modification time.
    pub modified: Timestamp,
    /// The creation time.
    pub created: Timestamp,
    /// The last attribute change time.
    pub attributes_changed: Timestamp,
    /// The checkpoint.
    pub checkpoint: U32Le,
    /// Reserved.
    pub reserved: U32Le,
    /// The extended attribute ICB.
    pub extended_attribute_icb: LongAd,
    /// The stream directory ICB.
    pub stream_directory_icb: LongAd,
    /// The implementation identifier.
    pub implementation: EntityId,
    /// The unique id.
    pub unique_id: U64Le,
    /// The length of the extended attributes that follow.
    pub extended_attributes_length: U32Le,
    /// The length of the allocation descriptors after them.
    pub allocation_descriptors_length: U32Le,
}

bitflags::bitflags! {
    /// File characteristics of a file identifier (ECMA-167 4/14.4.3).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FileCharacteristics: u8 {
        /// Hidden from the user.
        const HIDDEN = 0x01;
        /// Names a directory.
        const DIRECTORY = 0x02;
        /// Deleted.
        const DELETED = 0x04;
        /// Names the parent directory.
        const PARENT = 0x08;
        /// Names metadata (UDF stream directories).
        const METADATA = 0x10;
    }
}

/// The fixed part of a File Identifier Descriptor (ECMA-167 4/14.4); the
/// implementation use, the identifier and padding to four bytes follow.
///
/// @hadris-spec ECMA-167:4/14.4
/// @hadris-compliance partial
/// @hadris-note Identifiers are read across block and extent boundaries; deleted and parent identifiers are skipped in listings, and the tag location is written as the block holding the first byte but not checked on read.
/// @hadris-tests roundtrip::every_tree_reads_back, read::identifiers_cross_extent_boundaries
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FileIdentifierDescriptor {
    /// The tag; identifier [`tag::FILE_IDENTIFIER`](super::tag::FILE_IDENTIFIER).
    pub tag: Tag,
    /// The file version number; one.
    pub version: U16Le,
    /// [`FileCharacteristics`] bits.
    pub characteristics: u8,
    /// The length of the identifier.
    pub identifier_length: u8,
    /// The ICB of the file.
    pub icb: LongAd,
    /// The length of the implementation use before the identifier.
    pub implementation_use_length: U16Le,
}

impl FileIdentifierDescriptor {
    /// The size of the whole descriptor, padded to four bytes.
    pub fn total_len(&self) -> usize {
        (core::mem::size_of::<Self>()
            + usize::from(self.implementation_use_length.get())
            + usize::from(self.identifier_length)
            + 3)
            & !3
    }
}

/// Allocation Extent Descriptor (ECMA-167 4/14.5): a block that continues
/// a list of allocation descriptors.
///
/// @hadris-spec ECMA-167:4/14.5
/// @hadris-compliance partial
/// @hadris-note Followed from continuation extents of every allocation descriptor form, with a bound on the chain length; not written.
/// @hadris-tests read::allocation_descriptors_of_every_form_read_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AllocationExtentDescriptor {
    /// The tag; identifier [`tag::ALLOCATION_EXTENT`](super::tag::ALLOCATION_EXTENT).
    pub tag: Tag,
    /// The previous allocation extent location.
    pub previous: U32Le,
    /// The length of the allocation descriptors that follow.
    pub allocation_descriptors_length: U32Le,
}

/// The fixed part of a Path Component (ECMA-167 4/14.16.1); a symbolic
/// link's data is a sequence of them.
///
/// @hadris-spec ECMA-167:4/14.16.1
/// @hadris-compliance partial
/// @hadris-note Root, parent, current and named components are read and written; component versions are ignored.
/// @hadris-tests name::tests::symlink_targets_round_trip
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathComponent {
    /// 1 root, 2 root of the volume, 3 parent, 4 current, 5 a name.
    pub component_type: u8,
    /// The length of the identifier that follows.
    pub identifier_length: u8,
    /// The component file version number.
    pub version: U16Le,
}
