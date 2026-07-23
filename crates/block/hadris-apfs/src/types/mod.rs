//! APFS on-disk types and endian-aware parsers.

pub mod btree;
pub mod checksum;
pub mod common;
pub mod container;
pub mod filesystem;
pub mod object;
pub mod object_map;
pub mod space_manager;
pub mod volume;

pub use btree::{BTreeInfo, BTreeInfoFixed, BTreeNode, FixedEntry};
#[cfg(any(feature = "alloc", feature = "std"))]
pub use btree::{OwnedBTreeNode, OwnedEntry};
pub use common::{ObjectId, PhysicalAddress, TransactionId, Uuid};
pub use container::{
    CONTAINER_SUPERBLOCK_MAGIC, CheckpointMapBlock, CheckpointMapping, ContainerSuperblock,
};
#[cfg(any(feature = "alloc", feature = "std"))]
pub use filesystem::OwnedDirectoryEntryRecord;
pub use filesystem::{DirectoryEntryRecord, FileExtentRecord, FileSystemKey, InodeRecord};
pub use object::{ObjectHeader, ObjectType};
pub use object_map::{ObjectMapBlock, ObjectMapKey, ObjectMapValue};
#[cfg(any(feature = "alloc", feature = "std"))]
pub use space_manager::{ChunkInfo, parse_chunk_info_block};
pub use space_manager::{SpaceManagerDevice, SpaceManagerSummary};
pub use volume::{VOLUME_MAGIC, VolumeSuperblock};

pub(crate) fn take<const N: usize>(data: &[u8], offset: usize) -> crate::Result<[u8; N]> {
    let end = offset
        .checked_add(N)
        .ok_or(crate::ApfsError::AddressOverflow)?;
    let slice = data
        .get(offset..end)
        .ok_or(crate::ApfsError::InputTooSmall)?;
    Ok(slice.try_into().expect("fixed-size slice"))
}

pub(crate) fn le_u32(data: &[u8], offset: usize) -> crate::Result<u32> {
    Ok(u32::from_le_bytes(take(data, offset)?))
}
pub(crate) fn le_u64(data: &[u8], offset: usize) -> crate::Result<u64> {
    Ok(u64::from_le_bytes(take(data, offset)?))
}
