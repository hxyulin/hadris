use super::{CharSpec, EntityId, ExtentAd, Tag, Timestamp, U16Le, U32Le};

/// Volume Structure Descriptor (ECMA-167 2/9.1), one per 2048 bytes of
/// the Volume Recognition Sequence from byte 32768.
///
/// @hadris-spec ECMA-167:2/9.1
/// @hadris-compliance partial
/// @hadris-note BEA01, NSR02 or NSR03 and TEA01 are written, after ISO 9660 descriptors in a bridge volume; the reader requires an NSR descriptor inside an extended area.
/// @hadris-tests errors::malformed_volumes_are_refused, roundtrip::every_tree_reads_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolumeStructureDescriptor {
    /// The structure type; zero.
    pub structure_type: u8,
    /// The standard identifier, one of [`vsd`](super::vsd).
    pub identifier: [u8; 5],
    /// The structure version; one.
    pub version: u8,
    /// Structure data.
    pub data: [u8; 2041],
}

impl VolumeStructureDescriptor {
    /// A descriptor with the given identifier.
    pub fn new(identifier: [u8; 5]) -> Self {
        let mut vsd: Self = bytemuck::Zeroable::zeroed();
        vsd.identifier = identifier;
        vsd.version = 1;
        vsd
    }
}

/// Anchor Volume Descriptor Pointer (ECMA-167 3/10.2).
///
/// @hadris-spec ECMA-167:3/10.2
/// @hadris-compliance partial
/// @hadris-note Written at block 256 and at N-256; the reader looks at 256, N-256 and N-1 for each logical block size and falls back to the reserve sequence.
/// @hadris-tests roundtrip::every_tree_reads_back, errors::reserve_sequence_and_backup_anchor_are_used
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AnchorVolumeDescriptorPointer {
    /// The tag; identifier [`tag::ANCHOR`](super::tag::ANCHOR).
    pub tag: Tag,
    /// The Main Volume Descriptor Sequence.
    pub main: ExtentAd,
    /// The Reserve Volume Descriptor Sequence.
    pub reserve: ExtentAd,
    /// Reserved.
    pub reserved: [u8; 480],
}

/// Primary Volume Descriptor (ECMA-167 3/10.1).
///
/// @hadris-spec ECMA-167:3/10.1
/// @hadris-compliance partial
/// @hadris-note The prevailing descriptor by sequence number is used; its volume identifier is read, the other fields are written but not checked.
/// @hadris-tests roundtrip::every_tree_reads_back, read::prevailing_descriptors_win
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrimaryVolumeDescriptor {
    /// The tag; identifier [`tag::PRIMARY_VOLUME`](super::tag::PRIMARY_VOLUME).
    pub tag: Tag,
    /// The volume descriptor sequence number.
    pub sequence_number: U32Le,
    /// The primary volume descriptor number.
    pub number: U32Le,
    /// The volume identifier, a 32-byte d-string.
    pub volume_identifier: [u8; 32],
    /// The volume sequence number.
    pub volume_sequence_number: U16Le,
    /// The maximum volume sequence number.
    pub max_volume_sequence_number: U16Le,
    /// The interchange level.
    pub interchange_level: U16Le,
    /// The maximum interchange level.
    pub max_interchange_level: U16Le,
    /// The character set list.
    pub character_set_list: U32Le,
    /// The maximum character set list.
    pub max_character_set_list: U32Le,
    /// The volume set identifier, a 128-byte d-string.
    pub volume_set_identifier: [u8; 128],
    /// The descriptor character set.
    pub descriptor_character_set: CharSpec,
    /// The explanatory character set.
    pub explanatory_character_set: CharSpec,
    /// The volume abstract.
    pub volume_abstract: ExtentAd,
    /// The volume copyright notice.
    pub volume_copyright: ExtentAd,
    /// The application identifier.
    pub application: EntityId,
    /// The recording date and time.
    pub recorded: Timestamp,
    /// The implementation identifier.
    pub implementation: EntityId,
    /// Implementation use.
    pub implementation_use: [u8; 64],
    /// The predecessor volume descriptor sequence location.
    pub predecessor: U32Le,
    /// Flags.
    pub flags: U16Le,
    /// Reserved.
    pub reserved: [u8; 22],
}

/// Implementation Use Volume Descriptor (ECMA-167 3/10.4) with the UDF
/// Logical Volume Information in its implementation use area (UDF 2.2.7).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImplementationUseVolumeDescriptor {
    /// The tag; identifier [`tag::IMPLEMENTATION_USE`](super::tag::IMPLEMENTATION_USE).
    pub tag: Tag,
    /// The volume descriptor sequence number.
    pub sequence_number: U32Le,
    /// The implementation identifier; `*UDF LV Info` for UDF.
    pub implementation: EntityId,
    /// The logical volume information character set.
    pub lv_info_character_set: CharSpec,
    /// The logical volume identifier, a 128-byte d-string.
    pub logical_volume_identifier: [u8; 128],
    /// Three 36-byte d-strings of free text.
    pub lv_info: [[u8; 36]; 3],
    /// The implementation identifier of the writer.
    pub lv_implementation: EntityId,
    /// Implementation use.
    pub implementation_use: [u8; 128],
}

/// Partition Descriptor (ECMA-167 3/10.5).
///
/// @hadris-spec ECMA-167:3/10.5
/// @hadris-compliance partial
/// @hadris-note The prevailing descriptor for each partition number is used; the start and length bound every extent; contents other than NSR02 and NSR03 are refused.
/// @hadris-tests roundtrip::every_tree_reads_back, read::prevailing_descriptors_win
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PartitionDescriptor {
    /// The tag; identifier [`tag::PARTITION`](super::tag::PARTITION).
    pub tag: Tag,
    /// The volume descriptor sequence number.
    pub sequence_number: U32Le,
    /// Flags; bit 0 means the space is allocated.
    pub flags: U16Le,
    /// The partition number.
    pub number: U16Le,
    /// The contents: `+NSR02` or `+NSR03` for UDF.
    pub contents: EntityId,
    /// Contents use: the partition header descriptor.
    pub contents_use: [u8; 128],
    /// The access type: 1 read-only, 2 write-once, 3 rewritable, 4 overwritable.
    pub access_type: U32Le,
    /// The first sector.
    pub start: U32Le,
    /// The length in sectors.
    pub length: U32Le,
    /// The implementation identifier.
    pub implementation: EntityId,
    /// Implementation use.
    pub implementation_use: [u8; 128],
    /// Reserved.
    pub reserved: [u8; 156],
}

/// The fixed part of a Logical Volume Descriptor (ECMA-167 3/10.6); the
/// partition maps follow it.
///
/// @hadris-spec ECMA-167:3/10.6
/// @hadris-compliance partial
/// @hadris-note The logical block size must be 512 to 4096 bytes and agree with the anchor; type 1 partition maps are read and other map types are refused as unsupported.
/// @hadris-tests roundtrip::every_tree_reads_back, errors::unsupported_partition_maps_are_refused
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LogicalVolumeDescriptor {
    /// The tag; identifier [`tag::LOGICAL_VOLUME`](super::tag::LOGICAL_VOLUME).
    pub tag: Tag,
    /// The volume descriptor sequence number.
    pub sequence_number: U32Le,
    /// The descriptor character set.
    pub descriptor_character_set: CharSpec,
    /// The logical volume identifier, a 128-byte d-string.
    pub logical_volume_identifier: [u8; 128],
    /// The logical block size in bytes.
    pub block_size: U32Le,
    /// The domain identifier: `*OSTA UDF Compliant` with the UDF revision.
    pub domain: EntityId,
    /// Contents use: the long allocation descriptor of the file set
    /// descriptor sequence.
    pub contents_use: [u8; 16],
    /// The length of the partition maps in bytes.
    pub map_table_length: U32Le,
    /// The number of partition maps.
    pub map_count: U32Le,
    /// The implementation identifier.
    pub implementation: EntityId,
    /// Implementation use.
    pub implementation_use: [u8; 128],
    /// The logical volume integrity sequence.
    pub integrity_sequence: ExtentAd,
}

/// Type 1 partition map (ECMA-167 3/10.7.2): a partition of this volume.
///
/// @hadris-spec ECMA-167:3/10.7.2
/// @hadris-compliance partial
/// @hadris-note Up to eight type 1 maps are read; type 2 maps (virtual, sparable and metadata partitions) are refused as unsupported.
/// @hadris-tests read::prevailing_descriptors_win, errors::unsupported_partition_maps_are_refused
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Type1PartitionMap {
    /// The map type; one.
    pub map_type: u8,
    /// The map length; six.
    pub length: u8,
    /// The volume sequence number.
    pub volume_sequence_number: U16Le,
    /// The partition number of a partition descriptor.
    pub partition_number: U16Le,
}

/// The fixed part of an Unallocated Space Descriptor (ECMA-167 3/10.8);
/// extent descriptors follow it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UnallocatedSpaceDescriptor {
    /// The tag; identifier [`tag::UNALLOCATED_SPACE`](super::tag::UNALLOCATED_SPACE).
    pub tag: Tag,
    /// The volume descriptor sequence number.
    pub sequence_number: U32Le,
    /// The number of extent descriptors that follow.
    pub extent_count: U32Le,
}

/// Terminating Descriptor (ECMA-167 3/10.9, 4/14.2).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerminatingDescriptor {
    /// The tag; identifier [`tag::TERMINATING`](super::tag::TERMINATING).
    pub tag: Tag,
    /// Reserved.
    pub reserved: [u8; 496],
}

/// The fixed part of a Logical Volume Integrity Descriptor
/// (ECMA-167 3/10.10); the free space table, the size table and the
/// implementation use (UDF 2.2.6.4) follow it.
///
/// @hadris-spec ECMA-167:3/10.10
/// @hadris-compliance partial
/// @hadris-note Written closed with the next unique id, the partition size, and the file and directory counts; the reader takes the free space of a closed descriptor for the volume statistics.
/// @hadris-tests roundtrip::every_tree_reads_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LogicalVolumeIntegrityDescriptor {
    /// The tag; identifier [`tag::INTEGRITY`](super::tag::INTEGRITY).
    pub tag: Tag,
    /// The recording date and time.
    pub recorded: Timestamp,
    /// 0 open, 1 closed.
    pub integrity_type: U32Le,
    /// The next extent of the integrity sequence.
    pub next: ExtentAd,
    /// Contents use: the next unique id (UDF 3.2.1) in the first eight bytes.
    pub contents_use: [u8; 32],
    /// The number of partitions.
    pub partition_count: U32Le,
    /// The length of the implementation use area.
    pub implementation_use_length: U32Le,
}
