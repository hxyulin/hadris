//! Shared APFS read helpers.

use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

use crate::types::container::ContainerSuperblock;

/// An opened APFS container's static metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    /// Block-zero container superblock.
    pub superblock: ContainerSuperblock,
}

impl ContainerInfo {
    /// Returns the APFS block geometry described by the container superblock.
    pub fn geometry(&self) -> crate::Result<BlockGeometry> {
        let block_size = BlockSize::new(self.superblock.block_size)
            .ok_or(crate::ApfsError::InvalidValue("container block size"))?;
        Ok(BlockGeometry::new(
            block_size,
            BlockCount(self.superblock.block_count),
        ))
    }
}
