use core::num::NonZeroU32;

/// Size of one logical block in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockSize(NonZeroU32);

impl BlockSize {
    /// Creates a non-zero logical block size.
    pub const fn new(bytes: u32) -> Option<Self> {
        match NonZeroU32::new(bytes) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }

    /// Returns the size in bytes.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Zero-based logical block index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockIndex(pub u64);

/// Number of logical blocks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockCount(pub u64);

/// A contiguous logical-block range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockRange {
    /// First logical block in the range.
    pub start: BlockIndex,
    /// Number of logical blocks in the range.
    pub count: BlockCount,
}

impl BlockRange {
    /// Creates a block range.
    pub const fn new(start: BlockIndex, count: BlockCount) -> Self {
        Self { start, count }
    }

    /// Returns the exclusive end block, or `None` on overflow.
    pub const fn end(self) -> Option<BlockIndex> {
        match self.start.0.checked_add(self.count.0) {
            Some(end) => Some(BlockIndex(end)),
            None => None,
        }
    }
}

/// Geometry reported by a block device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockGeometry {
    /// Addressable logical block size.
    pub logical_block_size: BlockSize,
    /// Total number of addressable logical blocks.
    pub block_count: BlockCount,
    /// Physical block size used for alignment hints, when known.
    pub physical_block_size: Option<BlockSize>,
}

impl BlockGeometry {
    /// Creates geometry with no separate physical-block hint.
    pub const fn new(logical_block_size: BlockSize, block_count: BlockCount) -> Self {
        Self {
            logical_block_size,
            block_count,
            physical_block_size: None,
        }
    }

    /// Adds a physical-block alignment hint.
    pub const fn with_physical_block_size(mut self, physical: BlockSize) -> Self {
        self.physical_block_size = Some(physical);
        self
    }

    /// Returns the total byte length, or `None` if it cannot fit in `u64`.
    pub const fn byte_len(self) -> Option<u64> {
        self.block_count
            .0
            .checked_mul(self.logical_block_size.get() as u64)
    }

    /// Validates that a range lies within the geometry.
    pub const fn contains(self, range: BlockRange) -> bool {
        match range.end() {
            Some(end) => end.0 <= self.block_count.0,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_block_size() {
        assert_eq!(BlockSize::new(0), None);
    }

    #[test]
    fn checks_range_and_byte_overflow() {
        let geometry = BlockGeometry::new(BlockSize::new(512).unwrap(), BlockCount(8));
        assert!(geometry.contains(BlockRange::new(BlockIndex(6), BlockCount(2))));
        assert!(!geometry.contains(BlockRange::new(BlockIndex(7), BlockCount(2))));
        assert_eq!(geometry.byte_len(), Some(4096));
    }
}
