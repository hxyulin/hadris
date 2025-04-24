use std::fmt::{Debug, Display};

use crate::{
    str::utf16::FixedUtf16Str,
    types::{
        endian::{Endian, LittleEndian},
        number::{U32, U64},
    },
};

/// A 128-bit unique identifier (GUID)
/// This is the in the format:
/// u32 - time_low
/// u16 - time_mid
/// u16 - time_hi_and_version
/// u8 - clock_seq_hi_and_reserved
/// u8 - clock_seq_low
/// [u8; 6] - node (should be displayed in Big Endian)
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Guid([u8; 16]);

impl Guid {
    /// Generate a new GUID using the v4 algorithm.
    pub fn generate_v4() -> Self {
        use rand::RngCore;

        let mut bytes = [0u8; 16];
        rand::rng().fill_bytes(&mut bytes);

        // Set version: 0100xxxx (version 4)
        bytes[6] = (bytes[6] & 0x0F) | 0x40;

        // Set variant: 10xxxxxx
        bytes[8] = (bytes[8] & 0x3F) | 0x80;

        Self(bytes)
    }
}

impl Debug for Guid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Guid({})", self)
    }
}

impl Display for Guid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d1 = u32::from_le_bytes(self.0[0..4].try_into().unwrap());
        let d2 = u16::from_le_bytes(self.0[4..6].try_into().unwrap());
        let d3 = u16::from_le_bytes(self.0[6..8].try_into().unwrap());
        let d4 = &self.0[8..10];
        let d5 = &self.0[10..16];

        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            d1, d2, d3, d4[0], d4[1], d5[0], d5[1], d5[2], d5[3], d5[4], d5[5]
        )
    }
}

impl Default for Guid {
    fn default() -> Self {
        Self([0; 16])
    }
}

impl Guid {
    /// The GUID for the basic data partition
    pub const BASIC_DATA_PART: Self = Self([
        0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99,
        0xc7,
    ]);
    
    /// The GUID for the EFI system partition
    pub const EFI_SYSTEM_PART: Self = Self([
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);
}

/// The GPT header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct GptPartitionTableHeader {
    /// The signature for the GPT header, must be "EFI PART".
    pub signature: [u8; 8],
    pub revision: U32<LittleEndian>,
    pub header_size: U32<LittleEndian>,
    pub crc32: U32<LittleEndian>,
    pub reserved: U32<LittleEndian>,
    pub current_lba: U64<LittleEndian>,
    pub backup_lba: U64<LittleEndian>,
    pub first_usable_lba: U64<LittleEndian>,
    pub last_usable_lba: U64<LittleEndian>,
    pub disk_guid: Guid,
    pub partition_entry_lba: U64<LittleEndian>,
    pub num_partition_entries: U32<LittleEndian>,
    /// The size of the size of each partition entry, in bytes.
    ///
    /// Must be a 128 * 2^n bytes
    pub size_of_partition_entry: U32<LittleEndian>,
    pub partition_entry_array_crc32: U32<LittleEndian>,
}

impl GptPartitionTableHeader {
    /// The signature for the GPT header, must be "EFI PART".
    const SIGNATURE: [u8; 8] = *b"EFI PART";

    /// Check if the GPT header is valid, by checking the signature.
    pub fn is_valid(&self) -> bool {
        self.signature == Self::SIGNATURE
    }

    /// Generate the CRC32 checksum for the GPT header, discarding the current checksum.
    pub fn generate_crc32(&mut self) {
        use crate::alg::hash::crc::Crc32HasherIsoHdlc;
        self.crc32.set(0);
        let checksum = Crc32HasherIsoHdlc::checksum(bytemuck::bytes_of(self));
        self.crc32.set(checksum);
    }
}

impl Default for GptPartitionTableHeader {
    fn default() -> Self {
        Self {
            signature: Self::SIGNATURE,
            revision: U32::new(0x00010000),
            header_size: U32::new(0x5C),
            crc32: U32::new(0),
            reserved: U32::new(0),
            current_lba: U64::new(0),
            backup_lba: U64::new(0),
            first_usable_lba: U64::new(0),
            last_usable_lba: U64::new(0),
            disk_guid: Guid::default(),
            partition_entry_lba: U64::new(0),
            num_partition_entries: U32::new(0),
            size_of_partition_entry: U32::new(128),
            partition_entry_array_crc32: U32::new(0),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct GptPartitionEntry {
    pub type_guid: Guid,
    pub unique_partition_guid: Guid,
    pub starting_lba: U64<LittleEndian>,
    pub ending_lba: U64<LittleEndian>,
    pub attributes: U64<LittleEndian>,
    pub partition_name: FixedUtf16Str<36>,
}

impl GptPartitionEntry {
    pub fn is_empty(&self) -> bool {
        self.type_guid == Guid::default()
    }
}
