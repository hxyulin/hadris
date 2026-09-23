io_transform! {

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::codec::entry::{ChainError, FatKind};
use crate::error::{Error, Result};
#[cfg(feature = "write")]
use super::io::Write;
use super::io::{Read, Seek, SeekFrom};

/// Size of the sliding window used when reading FAT chains in bulk (bytes).
/// Set at build time via `HADRIS_FAT_CACHE_SIZE` (e.g. 4MiB, 8M). Build script sets
/// `FAT_CACHE_WINDOW_SIZE_BYTES` (rustc-env); we read it with `env!` and `.parse()`.
#[cfg(feature = "alloc")]
const FAT_CACHE_WINDOW_SIZE: &str = env!("FAT_CACHE_WINDOW_SIZE_BYTES");

fn map_next(result: core::result::Result<Option<u32>, ChainError>, cluster: usize, max: u32) -> Result<Option<u32>> {
    result.map_err(|err| match err {
        ChainError::Bad => Error::BadCluster {
            cluster: cluster as u32,
        },
        ChainError::OutOfBounds(value) => Error::ClusterOutOfBounds { cluster: value, max },
    })
}

/// Read a cluster chain in bulk using a sliding window over the FAT
/// (FAT12, FAT16, FAT32).
///
/// Used when `alloc` is enabled as the backend for [`with_cached_chain`](super::read::FileReader::with_cached_chain).
#[cfg(feature = "alloc")]
#[allow(clippy::too_many_arguments)]
async fn read_chain_wide<R>(
    reader: &mut R,
    fat_start: usize,
    fat_size: usize,
    start_cluster: u32,
    max_clusters: usize,
    kind: FatKind,
    max_cluster: u32,
) -> Result<Vec<u32>>
where
    R: Read + Seek,
{
    let cache_size = FAT_CACHE_WINDOW_SIZE.parse::<usize>().unwrap();
    let mut fat_buf = alloc::vec![0u8; FAT_CACHE_WINDOW_SIZE.parse::<usize>().unwrap()];
    let mut window_start = usize::MAX;
    let mut valid_len = 0usize;

    let mut chain = Vec::new();
    let mut current = start_cluster as usize;
    let mut iterations = 0;

    while current >= 2 && iterations <= max_clusters {
        chain.push(current as u32);
        iterations += 1;

        let offset_in_fat = kind.entry_offset(current as u64) as usize;
        let span = kind.entry_len();

        if offset_in_fat < window_start || offset_in_fat + span > window_start + valid_len {
            window_start = offset_in_fat;
            valid_len = cache_size.min(fat_size.saturating_sub(window_start));
            reader
                .seek(SeekFrom::Start((fat_start + window_start) as u64))
                .await?;
            reader.read_exact(&mut fat_buf[..valid_len]).await?;
        }

        let buffer_offset = offset_in_fat - window_start;
        let stored = kind.decode(current as u64, &fat_buf[buffer_offset..][..span]);
        match map_next(kind.next(stored, max_cluster), current, max_cluster)? {
            Some(next) => current = next as usize,
            None => break,
        }
    }
    Ok(chain)
}

/// Allocation-table implementation selected for a mounted FAT volume.
pub enum Fat {
    /// FAT12 allocation table.
    Fat12(Fat12),
    /// FAT16 allocation table.
    Fat16(Fat16),
    /// FAT32 allocation table.
    Fat32(Fat32),
}

impl Fat {
    /// Returns the next cluster in a chain, or `None` at end-of-chain.
    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        match self {
            Self::Fat12(fat12) => fat12.next_cluster(reader, cluster).await,
            Self::Fat16(fat16) => fat16.next_cluster(reader, cluster).await,
            Self::Fat32(fat32) => fat32.next_cluster(reader, cluster).await,
        }
    }

    /// Read an entire cluster chain into a vector (alloc only).
    ///
    /// Uses a 4 MiB sliding window over the FAT for all types to reduce I/O.
    #[cfg(feature = "alloc")]
    pub(crate) async fn read_chain<T: Read + Seek>(
        &self,
        reader: &mut T,
        start_cluster: u32,
        max_clusters: usize,
    ) -> Result<Vec<u32>> {
        match self {
            Self::Fat12(fat12) => {
                read_chain_wide(
                    reader,
                    fat12.start,
                    fat12.size,
                    start_cluster,
                    max_clusters,
                    FatKind::Fat12,
                    fat12.max_cluster as u32,
                )
                .await
            }
            Self::Fat16(fat16) => {
                read_chain_wide(
                    reader,
                    fat16.start,
                    fat16.size,
                    start_cluster,
                    max_clusters,
                    FatKind::Fat16,
                    fat16.max_cluster as u32,
                )
                .await
            }
            Self::Fat32(fat32) => {
                read_chain_wide(
                    reader,
                    fat32.start,
                    fat32.size,
                    start_cluster,
                    max_clusters,
                    FatKind::Fat32,
                    fat32.max_cluster,
                )
                .await
            }
        }
    }

    /// Get the FAT type for informational purposes
    pub fn fat_type(&self) -> FatType {
        match self {
            Self::Fat12(_) => FatType::Fat12,
            Self::Fat16(_) => FatType::Fat16,
            Self::Fat32(_) => FatType::Fat32,
        }
    }

    /// Highest valid cluster number on this volume (inclusive). Clusters 0 and 1
    /// are reserved, so the chain-loop ceiling is `max_cluster - 1` distinct
    /// clusters; one extra step lets the walker observe the end-of-chain marker
    /// before declaring a loop.
    pub fn max_cluster(&self) -> u32 {
        match self {
            Self::Fat12(f) => f.max_cluster as u32,
            Self::Fat16(f) => f.max_cluster as u32,
            Self::Fat32(f) => f.max_cluster,
        }
    }

    /// Walk a cluster chain starting at `start`, calling `visit` for each
    /// cluster (including `start`) until the chain ends. Returns the last
    /// cluster visited.
    ///
    /// Aborts with [`Error::ClusterLoop`] if more than `max_steps`
    /// clusters are walked. The natural upper bound is
    /// [`Self::max_cluster`] — any chain longer than that must repeat a
    /// cluster and is therefore corrupt.
    pub async fn walk_chain<T, F>(
        &self,
        reader: &mut T,
        start: u32,
        max_steps: u32,
        mut visit: F,
    ) -> Result<u32>
    where
        T: Read + Seek,
        F: FnMut(u32),
    {
        let mut current = start;
        let mut steps: u32 = 0;
        loop {
            visit(current);
            steps = steps.saturating_add(1);
            if steps > max_steps {
                return Err(Error::ClusterLoop { cluster: current });
            }
            match self.next_cluster(reader, current as usize).await? {
                Some(next) => current = next,
                None => return Ok(current),
            }
        }
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub async fn truncate_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
    ) -> Result<u32> {
        match self {
            Self::Fat12(fat12) => fat12.truncate_chain(rw, cluster as u16).await,
            Self::Fat16(fat16) => fat16.truncate_chain(rw, cluster as u16).await,
            Self::Fat32(fat32) => fat32.truncate_chain(rw, cluster as u32).await,
        }
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub async fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: usize) -> Result<u32> {
        match self {
            Self::Fat12(fat12) => fat12.free_chain(rw, cluster as u16).await,
            Self::Fat16(fat16) => fat16.free_chain(rw, cluster as u16).await,
            Self::Fat32(fat32) => fat32.free_chain(rw, cluster as u32).await,
        }
    }

    /// Mark a cluster as bad in the FAT.
    ///
    /// Writes the appropriate bad-cluster marker (0xFF7 / 0xFFF7 / 0x0FFFFFF7)
    /// to the FAT entry for the given cluster. This prevents the cluster from
    /// being allocated in the future.
    #[cfg(feature = "write")]
    pub async fn mark_bad<T: Read + Write + Seek>(&self, rw: &mut T, cluster: usize) -> Result<()> {
        match self {
            Self::Fat12(fat12) => fat12.mark_bad(rw, cluster as u16).await,
            Self::Fat16(fat16) => fat16.mark_bad(rw, cluster as u16).await,
            Self::Fat32(fat32) => fat32.mark_bad(rw, cluster as u32).await,
        }
    }

    /// Read the FAT-resident status bits from `FAT[1]`.
    ///
    /// The FAT spec stores two volume-level status bits in the high bits of
    /// the FAT entry for cluster 1:
    ///   * "clean shutdown" (set = clean, cleared = dirty)
    ///   * "no I/O errors" (set = ok, cleared = errors during last use)
    /// FAT12 has no spare bits in its packed 12-bit entries, so it returns
    /// `(false, false)` (treat as "clean, no errors") — matches the spec's
    /// "FAT12 has no status word" reading.
    ///
    /// Returns `(dirty, io_errors)` — both `true` means trouble.
    pub async fn read_status_flags<T: Read + Seek>(&self, reader: &mut T) -> Result<(bool, bool)> {
        match self {
            Self::Fat12(_) => Ok((false, false)),
            Self::Fat16(f) => {
                // FAT[1] sits at byte offset start + 2 (cluster 1, 16-bit entries).
                let offset = f.start + 2;
                reader.seek(SeekFrom::Start(offset as u64)).await?;
                let mut bytes = [0u8; 2];
                reader.read_exact(&mut bytes).await?;
                let val = u16::from_le_bytes(bytes);
                // Bit 15 cleared = dirty; bit 14 cleared = I/O errors.
                Ok((val & 0x8000 == 0, val & 0x4000 == 0))
            }
            Self::Fat32(f) => {
                // FAT[1] at byte offset start + 4 (cluster 1, 32-bit entries).
                let offset = f.start + 4;
                reader.seek(SeekFrom::Start(offset as u64)).await?;
                let mut bytes = [0u8; 4];
                reader.read_exact(&mut bytes).await?;
                let val = u32::from_le_bytes(bytes);
                // Bit 27 cleared = dirty; bit 26 cleared = I/O errors. Only
                // these two bits are status; the rest of the high nibble is
                // reserved and the low 28 bits are the cluster-1 entry value.
                Ok((val & 0x0800_0000 == 0, val & 0x0400_0000 == 0))
            }
        }
    }

    /// Returns the number of FAT copies on disk (typically 1 or 2).
    pub fn fat_copy_count(&self) -> usize {
        match self {
            Self::Fat12(f) => f.count,
            Self::Fat16(f) => f.count,
            Self::Fat32(f) => f.count,
        }
    }

    /// Compare a FAT entry between the primary (index 0) and backup (index 1) copies.
    ///
    /// Returns `Ok(true)` if both copies match, `Ok(false)` if they differ.
    /// Returns an error if there is only one FAT copy or on I/O failure.
    pub async fn compare_entry<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<bool> {
        if self.fat_copy_count() < 2 {
            return Err(Error::UnsupportedFatType("no backup FAT copy available"));
        }
        match self {
            Self::Fat12(f) => {
                let a = f.read_clus_at(reader, cluster, 0).await?;
                let b = f.read_clus_at(reader, cluster, 1).await?;
                Ok(a == b)
            }
            Self::Fat16(f) => {
                let a = f.read_clus_at(reader, cluster, 0).await?;
                let b = f.read_clus_at(reader, cluster, 1).await?;
                Ok(a == b)
            }
            Self::Fat32(f) => {
                let a = f.read_clus_at(reader, cluster, 0).await?;
                let b = f.read_clus_at(reader, cluster, 1).await?;
                Ok(a == b)
            }
        }
    }

    /// Read the next cluster, falling back to the backup FAT copy on error.
    ///
    /// Tries the primary FAT first. If reading fails and a backup FAT exists,
    /// reads from the backup copy instead.
    pub async fn next_cluster_with_fallback<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        match self.next_cluster(reader, cluster).await {
            Ok(result) => Ok(result),
            Err(_primary_err) if self.fat_copy_count() >= 2 => {
                // Try reading from backup FAT (index 1)
                match self {
                    Self::Fat12(f) => {
                        let entry = f.read_clus_at(reader, cluster, 1).await? & Fat12::ENTRY_MASK;
                        if Fat12::is_end_of_chain(entry) { return Ok(None); }
                        if Fat12::is_bad_cluster(entry) { return Err(Error::BadCluster { cluster: cluster as u32 }); }
                        f.validate_cluster(entry)?;
                        Ok(Some(entry as u32))
                    }
                    Self::Fat16(f) => {
                        let entry = f.read_clus_at(reader, cluster, 1).await?;
                        if Fat16::is_end_of_chain(entry) { return Ok(None); }
                        if Fat16::is_bad_cluster(entry) { return Err(Error::BadCluster { cluster: cluster as u32 }); }
                        f.validate_cluster(entry)?;
                        Ok(Some(entry as u32))
                    }
                    Self::Fat32(f) => {
                        let raw = f.read_clus_at(reader, cluster, 1).await?;
                        let entry = raw & Fat32::ENTRY_MASK;
                        if Fat32::is_end_of_chain(entry) { return Ok(None); }
                        if Fat32::is_bad_cluster(entry) { return Err(Error::BadCluster { cluster: cluster as u32 }); }
                        f.validate_cluster(entry)?;
                        Ok(Some(entry))
                    }
                }
            }
            Err(e) => Err(e),
        }
    }
}

/// FAT filesystem type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FatType {
    /// 12-bit FAT entries.
    Fat12,
    /// 16-bit FAT entries.
    Fat16,
    /// 28-bit cluster values stored in 32-bit FAT entries.
    Fat32,
}

impl core::fmt::Display for FatType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Fat12 => write!(f, "FAT12"),
            Self::Fat16 => write!(f, "FAT16"),
            Self::Fat32 => write!(f, "FAT32"),
        }
    }
}

/// FAT12 table implementation.
///
/// FAT12 uses 12-bit entries packed into 3 bytes for every 2 clusters.
pub struct Fat12 {
    start: usize,
    size: usize,
    count: usize,
    max_cluster: u16,
}

impl Fat12 {
    const KIND: FatKind = FatKind::Fat12;
    /// Mask for the 12-bit cluster number
    const ENTRY_MASK: u16 = Self::KIND.mask() as u16;
    /// Bad cluster marker
    #[cfg(feature = "write")]
    const BAD_CLUSTER: u16 = Self::KIND.bad_cluster() as u16;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u16 = crate::codec::entry::FIRST_DATA_CLUSTER as u16;

    /// Layout fields a [`crate::cache::FatSectorCache`] needs to mirror this
    /// FAT: `(start_byte, size_per_copy, copy_count)`.
    #[cfg(feature = "cache")]
    pub(crate) fn cache_layout(&self) -> (usize, usize, usize) {
        (self.start, self.size, self.count)
    }

    /// Creates a FAT12 table descriptor.
    pub fn new(start: usize, size: usize, count: usize, max_cluster: u16) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self {
            start,
            size,
            count,
            max_cluster,
        }
    }

    /// Get the maximum cluster number.
    pub fn max_cluster(&self) -> u16 {
        self.max_cluster
    }

    sync_only! {
        /// Calculates the byte offset used by synchronous analysis tooling.
        #[cfg(feature = "tool")]
        pub(crate) fn entry_byte_offset(&self, cluster: usize) -> usize {
            self.start + (cluster * 3) / 2
        }
    }

    async fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        self.read_clus_at(reader, cluster, 0).await
    }

    /// Read a FAT12 entry from a specific FAT copy (0 = primary, 1 = backup).
    async fn read_clus_at<T: Read + Seek>(&self, reader: &mut T, cluster: usize, fat_index: usize) -> Result<u16> {
        let byte_offset = self.start + fat_index * self.size + Self::KIND.entry_offset(cluster as u64) as usize;
        reader.seek(SeekFrom::Start(byte_offset as u64)).await?;

        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes).await?;

        Ok(Self::KIND.decode(cluster as u64, &bytes) as u16)
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u16) -> bool {
        Self::KIND.is_end_of_chain(value as u32)
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u16) -> bool {
        Self::KIND.is_bad(value as u32)
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u16) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(Error::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        if cluster > self.max_cluster {
            return Err(Error::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        Ok(())
    }

    /// Returns the next cluster in a chain, or `None` at end-of-chain.
    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let stored = self.read_clus(reader, cluster).await?;
        let max = self.max_cluster as u32;
        map_next(Self::KIND.next(stored as u32, max), cluster, max)
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u16 = 0x0000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u16 = Self::KIND.end_of_chain() as u16;

    /// Write a FAT12 entry at the specified cluster index to a specific FAT copy.
    #[cfg(feature = "write")]
    async fn write_clus_at<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
        value: u16,
        fat_index: usize,
    ) -> Result<()> {
        let byte_offset = self.start + fat_index * self.size + Self::KIND.entry_offset(cluster as u64) as usize;
        rw.seek(SeekFrom::Start(byte_offset as u64)).await?;

        // Read existing bytes (we need to preserve the other half)
        let mut bytes = [0u8; 2];
        rw.read_exact(&mut bytes).await?;
        Self::KIND.encode(cluster as u64, value as u32, &mut bytes);

        // Write back
        rw.seek(SeekFrom::Start(byte_offset as u64)).await?;
        rw.write_all(&bytes).await?;

        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub async fn write_clus<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
        value: u16,
    ) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(rw, cluster, value, i).await?;
        }
        Ok(())
    }

    /// Allocate a single cluster, returns the allocated cluster number.
    #[cfg(feature = "write")]
    pub async fn allocate_cluster<T: Read + Write + Seek>(&self, rw: &mut T, hint: u16) -> Result<u16> {
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to max_cluster
        for cluster in start..=self.max_cluster {
            let entry = self.read_clus(rw, cluster as usize).await? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from first cluster to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_clus(rw, cluster as usize).await? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;
                return Ok(cluster);
            }
        }

        Err(Error::NoFreeSpace)
    }

    /// Allocate a linked chain of `count` clusters, returning the first cluster.
    ///
    /// Each cluster is allocated from a rolling hint (near the previous one) and
    /// linked to its successor; the last is marked end-of-chain. Mirrors
    /// [`Fat32::allocate_chain`]. On partial failure the already-allocated
    /// clusters are not rolled back (matching the FAT32 behavior).
    #[cfg(feature = "write")]
    pub async fn allocate_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        count: usize,
        hint: u16,
    ) -> Result<u16> {
        if count == 0 {
            return Err(Error::NoFreeSpace);
        }
        let first = self.allocate_cluster(rw, hint).await?;
        let mut prev = first;
        for _ in 1..count {
            let next = self.allocate_cluster(rw, prev + 1).await?;
            self.write_clus(rw, prev as usize, next).await?;
            prev = next;
        }
        Ok(first)
    }

    /// Extend the chain whose current last cluster is `last` by `count`
    /// clusters, returning the first newly allocated cluster. If `count` is 0,
    /// no clusters are allocated and `last` is returned unchanged. Mirrors
    /// [`Fat32::extend_chain`].
    #[cfg(feature = "write")]
    pub async fn extend_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        last: u16,
        count: usize,
        hint: u16,
    ) -> Result<u16> {
        if count == 0 {
            return Ok(last);
        }
        let first_new = self.allocate_chain(rw, count, hint).await?;
        self.write_clus(rw, last as usize, first_new).await?;
        Ok(first_new)
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub async fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, start: u16) -> Result<u32> {
        let mut count = 0u32;
        let mut current = start;

        loop {
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            let next = self.read_clus(rw, current as usize).await? & Self::ENTRY_MASK;
            self.write_clus(rw, current as usize, Self::FREE_CLUSTER).await?;
            count += 1;

            if Self::is_end_of_chain(next)
                || Self::is_bad_cluster(next)
                || next == Self::FREE_CLUSTER
            {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub async fn truncate_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u16) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        // Read the next cluster in chain
        let next = self.read_clus(rw, cluster as usize).await? & Self::ENTRY_MASK;

        // Mark this cluster as end of chain
        self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;

        // Free the rest of the chain if there is one
        if !Self::is_end_of_chain(next)
            && next >= Self::FIRST_DATA_CLUSTER
            && next <= self.max_cluster
        {
            self.free_chain(rw, next).await
        } else {
            Ok(0)
        }
    }

    /// Mark a cluster as bad (0x0FF7) in all FAT copies.
    #[cfg(feature = "write")]
    pub async fn mark_bad<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u16) -> Result<()> {
        self.write_clus(rw, cluster as usize, Self::BAD_CLUSTER).await
    }
}

/// FAT16 table implementation.
pub struct Fat16 {
    start: usize,
    size: usize,
    count: usize,
    max_cluster: u16,
}

impl Fat16 {
    const KIND: FatKind = FatKind::Fat16;
    /// Bad cluster marker
    #[cfg(feature = "write")]
    const BAD_CLUSTER: u16 = Self::KIND.bad_cluster() as u16;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u16 = crate::codec::entry::FIRST_DATA_CLUSTER as u16;

    /// Layout fields a [`crate::cache::FatSectorCache`] needs to mirror this
    /// FAT: `(start_byte, size_per_copy, copy_count)`.
    #[cfg(feature = "cache")]
    pub(crate) fn cache_layout(&self) -> (usize, usize, usize) {
        (self.start, self.size, self.count)
    }

    /// Creates a FAT16 table descriptor.
    pub fn new(start: usize, size: usize, count: usize, max_cluster: u16) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self {
            start,
            size,
            count,
            max_cluster,
        }
    }

    /// Get the maximum cluster number.
    pub fn max_cluster(&self) -> u16 {
        self.max_cluster
    }

    sync_only! {
        #[cfg(feature = "tool")]
        pub(crate) fn entry_offset(&self, cluster: usize) -> usize {
            debug_assert!(cluster * size_of::<u16>() < self.size);
            self.start + cluster * size_of::<u16>()
        }
    }

    async fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        self.read_clus_at(reader, cluster, 0).await
    }

    /// Read a FAT16 entry from a specific FAT copy (0 = primary, 1 = backup).
    async fn read_clus_at<T: Read + Seek>(&self, reader: &mut T, cluster: usize, fat_index: usize) -> Result<u16> {
        let offset = self.start + fat_index * self.size + Self::KIND.entry_offset(cluster as u64) as usize;
        reader.seek(SeekFrom::Start(offset as u64)).await?;
        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes).await?;
        Ok(Self::KIND.decode(cluster as u64, &bytes) as u16)
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u16) -> bool {
        Self::KIND.is_end_of_chain(value as u32)
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u16) -> bool {
        Self::KIND.is_bad(value as u32)
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u16) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(Error::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        if cluster > self.max_cluster {
            return Err(Error::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        Ok(())
    }

    /// Returns the next cluster in a chain, or `None` at end-of-chain.
    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let stored = self.read_clus(reader, cluster).await?;
        let max = self.max_cluster as u32;
        map_next(Self::KIND.next(stored as u32, max), cluster, max)
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u16 = 0x0000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u16 = Self::KIND.end_of_chain() as u16;

    /// Write a cluster entry to the FAT table at the specified FAT copy
    #[cfg(feature = "write")]
    async fn write_clus_at<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u16,
        fat_index: usize,
    ) -> Result<()> {
        let offset = self.start + fat_index * self.size + Self::KIND.entry_offset(cluster as u64) as usize;
        writer.seek(SeekFrom::Start(offset as u64)).await?;
        let mut bytes = [0u8; 2];
        Self::KIND.encode(cluster as u64, value as u32, &mut bytes);
        writer.write_all(&bytes).await?;
        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub async fn write_clus<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u16,
    ) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(writer, cluster, value, i).await?;
        }
        Ok(())
    }

    /// Allocate a single cluster, returns the allocated cluster number.
    #[cfg(feature = "write")]
    pub async fn allocate_cluster<T: Read + Write + Seek>(&self, rw: &mut T, hint: u16) -> Result<u16> {
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to max_cluster
        for cluster in start..=self.max_cluster {
            let entry = self.read_clus(rw, cluster as usize).await?;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from first cluster to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_clus(rw, cluster as usize).await?;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;
                return Ok(cluster);
            }
        }

        Err(Error::NoFreeSpace)
    }

    /// Allocate a linked chain of `count` clusters, returning the first cluster.
    ///
    /// Each cluster is allocated from a rolling hint (near the previous one) and
    /// linked to its successor; the last is marked end-of-chain. Mirrors
    /// [`Fat32::allocate_chain`]. On partial failure the already-allocated
    /// clusters are not rolled back (matching the FAT32 behavior).
    #[cfg(feature = "write")]
    pub async fn allocate_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        count: usize,
        hint: u16,
    ) -> Result<u16> {
        if count == 0 {
            return Err(Error::NoFreeSpace);
        }
        let first = self.allocate_cluster(rw, hint).await?;
        let mut prev = first;
        for _ in 1..count {
            let next = self.allocate_cluster(rw, prev + 1).await?;
            self.write_clus(rw, prev as usize, next).await?;
            prev = next;
        }
        Ok(first)
    }

    /// Extend the chain whose current last cluster is `last` by `count`
    /// clusters, returning the first newly allocated cluster. If `count` is 0,
    /// no clusters are allocated and `last` is returned unchanged. Mirrors
    /// [`Fat32::extend_chain`].
    #[cfg(feature = "write")]
    pub async fn extend_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        last: u16,
        count: usize,
        hint: u16,
    ) -> Result<u16> {
        if count == 0 {
            return Ok(last);
        }
        let first_new = self.allocate_chain(rw, count, hint).await?;
        self.write_clus(rw, last as usize, first_new).await?;
        Ok(first_new)
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub async fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, start: u16) -> Result<u32> {
        let mut count = 0u32;
        let mut current = start;

        loop {
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            let next = self.read_clus(rw, current as usize).await?;
            self.write_clus(rw, current as usize, Self::FREE_CLUSTER).await?;
            count += 1;

            if Self::is_end_of_chain(next)
                || Self::is_bad_cluster(next)
                || next == Self::FREE_CLUSTER
            {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub async fn truncate_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u16) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        // Read the next cluster in chain
        let next = self.read_clus(rw, cluster as usize).await?;

        // Mark this cluster as end of chain
        self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;

        // Free the rest of the chain if there is one
        if !Self::is_end_of_chain(next)
            && next >= Self::FIRST_DATA_CLUSTER
            && next <= self.max_cluster
        {
            self.free_chain(rw, next).await
        } else {
            Ok(0)
        }
    }

    /// Mark a cluster as bad (0xFFF7) in all FAT copies.
    #[cfg(feature = "write")]
    pub async fn mark_bad<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u16) -> Result<()> {
        self.write_clus(rw, cluster as usize, Self::BAD_CLUSTER).await
    }
}

/// FAT32 allocation-table descriptor.
pub struct Fat32 {
    start: usize,
    size: usize,
    count: usize,
    max_cluster: u32,
}

impl Fat32 {
    const KIND: FatKind = FatKind::Fat32;
    /// Mask for the 28-bit cluster number (upper 4 bits are reserved)
    const ENTRY_MASK: u32 = Self::KIND.mask();
    /// Bad cluster marker
    #[cfg(feature = "write")]
    const BAD_CLUSTER: u32 = Self::KIND.bad_cluster();
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u32 = crate::codec::entry::FIRST_DATA_CLUSTER;

    /// Layout fields a [`crate::cache::FatSectorCache`] needs to mirror this
    /// FAT: `(start_byte, size_per_copy, copy_count)`.
    #[cfg(feature = "cache")]
    pub(crate) fn cache_layout(&self) -> (usize, usize, usize) {
        (self.start, self.size, self.count)
    }

    /// Creates a FAT32 table descriptor.
    pub fn new(start: usize, size: usize, count: usize, max_cluster: u32) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self {
            start,
            size,
            count,
            max_cluster,
        }
    }

    /// Get the maximum cluster number.
    pub fn max_cluster(&self) -> u32 {
        self.max_cluster
    }

    sync_only! {
        #[cfg(feature = "tool")]
        pub(crate) fn entry_offset(&self, cluster: usize) -> usize {
            debug_assert!(cluster * size_of::<u32>() < self.size);
            self.start + cluster * size_of::<u32>()
        }
    }

    async fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u32> {
        self.read_clus_at(reader, cluster, 0).await
    }

    /// Read a FAT32 entry from a specific FAT copy (0 = primary, 1 = backup).
    async fn read_clus_at<T: Read + Seek>(&self, reader: &mut T, cluster: usize, fat_index: usize) -> Result<u32> {
        let offset = self.start + fat_index * self.size + Self::KIND.entry_offset(cluster as u64) as usize;
        reader.seek(SeekFrom::Start(offset as u64)).await?;
        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes).await?;
        Ok(Self::KIND.decode(cluster as u64, &bytes))
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u32) -> bool {
        Self::KIND.is_end_of_chain(value)
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u32) -> bool {
        Self::KIND.is_bad(value)
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u32) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(Error::ClusterOutOfBounds {
                cluster,
                max: self.max_cluster,
            });
        }
        if cluster > self.max_cluster {
            return Err(Error::ClusterOutOfBounds {
                cluster,
                max: self.max_cluster,
            });
        }
        Ok(())
    }

    /// Returns the next cluster in a chain, or `None` at end-of-chain.
    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let stored = self.read_clus(reader, cluster).await?;
        map_next(Self::KIND.next(stored, self.max_cluster), cluster, self.max_cluster)
    }

    /// Write a cluster entry to the FAT table at the specified FAT copy
    #[cfg(feature = "write")]
    async fn write_clus_at<T: Read + Write + Seek>(
        &self,
        io: &mut T,
        cluster: usize,
        value: u32,
        fat_index: usize,
    ) -> Result<()> {
        let offset = self.start + fat_index * self.size + Self::KIND.entry_offset(cluster as u64) as usize;
        io.seek(SeekFrom::Start(offset as u64)).await?;
        let mut bytes = [0_u8; 4];
        io.read_exact(&mut bytes).await?;
        Self::KIND.encode(cluster as u64, value, &mut bytes);
        io.seek(SeekFrom::Start(offset as u64)).await?;
        io.write_all(&bytes).await?;
        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub async fn write_clus<T: Read + Write + Seek>(
        &self,
        io: &mut T,
        cluster: usize,
        value: u32,
    ) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(io, cluster, value, i).await?;
        }
        Ok(())
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u32 = 0x00000000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u32 = Self::KIND.end_of_chain();

    /// Allocate a single cluster, returns the allocated cluster number.
    /// Searches starting from `hint` for a free cluster.
    #[cfg(feature = "write")]
    pub async fn allocate_cluster<T: Read + Write + Seek>(&self, rw: &mut T, hint: u32) -> Result<u32> {
        // Start searching from hint, wrapping around if needed
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to max_cluster
        for cluster in start..=self.max_cluster {
            let entry = self.read_clus(rw, cluster as usize).await? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                // Mark as end of chain
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from first cluster to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_clus(rw, cluster as usize).await? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                // Mark as end of chain
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;
                return Ok(cluster);
            }
        }

        Err(Error::NoFreeSpace)
    }

    /// Allocate a chain of clusters, linking them together.
    /// Returns the first cluster of the allocated chain.
    #[cfg(feature = "write")]
    pub async fn allocate_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        count: usize,
        hint: u32,
    ) -> Result<u32> {
        if count == 0 {
            return Err(Error::NoFreeSpace);
        }

        let first = self.allocate_cluster(rw, hint).await?;
        let mut prev = first;

        for _ in 1..count {
            let next = self.allocate_cluster(rw, prev + 1).await?;
            // Link previous cluster to this one
            self.write_clus(rw, prev as usize, next).await?;
            prev = next;
        }

        Ok(first)
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub async fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, start: u32) -> Result<u32> {
        let mut count = 0;
        let mut current = start;

        loop {
            // Validate cluster
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            // Read the next cluster before freeing
            let raw_entry = self.read_clus(rw, current as usize).await?;
            let next = raw_entry & Self::ENTRY_MASK;

            // Free this cluster
            self.write_clus(rw, current as usize, Self::FREE_CLUSTER).await?;
            count += 1;

            // Check if this was the end of chain
            if Self::is_end_of_chain(next)
                || Self::is_bad_cluster(next)
                || next == Self::FREE_CLUSTER
            {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub async fn truncate_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u32) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        // Read the next cluster in chain
        let raw_entry = self.read_clus(rw, cluster as usize).await?;
        let next = raw_entry & Self::ENTRY_MASK;

        // Mark this cluster as end of chain
        self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN).await?;

        // Free the rest of the chain if there is one
        if !Self::is_end_of_chain(next)
            && next >= Self::FIRST_DATA_CLUSTER
            && next <= self.max_cluster
        {
            self.free_chain(rw, next).await
        } else {
            Ok(0)
        }
    }

    /// Mark a cluster as bad (0x0FFFFFF7) in all FAT copies.
    #[cfg(feature = "write")]
    pub async fn mark_bad<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u32) -> Result<()> {
        self.write_clus(rw, cluster as usize, Self::BAD_CLUSTER).await
    }

    /// Extend a cluster chain by appending new clusters.
    /// Returns the first cluster of the newly allocated portion.
    #[cfg(feature = "write")]
    pub async fn extend_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        last: u32,
        count: usize,
        hint: u32,
    ) -> Result<u32> {
        if count == 0 {
            return Ok(last);
        }

        let first_new = self.allocate_chain(rw, count, hint).await?;
        // Link the last cluster of existing chain to the new chain
        self.write_clus(rw, last as usize, first_new).await?;
        Ok(first_new)
    }
}

} // end io_transform!
