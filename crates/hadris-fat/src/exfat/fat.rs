//! exFAT File Allocation Table implementation.
//!
//! In exFAT, the FAT is only used for fragmented files (files without the
//! NoFatChain flag set). Contiguous files can be read directly from the
//! bitmap without consulting the FAT.

use core::mem::size_of;

use crate::error::{FatError, Result};
#[cfg(feature = "write")]
use crate::io::Write;
use crate::io::{Read, Seek, SeekFrom};

use super::ExFatInfo;

/// exFAT FAT table entry constants.
pub struct ExFatTable {
    /// FAT offset in bytes from start of volume
    fat_offset: u64,
    /// FAT length in bytes
    fat_length: u64,
    /// Number of FAT copies (1 or 2)
    fat_count: u8,
    /// Maximum valid cluster number
    max_cluster: u32,
}

impl ExFatTable {
    /// Free cluster marker
    pub const FREE_CLUSTER: u32 = 0x00000000;
    /// End of chain marker (0xFFFFFFFF)
    pub const END_OF_CHAIN: u32 = 0xFFFFFFFF;
    /// Bad cluster marker (0xFFFFFFF7)
    pub const BAD_CLUSTER: u32 = 0xFFFFFFF7;
    /// First media descriptor value
    pub const MEDIA_DESCRIPTOR: u32 = 0xFFFFFFF8;
    /// First valid data cluster
    pub const FIRST_DATA_CLUSTER: u32 = 2;

    /// Create a new FAT table accessor.
    pub fn new(info: &ExFatInfo) -> Self {
        Self {
            fat_offset: info.fat_offset,
            fat_length: info.fat_length,
            fat_count: info.fat_count,
            // Cluster count + 2 (for reserved entries 0 and 1)
            max_cluster: info.cluster_count + Self::FIRST_DATA_CLUSTER - 1,
        }
    }

    /// Read the FAT entry for a cluster.
    pub fn read_entry<DATA: Read + Seek>(&self, data: &mut DATA, cluster: u32) -> Result<u32> {
        self.validate_cluster(cluster)?;

        let offset = self.fat_offset + (cluster as u64) * (size_of::<u32>() as u64);
        data.seek(SeekFrom::Start(offset))?;

        let mut buf = [0u8; 4];
        data.read_exact(&mut buf)?;

        Ok(u32::from_le_bytes(buf))
    }

    /// Write a FAT entry for a cluster.
    #[cfg(feature = "write")]
    pub fn write_entry<DATA: Read + Write + Seek>(
        &self,
        data: &mut DATA,
        cluster: u32,
        value: u32,
    ) -> Result<()> {
        self.validate_cluster(cluster)?;

        // Write to all FAT copies
        for fat_idx in 0..self.fat_count {
            let offset = self.fat_offset
                + (fat_idx as u64) * self.fat_length
                + (cluster as u64) * (size_of::<u32>() as u64);
            data.seek(SeekFrom::Start(offset))?;
            data.write_all(&value.to_le_bytes())?;
        }

        Ok(())
    }

    /// Get the next cluster in a chain.
    ///
    /// Returns `None` if this is the end of the chain.
    pub fn next_cluster<DATA: Read + Seek>(
        &self,
        data: &mut DATA,
        cluster: u32,
    ) -> Result<Option<u32>> {
        let entry = self.read_entry(data, cluster)?;

        // Check for end of chain
        if entry == Self::END_OF_CHAIN || entry >= Self::MEDIA_DESCRIPTOR {
            return Ok(None);
        }

        // Check for bad cluster
        if entry == Self::BAD_CLUSTER {
            return Err(FatError::BadCluster { cluster });
        }

        // Check for free cluster (shouldn't happen in a valid chain)
        if entry == Self::FREE_CLUSTER {
            return Err(FatError::UnexpectedEndOfChain { cluster });
        }

        // Validate the next cluster
        self.validate_cluster(entry)?;

        Ok(Some(entry))
    }

    /// Allocate a single cluster.
    ///
    /// The cluster is marked as end-of-chain.
    /// Returns the allocated cluster number.
    #[cfg(feature = "write")]
    pub fn allocate_cluster<DATA: Read + Write + Seek>(
        &self,
        data: &mut DATA,
        hint: u32,
    ) -> Result<u32> {
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to end
        for cluster in start..=self.max_cluster {
            let entry = self.read_entry(data, cluster)?;
            if entry == Self::FREE_CLUSTER {
                self.write_entry(data, cluster, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from beginning to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_entry(data, cluster)?;
            if entry == Self::FREE_CLUSTER {
                self.write_entry(data, cluster, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        Err(FatError::NoFreeSpace)
    }

    /// Allocate a chain of clusters.
    ///
    /// Returns the first cluster of the chain.
    #[cfg(feature = "write")]
    pub fn allocate_chain<DATA: Read + Write + Seek>(
        &self,
        data: &mut DATA,
        count: u32,
        hint: u32,
    ) -> Result<u32> {
        if count == 0 {
            return Err(FatError::NoFreeSpace);
        }

        let first = self.allocate_cluster(data, hint)?;
        let mut prev = first;

        for _ in 1..count {
            let next = self.allocate_cluster(data, prev + 1)?;
            self.write_entry(data, prev, next)?;
            prev = next;
        }

        Ok(first)
    }

    /// Free a cluster chain starting at the specified cluster.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub fn free_chain<DATA: Read + Write + Seek>(
        &self,
        data: &mut DATA,
        start: u32,
    ) -> Result<u32> {
        let mut count = 0u32;
        let mut current = start;

        loop {
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            let next = self.read_entry(data, current)?;
            self.write_entry(data, current, Self::FREE_CLUSTER)?;
            count += 1;

            if next == Self::END_OF_CHAIN
                || next >= Self::MEDIA_DESCRIPTOR
                || next == Self::BAD_CLUSTER
                || next == Self::FREE_CLUSTER
            {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Extend a cluster chain by appending new clusters.
    ///
    /// Returns the first cluster of the newly allocated portion.
    #[cfg(feature = "write")]
    pub fn extend_chain<DATA: Read + Write + Seek>(
        &self,
        data: &mut DATA,
        last: u32,
        count: u32,
        hint: u32,
    ) -> Result<u32> {
        if count == 0 {
            return Ok(last);
        }

        let first_new = self.allocate_chain(data, count, hint)?;
        self.write_entry(data, last, first_new)?;
        Ok(first_new)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain.
    /// All following clusters are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub fn truncate_chain<DATA: Read + Write + Seek>(
        &self,
        data: &mut DATA,
        cluster: u32,
    ) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        let next = self.read_entry(data, cluster)?;
        self.write_entry(data, cluster, Self::END_OF_CHAIN)?;

        if next != Self::END_OF_CHAIN
            && next < Self::MEDIA_DESCRIPTOR
            && next >= Self::FIRST_DATA_CLUSTER
            && next <= self.max_cluster
        {
            self.free_chain(data, next)
        } else {
            Ok(0)
        }
    }

    /// Validate that a cluster number is within bounds.
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

    /// Get the maximum valid cluster number
    pub fn max_cluster(&self) -> u32 {
        self.max_cluster
    }
}
