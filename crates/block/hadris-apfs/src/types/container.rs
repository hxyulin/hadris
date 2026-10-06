//! APFS container superblock types.

use crate::types::common::Uuid;
use crate::types::object::{ObjectHeader, ObjectType};
use crate::types::{le_u32, le_u64, take};

/// Magic value in container superblocks (`NXSB`).
pub const CONTAINER_SUPERBLOCK_MAGIC: [u8; 4] = *b"NXSB";
/// Minimum APFS container block size.
pub const CONTAINER_MINIMUM_BLOCK_SIZE_BYTES: u32 = 4096;
/// Maximum APFS container block size.
pub const CONTAINER_MAXIMUM_BLOCK_SIZE_BYTES: u32 = 65536;
/// Maximum volume OID slots in the container superblock.
pub const CONTAINER_MAX_FILE_SYSTEMS: usize = 100;

/// Parsed APFS container superblock (`nx_superblock_t`).
/// @hadris-spec Apple-APFS:nx_superblock_t
/// @hadris-compliance partial
/// @hadris-note Parses checkpoint geometry and the software keylocker range; Fusion and other container extensions remain unsupported.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContainerSuperblock {
    /// Common object header.
    pub object: ObjectHeader,
    /// Container block size in bytes.
    pub block_size: u32,
    /// Number of blocks in the container.
    pub block_count: u64,
    /// Compatible feature flags.
    pub compatible_features: u64,
    /// Read-only compatible feature flags.
    pub readonly_compatible_features: u64,
    /// Incompatible feature flags.
    pub incompatible_features: u64,
    /// Container UUID.
    pub uuid: Uuid,
    /// Next object identifier.
    pub next_object_identifier: u64,
    /// Next transaction identifier.
    pub next_transaction_identifier: u64,
    /// Checkpoint descriptor area block count field.
    pub checkpoint_descriptor_area_block_count: u32,
    /// Checkpoint data area block count field.
    pub checkpoint_data_area_block_count: u32,
    /// Checkpoint descriptor area base block or tree OID.
    pub checkpoint_descriptor_area_block: u64,
    /// Checkpoint data area base block or tree OID.
    pub checkpoint_data_area_block: u64,
    /// First valid index in the checkpoint descriptor ring.
    pub checkpoint_descriptor_area_start_index: u32,
    /// Number of valid blocks in the checkpoint descriptor ring.
    pub checkpoint_descriptor_area_length: u32,
    /// Space manager ephemeral object identifier.
    pub space_manager_oid: u64,
    /// Container object map physical object identifier.
    pub object_map_oid: u64,
    /// Reaper ephemeral object identifier.
    pub reaper_oid: u64,
    /// Volume object identifiers from `nx_fs_oid`.
    pub volume_oids: [u64; CONTAINER_MAX_FILE_SYSTEMS],
    /// Container flags, including software encryption (`NX_CRYPTO_SW`).
    pub flags: u64,
    /// First physical APFS block of the container keybag.
    pub keylocker_start: u64,
    /// Number of APFS blocks occupied by the container keybag.
    pub keylocker_blocks: u64,
}

impl ContainerSuperblock {
    /// Parses a container superblock from bytes beginning at an APFS object.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let object = ObjectHeader::parse(data)?;
        if object.kind() != ObjectType::ContainerSuperblock as u16 {
            return Err(crate::ApfsError::InvalidValue("container object type"));
        }
        let magic = take::<4>(data, 32)?;
        if magic != CONTAINER_SUPERBLOCK_MAGIC {
            return Err(crate::ApfsError::InvalidMagic {
                expected: CONTAINER_SUPERBLOCK_MAGIC,
                actual: magic,
            });
        }
        let block_size = le_u32(data, 36)?;
        if !(CONTAINER_MINIMUM_BLOCK_SIZE_BYTES..=CONTAINER_MAXIMUM_BLOCK_SIZE_BYTES)
            .contains(&block_size)
        {
            return Err(crate::ApfsError::InvalidValue("container block size"));
        }

        let mut volume_oids = [0_u64; CONTAINER_MAX_FILE_SYSTEMS];
        let mut offset = 184;
        for oid in &mut volume_oids {
            *oid = le_u64(data, offset)?;
            offset += 8;
        }

        Ok(Self {
            object,
            block_size,
            block_count: le_u64(data, 40)?,
            compatible_features: le_u64(data, 48)?,
            readonly_compatible_features: le_u64(data, 56)?,
            incompatible_features: le_u64(data, 64)?,
            uuid: take(data, 72)?,
            next_object_identifier: le_u64(data, 88)?,
            next_transaction_identifier: le_u64(data, 96)?,
            checkpoint_descriptor_area_block_count: le_u32(data, 104)?,
            checkpoint_data_area_block_count: le_u32(data, 108)?,
            checkpoint_descriptor_area_block: le_u64(data, 112)?,
            checkpoint_data_area_block: le_u64(data, 120)?,
            checkpoint_descriptor_area_start_index: le_u32(data, 136)?,
            checkpoint_descriptor_area_length: le_u32(data, 140)?,
            space_manager_oid: le_u64(data, 152)?,
            object_map_oid: le_u64(data, 160)?,
            reaper_oid: le_u64(data, 168)?,
            volume_oids,
            flags: le_u64(data, 1264)?,
            keylocker_start: le_u64(data, 1296)?,
            keylocker_blocks: le_u64(data, 1304)?,
        })
    }

    /// Returns non-zero volume object identifiers.
    pub fn volumes(&self) -> impl Iterator<Item = u64> + '_ {
        self.volume_oids.iter().copied().filter(|oid| *oid != 0)
    }
}

/// One checkpoint map entry (`checkpoint_mapping_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CheckpointMapping {
    /// Mapped object's raw APFS object type.
    pub object_type: u32,
    /// Mapped object's raw APFS object subtype.
    pub object_subtype: u32,
    /// Object size in bytes.
    pub size: u32,
    /// Volume object identifier associated with this mapping, or zero.
    pub filesystem_identifier: u64,
    /// Ephemeral object identifier being mapped.
    pub container_identifier: u64,
    /// Physical APFS block address containing the object.
    pub address: u64,
}

impl CheckpointMapping {
    /// Parses a mapping from bytes.
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        Ok(Self {
            object_type: le_u32(data, 0)?,
            object_subtype: le_u32(data, 4)?,
            size: le_u32(data, 8)?,
            filesystem_identifier: le_u64(data, 16)?,
            container_identifier: le_u64(data, 24)?,
            address: le_u64(data, 32)?,
        })
    }

    /// Low 16-bit object kind.
    pub const fn kind(self) -> u16 {
        (self.object_type & 0xffff) as u16
    }
}

/// Parsed checkpoint map block (`checkpoint_map_phys_t`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CheckpointMapBlock {
    /// Common object header.
    pub object: ObjectHeader,
    /// Checkpoint map flags.
    pub flags: u32,
    /// Checkpoint mappings in this block.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub mappings: alloc::vec::Vec<CheckpointMapping>,
}

impl CheckpointMapBlock {
    /// Parses a checkpoint map block from an APFS object block.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let object = ObjectHeader::parse(data)?;
        if object.kind() != ObjectType::CheckpointMap as u16 {
            return Err(crate::ApfsError::InvalidValue("checkpoint map object type"));
        }
        let flags = le_u32(data, 32)?;
        let count = le_u32(data, 36)? as usize;
        if count
            > data
                .len()
                .checked_sub(40)
                .ok_or(crate::ApfsError::InputTooSmall)?
                / 40
        {
            return Err(crate::ApfsError::InputTooSmall);
        }
        let mut mappings = alloc::vec::Vec::with_capacity(count);
        let mut offset: usize = 40;
        for _ in 0..count {
            let end = offset
                .checked_add(40)
                .ok_or(crate::ApfsError::AddressOverflow)?;
            mappings.push(CheckpointMapping::parse(
                data.get(offset..end)
                    .ok_or(crate::ApfsError::InputTooSmall)?,
            )?);
            offset = end;
        }
        Ok(Self {
            object,
            flags,
            mappings,
        })
    }

    /// Whether this block is the final checkpoint map block in a chain.
    pub const fn is_last(&self) -> bool {
        self.flags & 1 != 0
    }
}

#[cfg(all(test, any(feature = "alloc", feature = "std")))]
mod tests {
    use super::*;

    #[test]
    fn oversized_checkpoint_mapping_count_is_rejected() {
        let mut block = alloc::vec![0_u8; 4096];
        block[24..26].copy_from_slice(&(ObjectType::CheckpointMap as u16).to_le_bytes());
        block[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            CheckpointMapBlock::parse(&block).unwrap_err(),
            crate::ApfsError::InputTooSmall
        );
    }
}
