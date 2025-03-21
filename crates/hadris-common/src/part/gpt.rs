use crate::{
    str::utf16::FixedUtf16Str,
    types::{
        endian::{Endian, LittleEndian},
        number::{U32, U64},
    },
};

#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Guid([u8; 16]);

impl Default for Guid {
    fn default() -> Self {
        Self([0; 16])
    }
}

impl Guid {
    pub const BASIC_DATA_PART: Self = Self([
        0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99,
        0xc7,
    ]);
    pub const EFI_SYSTEM_PART: Self = Self([
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);
}

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
    pub current_lba: U32<LittleEndian>,
    pub backup_lba: U32<LittleEndian>,
    pub first_usable_lba: U32<LittleEndian>,
    pub last_usable_lba: U32<LittleEndian>,
    pub disk_guid: Guid,
    pub partition_entry_lba: U32<LittleEndian>,
    pub num_partition_entries: U32<LittleEndian>,
    /// The size of the size of each partition entry, in bytes.
    ///
    /// Must be a 128 * 2^n bytes
    pub size_of_partition_entry: U32<LittleEndian>,
    pub partition_entry_array_crc32: U32<LittleEndian>,
}

impl GptPartitionTableHeader {
    const SIGNATURE: [u8; 8] = *b"EFI PART";
}
impl Default for GptPartitionTableHeader {
    fn default() -> Self {
        Self {
            signature: Self::SIGNATURE,
            revision: U32::new(0x00010000),
            header_size: U32::new(0x5C),
            crc32: U32::new(0),
            reserved: U32::new(0),
            current_lba: U32::new(0),
            backup_lba: U32::new(0),
            first_usable_lba: U32::new(0),
            last_usable_lba: U32::new(0),
            disk_guid: Guid::default(),
            partition_entry_lba: U32::new(0),
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
    type_guid: Guid,
    unique_partition_guid: Guid,
    starting_lba: U64<LittleEndian>,
    ending_lba: U64<LittleEndian>,
    attributes: U64<LittleEndian>,
    partition_name: FixedUtf16Str<36>,
}
