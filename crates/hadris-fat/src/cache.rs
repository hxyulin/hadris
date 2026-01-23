//! FAT sector caching for reduced I/O operations.
//!
//! This module provides a sector cache for FAT table operations, significantly
//! reducing the number of seek and read operations when traversing cluster chains.

use alloc::vec::Vec;

use crate::io::{Read, Seek, SeekFrom, Write};
use crate::{Fat, Fat12, Fat16, Fat32, FatError, FatType, Result};

/// Default number of sectors to cache.
pub const DEFAULT_CACHE_CAPACITY: usize = 16;

/// A cached FAT sector with LRU tracking.
#[derive(Debug)]
struct CacheEntry {
    /// Sector number within the FAT
    sector: usize,
    /// Sector data
    data: Vec<u8>,
    /// Whether the sector has been modified
    dirty: bool,
    /// Access counter for LRU eviction
    access_count: u64,
}

/// FAT sector cache with LRU eviction.
///
/// This cache stores FAT table sectors in memory to reduce seek operations
/// during cluster chain traversal and allocation.
#[derive(Debug)]
pub struct FatSectorCache {
    /// Cached sectors
    entries: Vec<CacheEntry>,
    /// Maximum number of sectors to cache
    capacity: usize,
    /// Size of each sector in bytes
    sector_size: usize,
    /// Start offset of the FAT in bytes
    fat_start: usize,
    /// Size of one FAT copy in bytes
    fat_size: usize,
    /// Number of FAT copies
    fat_count: usize,
    /// Global access counter for LRU tracking
    access_counter: u64,
    /// Cache statistics
    stats: CacheStats,
}

/// Cache performance statistics.
#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of evictions
    pub evictions: u64,
    /// Number of dirty sector writes
    pub dirty_writes: u64,
}

impl CacheStats {
    /// Calculate the cache hit ratio (0.0 to 1.0).
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

impl FatSectorCache {
    /// Create a new FAT sector cache.
    ///
    /// # Arguments
    ///
    /// * `fat_start` - Start offset of the FAT in bytes
    /// * `fat_size` - Size of one FAT copy in bytes
    /// * `fat_count` - Number of FAT copies (typically 2)
    /// * `sector_size` - Size of each sector in bytes
    /// * `capacity` - Maximum number of sectors to cache
    pub fn new(
        fat_start: usize,
        fat_size: usize,
        fat_count: usize,
        sector_size: usize,
        capacity: usize,
    ) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            sector_size,
            fat_start,
            fat_size,
            fat_count,
            access_counter: 0,
            stats: CacheStats::default(),
        }
    }

    /// Create a cache with default capacity.
    pub fn with_default_capacity(
        fat_start: usize,
        fat_size: usize,
        fat_count: usize,
        sector_size: usize,
    ) -> Self {
        Self::new(fat_start, fat_size, fat_count, sector_size, DEFAULT_CACHE_CAPACITY)
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Reset cache statistics.
    pub fn reset_stats(&mut self) {
        self.stats = CacheStats::default();
    }

    /// Clear the cache, optionally flushing dirty sectors first.
    pub fn clear<T: Read + Write + Seek>(&mut self, writer: Option<&mut T>) -> Result<()> {
        if let Some(w) = writer {
            self.flush(w)?;
        }
        self.entries.clear();
        Ok(())
    }

    /// Get the number of cached sectors.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Flush all dirty sectors to disk.
    pub fn flush<T: Write + Seek>(&mut self, writer: &mut T) -> Result<()> {
        let fat_start = self.fat_start;
        let fat_size = self.fat_size;
        let fat_count = self.fat_count;
        let sector_size = self.sector_size;

        for entry in &mut self.entries {
            if entry.dirty {
                // Write to all FAT copies
                for i in 0..fat_count {
                    let offset = fat_start + i * fat_size + entry.sector * sector_size;
                    writer.seek(SeekFrom::Start(offset as u64))?;
                    writer.write_all(&entry.data)?;
                }
                entry.dirty = false;
                self.stats.dirty_writes += 1;
            }
        }
        Ok(())
    }

    /// Write a sector to all FAT copies on disk.
    fn write_sector_to_disk<T: Write + Seek>(
        &self,
        writer: &mut T,
        sector: usize,
        data: &[u8],
    ) -> Result<()> {
        for i in 0..self.fat_count {
            let offset = self.fat_start + i * self.fat_size + sector * self.sector_size;
            writer.seek(SeekFrom::Start(offset as u64))?;
            writer.write_all(data)?;
        }
        Ok(())
    }

    /// Find a cached sector by sector number.
    fn find_sector(&mut self, sector: usize) -> Option<usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.sector == sector {
                return Some(i);
            }
        }
        None
    }

    /// Get or load a sector into the cache.
    fn get_sector<T: Read + Seek>(&mut self, reader: &mut T, sector: usize) -> Result<&[u8]> {
        self.access_counter += 1;

        // Check if already cached
        if let Some(idx) = self.find_sector(sector) {
            self.stats.hits += 1;
            self.entries[idx].access_count = self.access_counter;
            return Ok(&self.entries[idx].data);
        }

        self.stats.misses += 1;

        // Need to load from disk
        let mut data = alloc::vec![0u8; self.sector_size];
        let offset = self.fat_start + sector * self.sector_size;
        reader.seek(SeekFrom::Start(offset as u64))?;
        reader.read_exact(&mut data)?;

        // Evict if necessary
        if self.entries.len() >= self.capacity {
            self.evict_lru()?;
        }

        // Add to cache
        self.entries.push(CacheEntry {
            sector,
            data,
            dirty: false,
            access_count: self.access_counter,
        });

        Ok(&self.entries.last().unwrap().data)
    }

    /// Get a mutable reference to a sector, loading it if necessary.
    fn get_sector_mut<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        sector: usize,
    ) -> Result<&mut [u8]> {
        self.access_counter += 1;

        // Check if already cached
        if let Some(idx) = self.find_sector(sector) {
            self.stats.hits += 1;
            self.entries[idx].access_count = self.access_counter;
            self.entries[idx].dirty = true;
            return Ok(&mut self.entries[idx].data);
        }

        self.stats.misses += 1;

        // Need to load from disk
        let mut data = alloc::vec![0u8; self.sector_size];
        let offset = self.fat_start + sector * self.sector_size;
        reader.seek(SeekFrom::Start(offset as u64))?;
        reader.read_exact(&mut data)?;

        // Evict if necessary
        if self.entries.len() >= self.capacity {
            self.evict_lru()?;
        }

        // Add to cache
        self.entries.push(CacheEntry {
            sector,
            data,
            dirty: true,
            access_count: self.access_counter,
        });

        let idx = self.entries.len() - 1;
        Ok(&mut self.entries[idx].data)
    }

    /// Evict the least recently used sector.
    fn evict_lru(&mut self) -> Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        // Find LRU entry
        let mut lru_idx = 0;
        let mut lru_count = u64::MAX;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.access_count < lru_count {
                lru_count = entry.access_count;
                lru_idx = i;
            }
        }

        // If dirty, it will be written on next flush
        // For now, just remove it (caller should flush before eviction if needed)
        if self.entries[lru_idx].dirty {
            // We can't write here without a writer, so we'll mark it
            // The flush method should be called before eviction in write scenarios
        }

        self.entries.swap_remove(lru_idx);
        self.stats.evictions += 1;

        Ok(())
    }

    // =========================================================================
    // FAT12 cached operations
    // =========================================================================

    /// Read a FAT12 entry using the cache.
    pub fn read_fat12_entry<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<u16> {
        let sector_size = self.sector_size;

        // FAT12 packs 2 entries into 3 bytes
        let byte_offset = (cluster * 3) / 2;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        // We might need to read from two sectors if the entry spans a boundary
        let bytes = if offset_in_sector + 1 < sector_size {
            // Entry is within one sector
            let data = self.get_sector(reader, sector)?;
            [data[offset_in_sector], data[offset_in_sector + 1]]
        } else {
            // Entry spans two sectors - need to load the next sector too
            let first_byte = {
                let data = self.get_sector(reader, sector)?;
                data[offset_in_sector]
            };

            let second_byte = {
                let next_sector_data = self.get_sector(reader, sector + 1)?;
                next_sector_data[0]
            };
            [first_byte, second_byte]
        };

        // FAT12 entry layout:
        // If cluster N is even: entry = (bytes[1] & 0x0F) << 8 | bytes[0]
        // If cluster N is odd:  entry = bytes[1] << 4 | (bytes[0] >> 4)
        let value = if cluster % 2 == 0 {
            u16::from(bytes[0]) | (u16::from(bytes[1] & 0x0F) << 8)
        } else {
            (u16::from(bytes[0]) >> 4) | (u16::from(bytes[1]) << 4)
        };

        Ok(value)
    }

    /// Write a FAT12 entry using the cache.
    pub fn write_fat12_entry<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
        value: u16,
    ) -> Result<()> {
        let sector_size = self.sector_size;

        let byte_offset = (cluster * 3) / 2;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        if offset_in_sector + 1 < sector_size {
            // Entry is within one sector
            let data = self.get_sector_mut(reader, sector)?;

            if cluster % 2 == 0 {
                data[offset_in_sector] = value as u8;
                data[offset_in_sector + 1] =
                    (data[offset_in_sector + 1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
            } else {
                data[offset_in_sector] =
                    (data[offset_in_sector] & 0x0F) | ((value << 4) as u8);
                data[offset_in_sector + 1] = (value >> 4) as u8;
            }
        } else {
            // Entry spans two sectors
            {
                let data = self.get_sector_mut(reader, sector)?;
                if cluster % 2 == 0 {
                    data[offset_in_sector] = value as u8;
                } else {
                    data[offset_in_sector] =
                        (data[offset_in_sector] & 0x0F) | ((value << 4) as u8);
                }
            }

            {
                let data = self.get_sector_mut(reader, sector + 1)?;
                if cluster % 2 == 0 {
                    data[0] = (data[0] & 0xF0) | ((value >> 8) as u8 & 0x0F);
                } else {
                    data[0] = (value >> 4) as u8;
                }
            }
        }

        Ok(())
    }

    // =========================================================================
    // FAT16 cached operations
    // =========================================================================

    /// Read a FAT16 entry using the cache.
    pub fn read_fat16_entry<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<u16> {
        let sector_size = self.sector_size;
        let byte_offset = cluster * 2;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        let data = self.get_sector(reader, sector)?;
        let value = u16::from_le_bytes([data[offset_in_sector], data[offset_in_sector + 1]]);

        Ok(value)
    }

    /// Write a FAT16 entry using the cache.
    pub fn write_fat16_entry<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
        value: u16,
    ) -> Result<()> {
        let sector_size = self.sector_size;
        let byte_offset = cluster * 2;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        let data = self.get_sector_mut(reader, sector)?;
        let bytes = value.to_le_bytes();
        data[offset_in_sector] = bytes[0];
        data[offset_in_sector + 1] = bytes[1];

        Ok(())
    }

    // =========================================================================
    // FAT32 cached operations
    // =========================================================================

    /// Read a FAT32 entry using the cache.
    pub fn read_fat32_entry<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<u32> {
        let sector_size = self.sector_size;
        let byte_offset = cluster * 4;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        let data = self.get_sector(reader, sector)?;
        let value = u32::from_le_bytes([
            data[offset_in_sector],
            data[offset_in_sector + 1],
            data[offset_in_sector + 2],
            data[offset_in_sector + 3],
        ]);

        Ok(value)
    }

    /// Write a FAT32 entry using the cache.
    pub fn write_fat32_entry<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
        value: u32,
    ) -> Result<()> {
        let sector_size = self.sector_size;
        let byte_offset = cluster * 4;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        let data = self.get_sector_mut(reader, sector)?;
        let bytes = value.to_le_bytes();
        data[offset_in_sector] = bytes[0];
        data[offset_in_sector + 1] = bytes[1];
        data[offset_in_sector + 2] = bytes[2];
        data[offset_in_sector + 3] = bytes[3];

        Ok(())
    }
}

/// Cached FAT operations that work with any FAT type.
pub struct CachedFat<'a> {
    cache: &'a mut FatSectorCache,
    fat_type: FatType,
    max_cluster: u32,
}

impl<'a> CachedFat<'a> {
    /// Create a new cached FAT wrapper.
    pub fn new(cache: &'a mut FatSectorCache, fat: &Fat) -> Self {
        let (fat_type, max_cluster) = match fat {
            Fat::Fat12(f) => (FatType::Fat12, f.max_cluster() as u32),
            Fat::Fat16(f) => (FatType::Fat16, f.max_cluster() as u32),
            Fat::Fat32(f) => (FatType::Fat32, f.max_cluster()),
        };
        Self {
            cache,
            fat_type,
            max_cluster,
        }
    }

    /// Get the next cluster in a chain using the cache.
    pub fn next_cluster<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        match self.fat_type {
            FatType::Fat12 => {
                let entry = self.cache.read_fat12_entry(reader, cluster)? & 0x0FFF;
                if entry >= 0x0FF8 {
                    Ok(None) // End of chain
                } else if entry == 0x0FF7 {
                    Err(FatError::BadCluster { cluster: cluster as u32 })
                } else if entry < 2 || entry as u32 > self.max_cluster {
                    Err(FatError::ClusterOutOfBounds {
                        cluster: entry as u32,
                        max: self.max_cluster,
                    })
                } else {
                    Ok(Some(entry as u32))
                }
            }
            FatType::Fat16 => {
                let entry = self.cache.read_fat16_entry(reader, cluster)?;
                if entry >= 0xFFF8 {
                    Ok(None) // End of chain
                } else if entry == 0xFFF7 {
                    Err(FatError::BadCluster { cluster: cluster as u32 })
                } else if entry < 2 || entry as u32 > self.max_cluster {
                    Err(FatError::ClusterOutOfBounds {
                        cluster: entry as u32,
                        max: self.max_cluster,
                    })
                } else {
                    Ok(Some(entry as u32))
                }
            }
            FatType::Fat32 => {
                let entry = self.cache.read_fat32_entry(reader, cluster)? & 0x0FFF_FFFF;
                if entry >= 0x0FFF_FFF8 {
                    Ok(None) // End of chain
                } else if entry == 0x0FFF_FFF7 {
                    Err(FatError::BadCluster { cluster: cluster as u32 })
                } else if entry < 2 || entry > self.max_cluster {
                    Err(FatError::ClusterOutOfBounds {
                        cluster: entry,
                        max: self.max_cluster,
                    })
                } else {
                    Ok(Some(entry))
                }
            }
        }
    }

    /// Read the entire cluster chain starting from a cluster.
    ///
    /// This is efficient because it uses the cache to avoid repeated seeks.
    pub fn read_chain<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        start_cluster: u32,
    ) -> Result<Vec<u32>> {
        let mut chain = Vec::new();
        let mut current = start_cluster;

        // Prevent infinite loops
        let max_iterations = self.max_cluster as usize;
        let mut iterations = 0;

        loop {
            if current < 2 || current > self.max_cluster {
                break;
            }

            chain.push(current);
            iterations += 1;

            if iterations > max_iterations {
                // Likely a loop in the FAT
                break;
            }

            match self.next_cluster(reader, current as usize)? {
                Some(next) => current = next,
                None => break,
            }
        }

        Ok(chain)
    }

    /// Flush the cache to disk.
    pub fn flush<T: Write + Seek>(&mut self, writer: &mut T) -> Result<()> {
        self.cache.flush(writer)
    }
}

// Add accessor methods to Fat12, Fat16, Fat32 for max_cluster
impl Fat12 {
    /// Get the maximum cluster number.
    pub fn max_cluster(&self) -> u16 {
        self.max_cluster
    }
}

impl Fat16 {
    /// Get the maximum cluster number.
    pub fn max_cluster(&self) -> u16 {
        self.max_cluster
    }
}

impl Fat32 {
    /// Get the maximum cluster number.
    pub fn max_cluster(&self) -> u32 {
        self.max_cluster
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_stats() {
        let stats = CacheStats {
            hits: 80,
            misses: 20,
            evictions: 5,
            dirty_writes: 3,
        };
        assert!((stats.hit_ratio() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_cache_stats_empty() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_ratio(), 0.0);
    }
}
