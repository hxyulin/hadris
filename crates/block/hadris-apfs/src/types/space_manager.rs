//! APFS space manager summary and chunk-info parsing.
//!
//! This module parses `spaceman_phys_t` (`SpaceManagerBlockRaw`) headers plus
//! the `chunk_info_block_t` (`ChunkInfoBlockRaw`) arrays they reference, which
//! together describe free/used block ranges. Bitmap traversal (needed to find
//! the exact free bit positions within a chunk for allocation) is not
//! implemented yet.

use crate::types::le_u32;
use crate::types::le_u64;
use crate::types::object::{ObjectHeader, ObjectType};

/// Byte offset of the primary device's `spaceman_device_t` within
/// `spaceman_phys_t`.
const MAIN_DEVICE_OFFSET: usize = 48;
/// Byte offset of the tier-2 (Fusion) device's `spaceman_device_t`.
const TIER2_DEVICE_OFFSET: usize = MAIN_DEVICE_OFFSET + SPACE_MANAGER_DEVICE_SIZE;
/// Size in bytes of one `spaceman_device_t` entry.
const SPACE_MANAGER_DEVICE_SIZE: usize = 48;
/// Byte offset of `sm_block_count` within a `spaceman_device_t`.
const DEVICE_BLOCK_COUNT_OFFSET: usize = 0;
/// Byte offset of `sm_cib_count` within a `spaceman_device_t`.
const DEVICE_CIB_COUNT_OFFSET: usize = 16;
/// Byte offset of `sm_cab_count` within a `spaceman_device_t`.
const DEVICE_CAB_COUNT_OFFSET: usize = 20;
/// Byte offset of `sm_free_count` within a `spaceman_device_t`.
const DEVICE_FREE_COUNT_OFFSET: usize = 24;
/// Byte offset of `sm_addr_offset` within a `spaceman_device_t`.
const DEVICE_ADDRESS_OFFSET_OFFSET: usize = 32;

/// Size in bytes of one `chunk_info_t` entry.
#[cfg(any(feature = "alloc", feature = "std"))]
const CHUNK_INFO_SIZE: usize = 32;
/// Size in bytes of the `chunk_info_block_t` header before its entry array.
#[cfg(any(feature = "alloc", feature = "std"))]
const CHUNK_INFO_BLOCK_HEADER_SIZE: usize = 40;

/// Block usage summary for one space manager device slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceManagerDevice {
    /// Total physical blocks provided by this device.
    pub block_count: u64,
    /// Free (unallocated) physical blocks on this device.
    pub free_count: u64,
    /// Number of `chunk_info_block_t` blocks describing this device.
    pub chunk_info_block_count: u32,
    /// Number of `chunk_info_address_block_t` blocks describing this device,
    /// or zero when [`Self::chunk_info_block_count`] addresses are inline.
    pub chunk_info_address_block_count: u32,
    /// Byte offset from the start of the space manager block to the address
    /// array described by [`Self::chunk_info_block_count`] /
    /// [`Self::chunk_info_address_block_count`].
    pub address_offset: u32,
}

/// Minimal parsed space manager summary (`spaceman_phys_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceManagerSummary {
    /// Common object header.
    pub object: ObjectHeader,
    /// Container block size recorded by the space manager.
    pub block_size: u32,
    /// Main device block usage.
    pub main_device: SpaceManagerDevice,
    /// Tier-2 (Fusion) device block usage, when present.
    pub tier2_device: Option<SpaceManagerDevice>,
}

impl SpaceManagerSummary {
    /// Parses a space manager summary from a full APFS block.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let object = ObjectHeader::parse(data)?;
        if object.kind() != ObjectType::SpaceManager as u16 {
            return Err(crate::ApfsError::InvalidValue("space manager object type"));
        }
        let block_size = le_u32(data, 32)?;
        let main_device = parse_device(data, MAIN_DEVICE_OFFSET)?;
        let tier2_device = parse_device(data, TIER2_DEVICE_OFFSET).ok();
        Ok(Self {
            object,
            block_size,
            main_device,
            tier2_device: tier2_device.filter(|device| device.block_count != 0),
        })
    }

    /// Returns the physical block addresses of the main device's
    /// `chunk_info_block_t` blocks, when they're stored inline (no
    /// `chunk_info_address_block_t` indirection layer).
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub fn main_device_chunk_info_block_addresses(
        &self,
        data: &[u8],
    ) -> crate::Result<alloc::vec::Vec<u64>> {
        chunk_info_block_addresses(data, self.main_device)
    }
}

fn parse_device(data: &[u8], base: usize) -> crate::Result<SpaceManagerDevice> {
    Ok(SpaceManagerDevice {
        block_count: le_u64(data, base + DEVICE_BLOCK_COUNT_OFFSET)?,
        free_count: le_u64(data, base + DEVICE_FREE_COUNT_OFFSET)?,
        chunk_info_block_count: le_u32(data, base + DEVICE_CIB_COUNT_OFFSET)?,
        chunk_info_address_block_count: le_u32(data, base + DEVICE_CAB_COUNT_OFFSET)?,
        address_offset: le_u32(data, base + DEVICE_ADDRESS_OFFSET_OFFSET)?,
    })
}

#[cfg(any(feature = "alloc", feature = "std"))]
fn chunk_info_block_addresses(
    data: &[u8],
    device: SpaceManagerDevice,
) -> crate::Result<alloc::vec::Vec<u64>> {
    if device.chunk_info_address_block_count != 0 {
        return Err(crate::ApfsError::InvalidValue(
            "chunk info address block indirection is not supported yet",
        ));
    }
    let mut offset = device.address_offset as usize;
    let available = data
        .len()
        .checked_sub(offset)
        .ok_or(crate::ApfsError::InputTooSmall)?;
    if device.chunk_info_block_count as usize > available / 8 {
        return Err(crate::ApfsError::InputTooSmall);
    }
    let mut addresses = alloc::vec::Vec::with_capacity(device.chunk_info_block_count as usize);
    for _ in 0..device.chunk_info_block_count {
        addresses.push(le_u64(data, offset)?);
        offset = offset
            .checked_add(8)
            .ok_or(crate::ApfsError::InputTooSmall)?;
    }
    Ok(addresses)
}

/// One parsed `chunk_info_t` entry describing a range of blocks.
#[cfg(any(feature = "alloc", feature = "std"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkInfo {
    /// Starting physical block address described by this chunk.
    pub address: u64,
    /// Number of blocks / bits described by this chunk's bitmap.
    pub block_count: u32,
    /// Number of free (unset) bits in this chunk's bitmap.
    pub free_count: u32,
    /// Physical block holding this chunk's bitmap, or zero when the whole
    /// chunk is free and no bitmap is stored.
    pub bitmap_address: u64,
}

#[cfg(any(feature = "alloc", feature = "std"))]
impl ChunkInfo {
    const SIZE: usize = CHUNK_INFO_SIZE;

    fn parse(data: &[u8]) -> crate::Result<Self> {
        Ok(Self {
            address: le_u64(data, 8)?,
            block_count: le_u32(data, 16)?,
            free_count: le_u32(data, 20)?,
            bitmap_address: le_u64(data, 24)?,
        })
    }
}

/// Parses the `chunk_info_t` array out of a `chunk_info_block_t` block.
#[cfg(any(feature = "alloc", feature = "std"))]
pub fn parse_chunk_info_block(data: &[u8]) -> crate::Result<alloc::vec::Vec<ChunkInfo>> {
    let object = ObjectHeader::parse(data)?;
    if object.kind() != ObjectType::SpaceManagerChunkInformationBlock as u16 {
        return Err(crate::ApfsError::InvalidValue(
            "chunk info block object type",
        ));
    }
    let count = le_u32(data, 36)? as usize;
    let available = data
        .len()
        .checked_sub(CHUNK_INFO_BLOCK_HEADER_SIZE)
        .ok_or(crate::ApfsError::InputTooSmall)?;
    if count > available / ChunkInfo::SIZE {
        return Err(crate::ApfsError::InputTooSmall);
    }
    let mut entries = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        let start = i
            .checked_mul(ChunkInfo::SIZE)
            .and_then(|offset| CHUNK_INFO_BLOCK_HEADER_SIZE.checked_add(offset))
            .ok_or(crate::ApfsError::InputTooSmall)?;
        let end = start
            .checked_add(ChunkInfo::SIZE)
            .ok_or(crate::ApfsError::InputTooSmall)?;
        entries.push(ChunkInfo::parse(
            data.get(start..end)
                .ok_or(crate::ApfsError::InputTooSmall)?,
        )?);
    }
    Ok(entries)
}

#[cfg(all(test, any(feature = "alloc", feature = "std")))]
mod tests {
    use super::*;

    #[test]
    fn oversized_chunk_info_count_is_rejected() {
        let mut block = alloc::vec![0_u8; 4096];
        block[24..26]
            .copy_from_slice(&(ObjectType::SpaceManagerChunkInformationBlock as u16).to_le_bytes());
        block[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_chunk_info_block(&block).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
    }

    #[test]
    fn oversized_chunk_info_address_count_is_rejected() {
        let device = SpaceManagerDevice {
            block_count: 0,
            free_count: 0,
            chunk_info_block_count: u32::MAX,
            chunk_info_address_block_count: 0,
            address_offset: 4000,
        };
        assert_eq!(
            chunk_info_block_addresses(&[0_u8; 4096], device).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
    }
}
