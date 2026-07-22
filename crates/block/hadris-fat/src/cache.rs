//! FAT sector caching for reduced I/O operations.
//!
//! This module provides a write-back sector cache for FAT table operations,
//! significantly reducing the number of seek and read operations when
//! traversing cluster chains.
//!
//! # Quick start
//!
//! Most callers don't construct [`FatSectorCache`] directly — install one
//! on a [`crate::FatVolume`] via [`crate::FatVolumeBuilder::fat_cache`] and
//! drive it through [`crate::FatVolume::with_cached_fat`]:
//!
//! ```rust,no_run
//! # #[cfg(all(feature = "cache", feature = "std"))]
//! # {
//! use std::fs::OpenOptions;
//! use hadris_fat::FatVolume;
//!
//! let disk = OpenOptions::new().read(true).write(true).open("disk.img").unwrap();
//! let fs = FatVolume::builder(disk).fat_cache(16).open().unwrap();
//!
//! // Walk the FAT cluster chain starting at cluster 42, using the cache.
//! // The closure runs with both the cache and disk mutexes held.
//! let chain = fs
//!     .with_cached_fat(|cached, disk| cached.read_chain(disk, 42))
//!     .expect("cache installed")
//!     .expect("read_chain ok");
//!
//! // After dirty writes, flush before dropping the FatVolume to persist.
//! fs.flush().unwrap();
//! # }
//! ```
//!
//! As of phase C5 the built-in `FatVolume` operations (`read_file`,
//! `create_file`, `delete`, `truncate`, etc.) consult the cache
//! transparently — internal FAT-table reads go through
//! `next_cluster_routed` and writes go through `write_clus_routed` /
//! `allocate_cluster_routed` / `free_chain_routed` /
//! `truncate_chain_routed`. [`crate::FatVolume::with_cached_fat`] remains the
//! recommended entry point for *bulk* user-driven walks (free-cluster
//! scans, chain traversal) where holding the cache+disk locks across
//! many entries is cheaper than re-acquiring them per call.
//!
//! Caching is a **synchronous-only** feature: the `cache` Cargo feature
//! implies `sync`, and the FatVolume cache wiring is emitted only in the sync
//! slice. Async builds compile with the same on-disk surface but fall
//! through to direct `Fat::*` reads/writes (deferred to phase C5b — see
//! issue #27).
//!
//! # Semantics
//!
//! Reads ([`FatSectorCache::read_fat12_entry`] etc.) take any
//! `Read + Seek` source — they never need to write. If the cache is full of
//! *dirty* entries (lines you wrote and haven't flushed), a fresh read may
//! need to evict a dirty entry; in that case it returns
//! [`Error::CacheDirtyEviction`] rather than silently dropping bytes.
//! Call [`FatSectorCache::flush`] (or [`crate::FatVolume::flush`]) to empty the
//! dirty pool.
//!
//! Writes ([`FatSectorCache::write_fat12_entry`] etc.) take
//! `Read + Write + Seek` and *write through* on eviction: a dirty sector is
//! flushed to every FAT copy before it leaves the cache, so writes are never
//! silently lost.

use alloc::vec::Vec;

use super::fat_table::{Fat, FatType};
use super::io::{Read, Seek, SeekFrom, Write};
use crate::error::{Error, Result};

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
        Self::new(
            fat_start,
            fat_size,
            fat_count,
            sector_size,
            DEFAULT_CACHE_CAPACITY,
        )
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

    /// Find a cached sector by sector number.
    fn find_sector(&mut self, sector: usize) -> Option<usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.sector == sector {
                return Some(i);
            }
        }
        None
    }

    /// Read a sector into the cache. Read-only path: never marks the entry
    /// dirty, never needs a writer. Returns [`Error::CacheDirtyEviction`]
    /// if the cache is at capacity and *every* resident sector is dirty —
    /// the caller must `flush()` first because dropping a dirty entry would
    /// silently lose unwritten data.
    fn get_sector<T: Read + Seek>(&mut self, reader: &mut T, sector: usize) -> Result<&[u8]> {
        self.access_counter += 1;

        // Check if already cached
        if let Some(idx) = self.find_sector(sector) {
            self.stats.hits += 1;
            self.entries[idx].access_count = self.access_counter;
            return Ok(&self.entries[idx].data);
        }

        self.stats.misses += 1;

        // Evict before reading: read-paths can only evict clean entries, so
        // the eviction may fail with CacheDirtyEviction. Returning early
        // means we never read a sector we then can't park.
        if self.entries.len() >= self.capacity {
            self.evict_lru_clean()?;
        }

        // Load from disk
        let mut data = alloc::vec![0u8; self.sector_size];
        let offset = self.fat_start + sector * self.sector_size;
        reader.seek(SeekFrom::Start(offset as u64))?;
        reader.read_exact(&mut data)?;

        self.entries.push(CacheEntry {
            sector,
            data,
            dirty: false,
            access_count: self.access_counter,
        });

        Ok(&self.entries.last().unwrap().data)
    }

    /// Get a mutable reference to a sector, loading it if necessary.
    ///
    /// Write path: requires `Read + Write + Seek` so a dirty LRU can be
    /// flushed (write-through to every FAT copy) before being evicted.
    /// Marks the returned entry dirty.
    fn get_sector_mut<T: Read + Write + Seek>(
        &mut self,
        io: &mut T,
        sector: usize,
    ) -> Result<&mut [u8]> {
        self.access_counter += 1;

        if let Some(idx) = self.find_sector(sector) {
            self.stats.hits += 1;
            self.entries[idx].access_count = self.access_counter;
            self.entries[idx].dirty = true;
            return Ok(&mut self.entries[idx].data);
        }

        self.stats.misses += 1;

        // Evict before reading: write-paths can flush dirty entries on
        // eviction, so this never fails for cache-pressure reasons (only if
        // the underlying writer fails).
        if self.entries.len() >= self.capacity {
            self.evict_lru_flush(io)?;
        }

        let mut data = alloc::vec![0u8; self.sector_size];
        let offset = self.fat_start + sector * self.sector_size;
        io.seek(SeekFrom::Start(offset as u64))?;
        io.read_exact(&mut data)?;

        self.entries.push(CacheEntry {
            sector,
            data,
            dirty: true,
            access_count: self.access_counter,
        });

        let idx = self.entries.len() - 1;
        Ok(&mut self.entries[idx].data)
    }

    /// Find the index of the LRU entry, or `None` if the cache is empty.
    fn find_lru_index(&self) -> Option<usize> {
        let mut lru_idx = None;
        let mut lru_count = u64::MAX;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.access_count < lru_count {
                lru_count = entry.access_count;
                lru_idx = Some(i);
            }
        }
        lru_idx
    }

    /// Find the LRU among *non-dirty* entries, or `None` if all entries are
    /// dirty (or the cache is empty).
    fn find_lru_clean_index(&self) -> Option<usize> {
        let mut lru_idx = None;
        let mut lru_count = u64::MAX;
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.dirty && entry.access_count < lru_count {
                lru_count = entry.access_count;
                lru_idx = Some(i);
            }
        }
        lru_idx
    }

    /// Read-path eviction: drops the LRU non-dirty entry. If every entry is
    /// dirty, returns [`Error::CacheDirtyEviction`] — read paths can't
    /// safely drop unwritten data, and there's no writer available here to
    /// flush it.
    fn evict_lru_clean(&mut self) -> Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }
        match self.find_lru_clean_index() {
            Some(idx) => {
                self.entries.swap_remove(idx);
                self.stats.evictions += 1;
                Ok(())
            }
            None => {
                // Every resident entry is dirty. Surface the LRU's sector
                // so the caller has a useful breadcrumb (which sector they
                // need to flush before retrying).
                let lru = self.find_lru_index().expect("non-empty above");
                Err(Error::CacheDirtyEviction {
                    sector: self.entries[lru].sector as u32,
                })
            }
        }
    }

    /// Write-path eviction: flushes the LRU entry to every FAT copy if
    /// dirty, then removes it. Always succeeds unless the underlying writer
    /// fails.
    fn evict_lru_flush<T: Write + Seek>(&mut self, writer: &mut T) -> Result<()> {
        let Some(idx) = self.find_lru_index() else {
            return Ok(());
        };
        if self.entries[idx].dirty {
            // Mirror `flush`'s loop: write to every FAT copy so backup FATs
            // stay consistent with the primary.
            for copy in 0..self.fat_count {
                let offset = self.fat_start
                    + copy * self.fat_size
                    + self.entries[idx].sector * self.sector_size;
                writer.seek(SeekFrom::Start(offset as u64))?;
                writer.write_all(&self.entries[idx].data)?;
            }
            self.entries[idx].dirty = false;
            self.stats.dirty_writes += 1;
        }
        self.entries.swap_remove(idx);
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
        let value = if cluster.is_multiple_of(2) {
            u16::from(bytes[0]) | (u16::from(bytes[1] & 0x0F) << 8)
        } else {
            (u16::from(bytes[0]) >> 4) | (u16::from(bytes[1]) << 4)
        };

        Ok(value)
    }

    /// Write a FAT12 entry through the cache.
    ///
    /// Requires `Read + Write + Seek` because the cache is write-back: an
    /// LRU eviction during this call must flush a dirty sector to disk
    /// before discarding it.
    pub fn write_fat12_entry<T: Read + Write + Seek>(
        &mut self,
        io: &mut T,
        cluster: usize,
        value: u16,
    ) -> Result<()> {
        let sector_size = self.sector_size;

        let byte_offset = (cluster * 3) / 2;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        if offset_in_sector + 1 < sector_size {
            // Entry is within one sector
            let data = self.get_sector_mut(io, sector)?;

            if cluster.is_multiple_of(2) {
                data[offset_in_sector] = value as u8;
                data[offset_in_sector + 1] =
                    (data[offset_in_sector + 1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
            } else {
                data[offset_in_sector] = (data[offset_in_sector] & 0x0F) | ((value << 4) as u8);
                data[offset_in_sector + 1] = (value >> 4) as u8;
            }
        } else {
            // Entry spans two sectors
            {
                let data = self.get_sector_mut(io, sector)?;
                if cluster.is_multiple_of(2) {
                    data[offset_in_sector] = value as u8;
                } else {
                    data[offset_in_sector] = (data[offset_in_sector] & 0x0F) | ((value << 4) as u8);
                }
            }

            {
                let data = self.get_sector_mut(io, sector + 1)?;
                if cluster.is_multiple_of(2) {
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

    /// Write a FAT16 entry through the cache.
    ///
    /// Requires `Read + Write + Seek` because eviction may flush a dirty
    /// sector before discarding it (see [`Self::write_fat12_entry`]).
    pub fn write_fat16_entry<T: Read + Write + Seek>(
        &mut self,
        io: &mut T,
        cluster: usize,
        value: u16,
    ) -> Result<()> {
        let sector_size = self.sector_size;
        let byte_offset = cluster * 2;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        let data = self.get_sector_mut(io, sector)?;
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

    /// Write a FAT32 entry through the cache.
    ///
    /// Requires `Read + Write + Seek` because eviction may flush a dirty
    /// sector before discarding it (see [`Self::write_fat12_entry`]).
    pub fn write_fat32_entry<T: Read + Write + Seek>(
        &mut self,
        io: &mut T,
        cluster: usize,
        value: u32,
    ) -> Result<()> {
        let sector_size = self.sector_size;
        let byte_offset = cluster * 4;
        let sector = byte_offset / sector_size;
        let offset_in_sector = byte_offset % sector_size;

        let data = self.get_sector_mut(io, sector)?;
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
                    Err(Error::BadCluster {
                        cluster: cluster as u32,
                    })
                } else if entry < 2 || entry as u32 > self.max_cluster {
                    Err(Error::ClusterOutOfBounds {
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
                    Err(Error::BadCluster {
                        cluster: cluster as u32,
                    })
                } else if entry < 2 || entry as u32 > self.max_cluster {
                    Err(Error::ClusterOutOfBounds {
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
                    Err(Error::BadCluster {
                        cluster: cluster as u32,
                    })
                } else if entry < 2 || entry > self.max_cluster {
                    Err(Error::ClusterOutOfBounds {
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
    /// On a chain longer than `max_cluster` clusters (only possible if the
    /// FAT is corrupt and contains a loop), returns
    /// [`Error::ClusterLoop`] rather than silently returning a
    /// partial chain.
    pub fn read_chain<T: Read + Seek>(
        &mut self,
        reader: &mut T,
        start_cluster: u32,
    ) -> Result<Vec<u32>> {
        let mut chain = Vec::new();
        let mut current = start_cluster;

        // A healthy chain visits each cluster at most once, so it can't
        // exceed `max_cluster - 1` clusters (clusters 0 and 1 are reserved).
        // Anything larger means the chain has a loop.
        let max_iterations = self.max_cluster as usize;

        loop {
            if current < 2 || current > self.max_cluster {
                break;
            }

            chain.push(current);

            if chain.len() > max_iterations {
                return Err(Error::ClusterLoop { cluster: current });
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
