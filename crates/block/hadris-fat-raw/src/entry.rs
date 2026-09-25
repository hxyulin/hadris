//! FAT12, FAT16 and FAT32 allocation-table entries.

/// The FAT variant of a volume, named by its allocation-table entry width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum FatKind {
    /// 12-bit entries, fewer than 4085 clusters.
    Fat12,
    /// 16-bit entries, fewer than 65525 clusters.
    Fat16,
    /// 28-bit entries.
    Fat32,
}

/// Why a FAT entry does not name a next cluster. These are the only two
/// cases, so the enum is exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    /// The entry holds the bad-cluster marker.
    Bad,
    /// The entry holds a cluster number outside `2..=max_cluster`.
    OutOfBounds(u32),
}

/// The first data cluster; clusters 0 and 1 are reserved.
pub const FIRST_DATA_CLUSTER: u32 = 2;
/// The most data clusters of a FAT12 volume; more make it FAT16.
pub const FAT12_MAX_CLUSTERS: u32 = 4084;
/// The most data clusters of a FAT16 volume; more make it FAT32.
pub const FAT16_MAX_CLUSTERS: u32 = 65524;
/// The most data clusters of a FAT32 volume, keeping cluster numbers below
/// the bad-cluster marker.
pub const FAT32_MAX_CLUSTERS: u32 = 0x0FFF_FFF5;

impl FatKind {
    /// Bits of an entry that hold the cluster value.
    pub const fn mask(self) -> u32 {
        match self {
            Self::Fat12 => 0x0FFF,
            Self::Fat16 => 0xFFFF,
            Self::Fat32 => 0x0FFF_FFFF,
        }
    }

    /// The end-of-chain marker written by Hadris, and the lowest value read
    /// as end of chain.
    pub const fn end_of_chain(self) -> u32 {
        match self {
            Self::Fat12 => 0x0FF8,
            Self::Fat16 => 0xFFF8,
            Self::Fat32 => 0x0FFF_FFF8,
        }
    }

    /// The bad-cluster marker.
    pub const fn bad_cluster(self) -> u32 {
        match self {
            Self::Fat12 => 0x0FF7,
            Self::Fat16 => 0xFFF7,
            Self::Fat32 => 0x0FFF_FFF7,
        }
    }

    /// Byte offset of `cluster`'s entry within one FAT copy.
    pub const fn entry_offset(self, cluster: u64) -> u64 {
        match self {
            Self::Fat12 => cluster * 3 / 2,
            Self::Fat16 => cluster * 2,
            Self::Fat32 => cluster * 4,
        }
    }

    /// Number of bytes read at [`entry_offset`](Self::entry_offset) to decode
    /// one entry.
    pub const fn entry_len(self) -> usize {
        match self {
            Self::Fat12 | Self::Fat16 => 2,
            Self::Fat32 => 4,
        }
    }

    /// The bit of FAT entry 1 that is set while the volume is cleanly
    /// unmounted, 0 on FAT12, which has none.
    pub const fn clean_bit(self) -> u32 {
        match self {
            Self::Fat12 => 0,
            Self::Fat16 => 0x8000,
            Self::Fat32 => 0x0800_0000,
        }
    }

    /// Whether a masked entry value ends a chain.
    pub const fn is_end_of_chain(self, value: u32) -> bool {
        value >= self.end_of_chain()
    }

    /// Whether a masked entry value marks a bad cluster.
    pub const fn is_bad(self, value: u32) -> bool {
        value == self.bad_cluster()
    }

    /// Decodes the stored entry of `cluster` from the [`entry_len`](Self::entry_len)
    /// bytes at its offset. FAT32's reserved high nibble is kept.
    ///
    /// # Panics
    ///
    /// When `bytes` is shorter than [`entry_len`](Self::entry_len).
    pub fn decode(self, cluster: u64, bytes: &[u8]) -> u32 {
        match self {
            Self::Fat12 if cluster % 2 == 0 => {
                u32::from(bytes[0]) | (u32::from(bytes[1] & 0x0F) << 8)
            }
            Self::Fat12 => (u32::from(bytes[0]) >> 4) | (u32::from(bytes[1]) << 4),
            Self::Fat16 => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
            Self::Fat32 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        }
    }

    /// Stores `value` as the entry of `cluster` into the bytes at its offset,
    /// keeping the neighbouring FAT12 nibble and FAT32's reserved high nibble.
    ///
    /// # Panics
    ///
    /// When `bytes` is shorter than [`entry_len`](Self::entry_len).
    pub fn encode(self, cluster: u64, value: u32, bytes: &mut [u8]) {
        match self {
            Self::Fat12 if cluster % 2 == 0 => {
                bytes[0] = value as u8;
                bytes[1] = (bytes[1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
            }
            Self::Fat12 => {
                bytes[0] = (bytes[0] & 0x0F) | ((value << 4) as u8);
                bytes[1] = (value >> 4) as u8;
            }
            Self::Fat16 => bytes[..2].copy_from_slice(&(value as u16).to_le_bytes()),
            Self::Fat32 => {
                let old = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let new = (old & !self.mask()) | (value & self.mask());
                bytes[..4].copy_from_slice(&new.to_le_bytes());
            }
        }
    }

    /// Bits of one entry on disk: 12, 16 or 32.
    pub const fn entry_bits(self) -> u64 {
        match self {
            Self::Fat12 => 12,
            Self::Fat16 => 16,
            Self::Fat32 => 32,
        }
    }

    /// Whether the first two stored entries of a FAT hold what the
    /// specification requires: the media byte in entry 0 and an
    /// end-of-chain marker in entry 1. The FAT16 and FAT32 dirty bits in
    /// entry 1 are ignored.
    pub const fn reserved_entries_valid(self, media: u8, first: u32, second: u32) -> bool {
        let mask = self.mask();
        let flags = match self {
            Self::Fat12 => 0,
            Self::Fat16 => 0xC000,
            Self::Fat32 => 0x0C00_0000,
        };
        (first & mask) == ((mask & !0xFF) | media as u32)
            && ((second & mask) | flags) >= self.end_of_chain()
    }

    /// Interprets a stored entry as the link to the next cluster: `None` at
    /// the end of the chain.
    pub const fn next(self, stored: u32, max_cluster: u32) -> Result<Option<u32>, ChainError> {
        let value = stored & self.mask();
        if self.is_end_of_chain(value) {
            Ok(None)
        } else if self.is_bad(value) {
            Err(ChainError::Bad)
        } else if value < FIRST_DATA_CLUSTER || value > max_cluster {
            Err(ChainError::OutOfBounds(value))
        } else {
            Ok(Some(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat12_packs_two_entries_in_three_bytes() {
        let mut fat = [0u8; 8];
        let kind = FatKind::Fat12;
        for (cluster, value) in [(2u64, 0xABCu32), (3, 0x123), (4, 0xFFF)] {
            let at = kind.entry_offset(cluster) as usize;
            kind.encode(cluster, value, &mut fat[at..at + 2]);
        }
        assert_eq!(fat[3..6], [0xBC, 0x3A, 0x12]);
        for (cluster, value) in [(2u64, 0xABCu32), (3, 0x123), (4, 0xFFF)] {
            let at = kind.entry_offset(cluster) as usize;
            assert_eq!(kind.decode(cluster, &fat[at..at + 2]), value);
        }
    }

    #[test]
    fn fat16_is_little_endian() {
        let mut bytes = [0u8; 2];
        FatKind::Fat16.encode(7, 0xFFF8, &mut bytes);
        assert_eq!(bytes, [0xF8, 0xFF]);
        assert_eq!(FatKind::Fat16.decode(7, &bytes), 0xFFF8);
        assert_eq!(FatKind::Fat16.entry_offset(7), 14);
    }

    #[test]
    fn fat32_keeps_reserved_nibble() {
        let mut bytes = 0xF000_0000u32.to_le_bytes();
        FatKind::Fat32.encode(9, 0xFFFF_1234, &mut bytes);
        assert_eq!(u32::from_le_bytes(bytes), 0xFFFF_1234);
        FatKind::Fat32.encode(9, 0x0000_0005, &mut bytes);
        assert_eq!(FatKind::Fat32.decode(9, &bytes), 0xF000_0005);
        assert_eq!(FatKind::Fat32.next(0xF000_0005, 10), Ok(Some(5)));
    }

    #[test]
    fn next_classifies_markers_and_bounds() {
        for kind in [FatKind::Fat12, FatKind::Fat16, FatKind::Fat32] {
            assert_eq!(kind.next(kind.end_of_chain(), 100), Ok(None));
            assert_eq!(kind.next(kind.mask(), 100), Ok(None));
            assert_eq!(kind.next(kind.bad_cluster(), 100), Err(ChainError::Bad));
            assert_eq!(kind.next(0, 100), Err(ChainError::OutOfBounds(0)));
            assert_eq!(kind.next(1, 100), Err(ChainError::OutOfBounds(1)));
            assert_eq!(kind.next(101, 100), Err(ChainError::OutOfBounds(101)));
            assert_eq!(kind.next(2, 100), Ok(Some(2)));
            assert_eq!(kind.next(100, 100), Ok(Some(100)));
        }
    }
}
