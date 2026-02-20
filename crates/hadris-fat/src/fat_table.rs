io_transform! {

use core::mem::size_of;

use crate::error::{FatError, Result};
#[cfg(feature = "write")]
use super::io::Write;
use super::io::{Read, Seek, SeekFrom};

pub enum Fat {
    Fat12(Fat12),
    Fat16(Fat16),
    Fat32(Fat32),
}

impl Fat {
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

    /// Get the FAT type for informational purposes
    pub fn fat_type(&self) -> FatType {
        match self {
            Self::Fat12(_) => FatType::Fat12,
            Self::Fat16(_) => FatType::Fat16,
            Self::Fat32(_) => FatType::Fat32,
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
}

/// FAT filesystem type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat12,
    Fat16,
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
    #[allow(dead_code)]
    count: usize,
    max_cluster: u16,
}

impl Fat12 {
    /// Mask for the 12-bit cluster number
    const ENTRY_MASK: u16 = 0x0FFF;
    /// End of chain markers: 0x0FF8 - 0x0FFF indicate end of cluster chain
    const END_OF_CHAIN_MIN: u16 = 0x0FF8;
    /// Bad cluster marker
    const BAD_CLUSTER: u16 = 0x0FF7;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u16 = 2;

    pub fn new(start: usize, size: usize, count: usize, max_cluster: u16) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self {
            start,
            size,
            count,
            max_cluster,
        }
    }

    /// Calculate byte offset for a FAT12 entry.
    /// FAT12 packs 2 entries into 3 bytes: entry N starts at byte (N * 3) / 2
    pub(crate) fn entry_byte_offset(&self, cluster: usize) -> usize {
        self.start + (cluster * 3) / 2
    }

    async fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        let byte_offset = self.entry_byte_offset(cluster);
        reader.seek(SeekFrom::Start(byte_offset as u64)).await?;

        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes).await?;

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

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u16) -> bool {
        value >= Self::END_OF_CHAIN_MIN
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u16) -> bool {
        value == Self::BAD_CLUSTER
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u16) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        if cluster > self.max_cluster {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        Ok(())
    }

    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let entry = self.read_clus(reader, cluster).await? & Self::ENTRY_MASK;

        if Self::is_end_of_chain(entry) {
            return Ok(None);
        }

        if Self::is_bad_cluster(entry) {
            return Err(FatError::BadCluster {
                cluster: cluster as u32,
            });
        }

        self.validate_cluster(entry)?;

        Ok(Some(entry as u32))
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u16 = 0x0000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u16 = 0x0FF8;

    /// Write a FAT12 entry at the specified cluster index to a specific FAT copy.
    #[cfg(feature = "write")]
    async fn write_clus_at<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
        value: u16,
        fat_index: usize,
    ) -> Result<()> {
        let byte_offset = self.start + fat_index * self.size + (cluster * 3) / 2;
        rw.seek(SeekFrom::Start(byte_offset as u64)).await?;

        // Read existing bytes (we need to preserve the other half)
        let mut bytes = [0u8; 2];
        rw.read_exact(&mut bytes).await?;

        // Modify the appropriate bits
        if cluster.is_multiple_of(2) {
            // Even: modify lower 8 bits of bytes[0] and lower 4 bits of bytes[1]
            bytes[0] = value as u8;
            bytes[1] = (bytes[1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
        } else {
            // Odd: modify upper 4 bits of bytes[0] and all of bytes[1]
            bytes[0] = (bytes[0] & 0x0F) | ((value << 4) as u8);
            bytes[1] = (value >> 4) as u8;
        }

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

        Err(FatError::NoFreeSpace)
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
}

/// FAT16 table implementation.
pub struct Fat16 {
    start: usize,
    size: usize,
    #[allow(dead_code)]
    count: usize,
    max_cluster: u16,
}

impl Fat16 {
    /// End of chain markers: 0xFFF8 - 0xFFFF indicate end of cluster chain
    const END_OF_CHAIN_MIN: u16 = 0xFFF8;
    /// Bad cluster marker
    const BAD_CLUSTER: u16 = 0xFFF7;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u16 = 2;

    pub fn new(start: usize, size: usize, count: usize, max_cluster: u16) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self {
            start,
            size,
            count,
            max_cluster,
        }
    }

    pub(crate) fn entry_offset(&self, cluster: usize) -> usize {
        debug_assert!(cluster * size_of::<u16>() < self.size);
        self.start + cluster * size_of::<u16>()
    }

    async fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        reader.seek(SeekFrom::Start(self.entry_offset(cluster) as u64)).await?;
        let mut data = 0u16;
        reader.read_exact(bytemuck::bytes_of_mut(&mut data)).await?;
        Ok(u16::from_le(data))
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u16) -> bool {
        value >= Self::END_OF_CHAIN_MIN
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u16) -> bool {
        value == Self::BAD_CLUSTER
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u16) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        if cluster > self.max_cluster {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        Ok(())
    }

    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let entry = self.read_clus(reader, cluster).await?;

        if Self::is_end_of_chain(entry) {
            return Ok(None);
        }

        if Self::is_bad_cluster(entry) {
            return Err(FatError::BadCluster {
                cluster: cluster as u32,
            });
        }

        self.validate_cluster(entry)?;

        Ok(Some(entry as u32))
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u16 = 0x0000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u16 = 0xFFF8;

    /// Write a cluster entry to the FAT table at the specified FAT copy
    #[cfg(feature = "write")]
    async fn write_clus_at<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u16,
        fat_index: usize,
    ) -> Result<()> {
        let offset = self.start + fat_index * self.size + cluster * size_of::<u16>();
        writer.seek(SeekFrom::Start(offset as u64)).await?;
        writer.write_all(&value.to_le_bytes()).await?;
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

        Err(FatError::NoFreeSpace)
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
}

pub struct Fat32 {
    start: usize,
    size: usize,
    #[allow(dead_code)]
    count: usize,
    max_cluster: u32,
}

impl Fat32 {
    /// Mask for the 28-bit cluster number (upper 4 bits are reserved)
    const ENTRY_MASK: u32 = 0x0FFF_FFFF;
    /// End of chain markers: 0x0FFFFFF8 - 0x0FFFFFFF indicate end of cluster chain
    const END_OF_CHAIN_MIN: u32 = 0x0FFF_FFF8;
    /// Bad cluster marker
    const BAD_CLUSTER: u32 = 0x0FFF_FFF7;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u32 = 2;

    pub fn new(start: usize, size: usize, count: usize, max_cluster: u32) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self {
            start,
            size,
            count,
            max_cluster,
        }
    }

    pub(crate) fn entry_offset(&self, cluster: usize) -> usize {
        debug_assert!(cluster * size_of::<u32>() < self.size);
        self.start + cluster * size_of::<u32>()
    }

    async fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u32> {
        reader.seek(SeekFrom::Start(self.entry_offset(cluster) as u64)).await?;
        let mut data = 0u32;
        reader.read_exact(bytemuck::bytes_of_mut(&mut data)).await?;
        Ok(data)
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u32) -> bool {
        value >= Self::END_OF_CHAIN_MIN
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u32) -> bool {
        value == Self::BAD_CLUSTER
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u32) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.max_cluster,
            });
        }
        if cluster > self.max_cluster {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.max_cluster,
            });
        }
        Ok(())
    }

    pub async fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        // Read the FAT entry for this cluster
        let raw_entry = self.read_clus(reader, cluster).await?;
        let entry = raw_entry & Self::ENTRY_MASK;

        // Check for end of chain
        if Self::is_end_of_chain(entry) {
            return Ok(None);
        }

        // Check for bad cluster
        if Self::is_bad_cluster(entry) {
            return Err(FatError::BadCluster {
                cluster: cluster as u32,
            });
        }

        // Validate the next cluster is in bounds
        self.validate_cluster(entry)?;

        Ok(Some(entry))
    }

    /// Write a cluster entry to the FAT table at the specified FAT copy
    #[cfg(feature = "write")]
    async fn write_clus_at<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u32,
        fat_index: usize,
    ) -> Result<()> {
        let offset = self.start + fat_index * self.size + cluster * size_of::<u32>();
        writer.seek(SeekFrom::Start(offset as u64)).await?;
        writer.write_all(&value.to_le_bytes()).await?;
        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub async fn write_clus<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u32,
    ) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(writer, cluster, value, i).await?;
        }
        Ok(())
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u32 = 0x00000000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u32 = 0x0FFFFFF8;

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

        Err(FatError::NoFreeSpace)
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
            return Err(FatError::NoFreeSpace);
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
