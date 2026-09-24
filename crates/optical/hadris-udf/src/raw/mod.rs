//! On-disk layouts of ECMA-167 and the OSTA UDF specification.
//!
//! The items here mirror the specifications byte for byte. The module may
//! gain items; the existing ones follow the specifications and stay
//! exhaustive. The crate root never re-exports them. Tools such as
//! verifiers and dumpers read and write them directly; the rest of the
//! crate does not need them to walk a tree.
//!
//! Every multi-byte field is a little-endian byte array ([`U16Le`],
//! [`U32Le`], [`U64Le`]), so each layout has alignment one and can be read
//! from any offset of a buffer with [`bytemuck::pod_read_unaligned`].

use core::fmt;

mod descriptor;
mod file;

pub use descriptor::{
    AnchorVolumeDescriptorPointer, ImplementationUseVolumeDescriptor, LogicalVolumeDescriptor,
    LogicalVolumeIntegrityDescriptor, PartitionDescriptor, PrimaryVolumeDescriptor,
    TerminatingDescriptor, Type1PartitionMap, UnallocatedSpaceDescriptor,
    VolumeStructureDescriptor,
};
pub use file::{
    AllocationExtentDescriptor, ExtendedFileEntry, FileCharacteristics, FileEntry,
    FileIdentifierDescriptor, FileSetDescriptor, IcbFlags, IcbTag, PathComponent, Permissions,
};

/// The logical sector of the first Volume Structure Descriptor, in
/// 2048-byte units (byte 32768).
pub const VRS_START: u64 = 16;

/// The logical block of the first Anchor Volume Descriptor Pointer.
pub const ANCHOR_BLOCK: u32 = 256;

/// The Volume Structure Descriptor identifiers (ECMA-167 2/9.1, 3/9.1).
pub mod vsd {
    /// Beginning Extended Area Descriptor.
    pub const BEA01: [u8; 5] = *b"BEA01";
    /// NSR descriptor of ECMA-167 2nd edition (UDF 1.02 and 1.50).
    pub const NSR02: [u8; 5] = *b"NSR02";
    /// NSR descriptor of ECMA-167 3rd edition (UDF 2.00 and later).
    pub const NSR03: [u8; 5] = *b"NSR03";
    /// Terminating Extended Area Descriptor.
    pub const TEA01: [u8; 5] = *b"TEA01";
    /// An ISO 9660 volume descriptor.
    pub const CD001: [u8; 5] = *b"CD001";
    /// A boot descriptor.
    pub const BOOT2: [u8; 5] = *b"BOOT2";
}

/// Descriptor tag identifiers (ECMA-167 3/7.2.1, 4/7.2.1).
///
/// @hadris-spec ECMA-167:3/7.2.1
/// @hadris-compliance partial
/// @hadris-note Every identifier of ECMA-167 parts 3 and 4 is named; the reader checks the identifier each context requires.
/// @hadris-tests raw::tests::tag_seal_and_parse_roundtrip
/// @hadris-fuzz udf_read
pub mod tag {
    /// Primary Volume Descriptor.
    pub const PRIMARY_VOLUME: u16 = 1;
    /// Anchor Volume Descriptor Pointer.
    pub const ANCHOR: u16 = 2;
    /// Volume Descriptor Pointer.
    pub const VOLUME_POINTER: u16 = 3;
    /// Implementation Use Volume Descriptor.
    pub const IMPLEMENTATION_USE: u16 = 4;
    /// Partition Descriptor.
    pub const PARTITION: u16 = 5;
    /// Logical Volume Descriptor.
    pub const LOGICAL_VOLUME: u16 = 6;
    /// Unallocated Space Descriptor.
    pub const UNALLOCATED_SPACE: u16 = 7;
    /// Terminating Descriptor.
    pub const TERMINATING: u16 = 8;
    /// Logical Volume Integrity Descriptor.
    pub const INTEGRITY: u16 = 9;
    /// File Set Descriptor.
    pub const FILE_SET: u16 = 256;
    /// File Identifier Descriptor.
    pub const FILE_IDENTIFIER: u16 = 257;
    /// Allocation Extent Descriptor.
    pub const ALLOCATION_EXTENT: u16 = 258;
    /// Indirect Entry.
    pub const INDIRECT_ENTRY: u16 = 259;
    /// Terminal Entry.
    pub const TERMINAL_ENTRY: u16 = 260;
    /// File Entry.
    pub const FILE_ENTRY: u16 = 261;
    /// Extended Attribute Header Descriptor.
    pub const EXTENDED_ATTRIBUTE_HEADER: u16 = 262;
    /// Unallocated Space Entry.
    pub const UNALLOCATED_SPACE_ENTRY: u16 = 263;
    /// Space Bitmap Descriptor.
    pub const SPACE_BITMAP: u16 = 264;
    /// Partition Integrity Entry.
    pub const PARTITION_INTEGRITY: u16 = 265;
    /// Extended File Entry.
    pub const EXTENDED_FILE_ENTRY: u16 = 266;
}

/// ICB file types (ECMA-167 4/14.6.6, UDF 2.3.5.2).
pub mod file_type {
    /// Not specified.
    pub const UNSPECIFIED: u8 = 0;
    /// Unallocated Space Entry.
    pub const UNALLOCATED_SPACE: u8 = 1;
    /// Partition Integrity Entry.
    pub const PARTITION_INTEGRITY: u8 = 2;
    /// Indirect Entry.
    pub const INDIRECT: u8 = 3;
    /// A directory.
    pub const DIRECTORY: u8 = 4;
    /// A regular file.
    pub const FILE: u8 = 5;
    /// A block device.
    pub const BLOCK_DEVICE: u8 = 6;
    /// A character device.
    pub const CHAR_DEVICE: u8 = 7;
    /// An extended attribute record.
    pub const EXTENDED_ATTRIBUTES: u8 = 8;
    /// A FIFO.
    pub const FIFO: u8 = 9;
    /// A socket.
    pub const SOCKET: u8 = 10;
    /// Terminal Entry.
    pub const TERMINAL: u8 = 11;
    /// A symbolic link.
    pub const SYMLINK: u8 = 12;
    /// A stream directory.
    pub const STREAM_DIRECTORY: u8 = 13;
    /// A real-time file (UDF 2.3.5.2).
    pub const REAL_TIME_FILE: u8 = 249;
}

/// The allocation descriptor types in the low bits of the ICB flags
/// (ECMA-167 4/14.6.8).
pub mod allocation {
    /// Short allocation descriptors.
    pub const SHORT: u16 = 0;
    /// Long allocation descriptors.
    pub const LONG: u16 = 1;
    /// Extended allocation descriptors.
    pub const EXTENDED: u16 = 2;
    /// The data is recorded in the allocation descriptor area itself.
    pub const EMBEDDED: u16 = 3;
}

/// The extent types in the top two bits of an allocation descriptor's
/// length (ECMA-167 4/14.14.1.1).
pub mod extent {
    /// Recorded and allocated.
    pub const RECORDED: u8 = 0;
    /// Allocated and not recorded: reads as zeros.
    pub const ALLOCATED: u8 = 1;
    /// Neither allocated nor recorded: reads as zeros.
    pub const UNALLOCATED: u8 = 2;
    /// The next extent of allocation descriptors.
    pub const CONTINUATION: u8 = 3;
    /// The largest length an allocation descriptor can hold.
    pub const MAX_LENGTH: u32 = (1 << 30) - 1;
}

macro_rules! le_int {
    ($(#[$meta:meta])* $name:ident, $int:ty, $len:literal) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name([u8; $len]);

        impl $name {
            /// Encodes `value`.
            pub const fn new(value: $int) -> Self {
                Self(value.to_le_bytes())
            }

            /// Decodes the value.
            pub const fn get(self) -> $int {
                <$int>::from_le_bytes(self.0)
            }

            /// Replaces the value.
            pub fn set(&mut self, value: $int) {
                self.0 = value.to_le_bytes();
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.get(), f)
            }
        }
    };
}

le_int!(
    /// A little-endian 16-bit field (ECMA-167 1/7.1.3).
    U16Le, u16, 2
);
le_int!(
    /// A little-endian 32-bit field (ECMA-167 1/7.1.5).
    U32Le, u32, 4
);
le_int!(
    /// A little-endian 64-bit field (ECMA-167 1/7.1.7).
    U64Le, u64, 8
);

/// The CRC of a descriptor: CRC-CCITT with polynomial `0x1021` and
/// initial value zero (ECMA-167 3/7.2.6).
pub const fn crc16(data: &[u8]) -> u16 {
    crc16_update(0, data)
}

/// Continues [`crc16`] over more bytes.
pub const fn crc16_update(crc: u16, data: &[u8]) -> u16 {
    let mut crc = crc;
    let mut i = 0;
    while i < data.len() {
        let mut x = ((crc >> 8) ^ data[i] as u16) & 0xFF;
        x ^= x >> 4;
        crc = (crc << 8) ^ (x << 12) ^ (x << 5) ^ x;
        i += 1;
    }
    crc
}

/// Descriptor tag (ECMA-167 3/7.2).
///
/// @hadris-spec ECMA-167:3/7.2
/// @hadris-compliance partial
/// @hadris-note The checksum, CRC, identifier, version, reserved byte and location are checked for every volume, file set, file entry and allocation extent descriptor; identifier descriptors are not checked for their location.
/// @hadris-tests raw::tests::tag_seal_and_parse_roundtrip, errors::malformed_volumes_are_refused
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Tag {
    /// The descriptor type, one of [`tag`].
    pub identifier: U16Le,
    /// 2 for ECMA-167 2nd edition volumes (NSR02), 3 for 3rd edition (NSR03).
    pub version: U16Le,
    /// The modulo-256 sum of bytes 0 to 3 and 5 to 15.
    pub checksum: u8,
    /// Zero.
    pub reserved: u8,
    /// The serial number of the volume set.
    pub serial: U16Le,
    /// The CRC of the `crc_length` bytes after the tag.
    pub crc: U16Le,
    /// How many bytes after the tag the CRC covers.
    pub crc_length: U16Le,
    /// The block recording the descriptor: logical within its partition
    /// for file structures, absolute for volume structures.
    pub location: U32Le,
}

impl Tag {
    /// The size of a tag.
    pub const SIZE: usize = 16;

    /// The checksum of the first 16 bytes of `bytes`, skipping byte 4.
    pub fn checksum_of(bytes: &[u8; 16]) -> u8 {
        bytes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 4)
            .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte))
    }

    /// Reads a tag from the start of `bytes`, which holds at least 16 bytes.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        bytes.get(..Self::SIZE).map(bytemuck::pod_read_unaligned)
    }

    /// Whether the stored checksum matches.
    pub fn is_checksum_valid(&self) -> bool {
        Self::checksum_of(bytemuck::cast_ref(self)) == self.checksum
    }

    /// Whether `body`, the bytes after the tag, holds `crc_length` bytes
    /// whose CRC matches.
    pub fn is_crc_valid(&self, body: &[u8]) -> bool {
        body.get(..usize::from(self.crc_length.get()))
            .is_some_and(|covered| crc16(covered) == self.crc.get())
    }

    /// Writes a tag at the start of `descriptor`, with a CRC over the
    /// `crc_length` bytes after it, as many as `descriptor` holds, and the
    /// checksum. Does nothing when `descriptor` is shorter than a tag.
    pub fn seal(
        descriptor: &mut [u8],
        identifier: u16,
        version: u16,
        location: u32,
        crc_length: usize,
    ) {
        if descriptor.len() < Self::SIZE {
            return;
        }
        let crc_length = crc_length
            .min(descriptor.len() - Self::SIZE)
            .min(usize::from(u16::MAX));
        let mut tag = Tag {
            identifier: U16Le::new(identifier),
            version: U16Le::new(version),
            checksum: 0,
            reserved: 0,
            serial: U16Le::new(0),
            crc: U16Le::new(crc16(&descriptor[Self::SIZE..Self::SIZE + crc_length])),
            crc_length: U16Le::new(crc_length as u16),
            location: U32Le::new(location),
        };
        tag.checksum = Self::checksum_of(bytemuck::cast_ref(&tag));
        descriptor[..Self::SIZE].copy_from_slice(bytemuck::bytes_of(&tag));
    }
}

/// Extent descriptor (ECMA-167 3/7.1).
///
/// @hadris-spec ECMA-167:3/7.1
/// @hadris-compliance partial
/// @hadris-note Extents of the volume descriptor sequences and the integrity sequence are followed; their bounds are checked against the device.
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExtentAd {
    /// The length in bytes.
    pub length: U32Le,
    /// The first logical sector.
    pub location: U32Le,
}

/// Recorded address (ECMA-167 4/7.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LbAddr {
    /// The logical block within the partition.
    pub block: U32Le,
    /// The partition reference number: an index into the logical volume's
    /// partition maps.
    pub partition: U16Le,
}

/// Short allocation descriptor (ECMA-167 4/14.14.1): an extent in the
/// partition of the entry that holds it.
///
/// @hadris-spec ECMA-167:4/14.14.1
/// @hadris-compliance partial
/// @hadris-note The four extent types are read; allocated-not-recorded and unallocated extents read as zeros, and a continuation leads to an allocation extent descriptor.
/// @hadris-tests read::allocation_descriptors_of_every_form_read_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShortAd {
    /// The length in bytes in the low 30 bits, the extent type in the top two.
    pub length: U32Le,
    /// The first logical block.
    pub position: U32Le,
}

/// Long allocation descriptor (ECMA-167 4/14.14.2): an extent in any
/// partition of the logical volume.
///
/// @hadris-spec ECMA-167:4/14.14.2
/// @hadris-compliance partial
/// @hadris-note The partition reference is checked against the logical volume's partition maps; the implementation use bytes are written for UDF 2.00 and later and ignored on read.
/// @hadris-tests read::allocation_descriptors_of_every_form_read_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LongAd {
    /// The length in bytes in the low 30 bits, the extent type in the top two.
    pub length: U32Le,
    /// Where the extent starts.
    pub location: LbAddr,
    /// Implementation use; for UDF, flags and the UDF unique id.
    pub implementation_use: [u8; 6],
}

/// Extended allocation descriptor (ECMA-167 4/14.14.3).
///
/// @hadris-spec ECMA-167:4/14.14.3
/// @hadris-compliance partial
/// @hadris-note Read like long allocation descriptors; the recorded and information lengths are not used.
/// @hadris-tests read::allocation_descriptors_of_every_form_read_back
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExtAd {
    /// The length in bytes in the low 30 bits, the extent type in the top two.
    pub length: U32Le,
    /// The recorded length.
    pub recorded_length: U32Le,
    /// The information length.
    pub information_length: U32Le,
    /// Where the extent starts.
    pub location: LbAddr,
    /// Implementation use.
    pub implementation_use: [u8; 2],
}

macro_rules! ad_methods {
    ($($ty:ty),*) => {$(
        impl $ty {
            /// The length in bytes.
            pub fn len(&self) -> u32 {
                self.length.get() & extent::MAX_LENGTH
            }

            /// Whether the length is zero, which ends a list of descriptors.
            pub fn is_empty(&self) -> bool {
                self.len() == 0
            }

            /// The extent type, one of [`extent`].
            pub fn extent_type(&self) -> u8 {
                (self.length.get() >> 30) as u8
            }
        }
    )*};
}

ad_methods!(ShortAd, LongAd, ExtAd);

/// Entity identifier (ECMA-167 1/7.4, UDF 2.1.5).
///
/// @hadris-spec ECMA-167:1/7.4
/// @hadris-compliance partial
/// @hadris-note The domain identifier's revision suffix is read; other suffixes are written but not checked.
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EntityId {
    /// Flags: bit 0 dirty, bit 1 protected.
    pub flags: u8,
    /// The identifier, padded with zeros.
    pub identifier: [u8; 23],
    /// The identifier suffix.
    pub suffix: [u8; 8],
}

impl Default for EntityId {
    fn default() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

impl EntityId {
    /// The domain of UDF volumes.
    pub const OSTA_DOMAIN: &'static [u8] = b"*OSTA UDF Compliant";

    /// An identifier with an empty suffix.
    pub fn new(identifier: &[u8]) -> Self {
        let mut id = Self::default();
        let len = identifier.len().min(23);
        id.identifier[..len].copy_from_slice(&identifier[..len]);
        id
    }

    /// The identifier without its padding.
    pub fn name(&self) -> &[u8] {
        let end = self.identifier.iter().position(|&b| b == 0).unwrap_or(23);
        &self.identifier[..end]
    }

    /// The UDF revision a domain or UDF identifier suffix records.
    pub fn udf_revision(&self) -> u16 {
        u16::from_le_bytes([self.suffix[0], self.suffix[1]])
    }
}

/// Character set specification (ECMA-167 1/7.2.1).
///
/// @hadris-spec ECMA-167:1/7.2.1
/// @hadris-compliance partial
/// @hadris-note Written as OSTA Compressed Unicode, the only character set UDF allows; the reader does not check it.
/// @hadris-tests name::tests::cs0_round_trips_latin1_and_utf16
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CharSpec {
    /// The character set type; 0 for CS0.
    pub kind: u8,
    /// The character set information.
    pub info: [u8; 63],
}

impl CharSpec {
    /// OSTA Compressed Unicode (UDF 2.1.2).
    pub const OSTA: Self = {
        let mut info = [0; 63];
        let name = b"OSTA Compressed Unicode";
        let mut i = 0;
        while i < name.len() {
            info[i] = name[i];
            i += 1;
        }
        Self { kind: 0, info }
    };
}

impl Default for CharSpec {
    fn default() -> Self {
        Self::OSTA
    }
}

/// Timestamp (ECMA-167 1/7.3).
///
/// @hadris-spec ECMA-167:1/7.3
/// @hadris-compliance partial
/// @hadris-note Coordinated and local times with an offset convert to and from `hadris_fs::DateTime`; agreement times and times without an offset read as UTC.
/// @hadris-tests time::tests::timestamps_round_trip
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Timestamp {
    /// The type in the top four bits, the offset from UTC in minutes as a
    /// 12-bit signed value in the others.
    pub type_and_zone: U16Le,
    /// The year, 1 to 9999.
    pub year: U16Le,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day, 1 to 31.
    pub day: u8,
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute, 0 to 59.
    pub minute: u8,
    /// The second, 0 to 59.
    pub second: u8,
    /// Hundredths of a second.
    pub centiseconds: u8,
    /// Hundreds of microseconds.
    pub hundreds_of_microseconds: u8,
    /// Microseconds.
    pub microseconds: u8,
}

impl Timestamp {
    /// The offset value that means the zone is not specified.
    pub const NO_ZONE: i16 = -2047;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_have_their_specified_sizes() {
        use core::mem::size_of;
        assert_eq!(size_of::<Tag>(), 16);
        assert_eq!(size_of::<ExtentAd>(), 8);
        assert_eq!(size_of::<ShortAd>(), 8);
        assert_eq!(size_of::<LongAd>(), 16);
        assert_eq!(size_of::<ExtAd>(), 20);
        assert_eq!(size_of::<EntityId>(), 32);
        assert_eq!(size_of::<CharSpec>(), 64);
        assert_eq!(size_of::<Timestamp>(), 12);
        assert_eq!(size_of::<AnchorVolumeDescriptorPointer>(), 512);
        assert_eq!(size_of::<PrimaryVolumeDescriptor>(), 512);
        assert_eq!(size_of::<ImplementationUseVolumeDescriptor>(), 512);
        assert_eq!(size_of::<PartitionDescriptor>(), 512);
        assert_eq!(size_of::<LogicalVolumeDescriptor>(), 440);
        assert_eq!(size_of::<UnallocatedSpaceDescriptor>(), 24);
        assert_eq!(size_of::<TerminatingDescriptor>(), 512);
        assert_eq!(size_of::<LogicalVolumeIntegrityDescriptor>(), 80);
        assert_eq!(size_of::<FileSetDescriptor>(), 512);
        assert_eq!(size_of::<IcbTag>(), 20);
        assert_eq!(size_of::<FileEntry>(), 176);
        assert_eq!(size_of::<ExtendedFileEntry>(), 216);
        assert_eq!(size_of::<FileIdentifierDescriptor>(), 38);
        assert_eq!(size_of::<AllocationExtentDescriptor>(), 24);
        assert_eq!(size_of::<PathComponent>(), 4);
        assert_eq!(size_of::<VolumeStructureDescriptor>(), 2048);
        assert_eq!(size_of::<Type1PartitionMap>(), 6);
    }

    #[test]
    fn tag_seal_and_parse_roundtrip() {
        let mut descriptor = [0u8; 64];
        descriptor[16..20].copy_from_slice(b"body");
        Tag::seal(&mut descriptor, tag::PRIMARY_VOLUME, 2, 17, 48);
        let tag = Tag::read(&descriptor).unwrap();
        assert_eq!(tag.identifier.get(), tag::PRIMARY_VOLUME);
        assert_eq!(tag.location.get(), 17);
        assert!(tag.is_checksum_valid());
        assert!(tag.is_crc_valid(&descriptor[16..]));
        assert!(!tag.is_crc_valid(&descriptor[16..40]));

        let mut corrupt = descriptor;
        corrupt[17] ^= 1;
        assert!(!Tag::read(&corrupt).unwrap().is_crc_valid(&corrupt[16..]));
        corrupt = descriptor;
        corrupt[12] ^= 1;
        assert!(!Tag::read(&corrupt).unwrap().is_checksum_valid());
    }

    #[test]
    fn crc_matches_the_itu_reference() {
        assert_eq!(crc16(&[]), 0);
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn allocation_descriptor_lengths_split_type_bits() {
        let ad = ShortAd {
            length: U32Le::new((3 << 30) | 2048),
            position: U32Le::new(9),
        };
        assert_eq!((ad.len(), ad.extent_type()), (2048, extent::CONTINUATION));
        assert!(ShortAd::default().is_empty());
    }
}
