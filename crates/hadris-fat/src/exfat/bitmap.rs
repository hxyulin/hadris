//! exFAT Allocation Bitmap implementation.
//!
//! exFAT uses an allocation bitmap to track which clusters are in use.
//! Unlike FAT12/16/32, the bitmap provides O(1) allocation status lookup.
//!
//! The allocation bitmap is stored in the cluster heap as a special file
//! identified by the Allocation Bitmap directory entry (type 0x81).

use alloc::vec::Vec;

use crate::error::{FatError, Result};
use crate::io::{Read, Seek, SeekFrom};
#[cfg(feature = "write")]
use crate::io::Write;

use super::ExFatInfo;

/// Allocation bitmap for tracking cluster usage.
///
/// Each bit represents one cluster: 0 = free, 1 = allocated.
/// Bit 0 of byte 0 corresponds to cluster 2 (the first data cluster).
pub struct AllocationBitmap {
    /// First cluster containing the bitmap
    first_cluster: u32,
    /// Total size of bitmap in bytes
    size: u64,
    /// Cached bitmap data (loaded on demand)
    data: Vec<u8>,
    /// Total number of clusters in the filesystem
    cluster_count: u32,
    /// Whether the bitmap is contiguous (NoFatChain flag)
    is_contiguous: bool,
}

impl AllocationBitmap {
    /// Minimum cluster number (clusters start at 2)
    const FIRST_DATA_CLUSTER: u32 = 2;

    /// Create a new allocation bitmap.
    ///
    /// # Arguments
    /// * `first_cluster` - First cluster containing the bitmap data
    /// * `size` - Total size of the bitmap in bytes
    /// * `cluster_count` - Total number of clusters in the filesystem
    /// * `is_contiguous` - Whether the bitmap is stored contiguously
    pub fn new(first_cluster: u32, size: u64, cluster_count: u32, is_contiguous: bool) -> Self {
        Self {
            first_cluster,
            size,
            data: Vec::new(),
            cluster_count,
            is_contiguous,
        }
    }

    /// Load the bitmap data from disk.
    pub fn load<DATA: Read + Seek>(&mut self, data: &mut DATA, info: &ExFatInfo) -> Result<()> {
        if self.is_contiguous {
            // Read contiguous bitmap
            let offset = info.cluster_to_offset(self.first_cluster);
            data.seek(SeekFrom::Start(offset))?;

            self.data.resize(self.size as usize, 0);
            data.read_exact(&mut self.data)?;
        } else {
            // TODO: Handle fragmented bitmap by following FAT chain
            // For now, treat as contiguous - most implementations use contiguous bitmaps
            let offset = info.cluster_to_offset(self.first_cluster);
            data.seek(SeekFrom::Start(offset))?;

            self.data.resize(self.size as usize, 0);
            data.read_exact(&mut self.data)?;
        }

        Ok(())
    }

    /// Check if a cluster is allocated.
    ///
    /// Returns `true` if the cluster is in use, `false` if free.
    pub fn is_allocated(&self, cluster: u32) -> Result<bool> {
        self.validate_cluster(cluster)?;

        let index = (cluster - Self::FIRST_DATA_CLUSTER) as usize;
        let byte_index = index / 8;
        let bit_index = index % 8;

        if byte_index >= self.data.len() {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.cluster_count + Self::FIRST_DATA_CLUSTER - 1,
            });
        }

        Ok((self.data[byte_index] & (1 << bit_index)) != 0)
    }

    /// Set a cluster's allocation status.
    #[cfg(feature = "write")]
    pub fn set_allocated(&mut self, cluster: u32, allocated: bool) -> Result<()> {
        self.validate_cluster(cluster)?;

        let index = (cluster - Self::FIRST_DATA_CLUSTER) as usize;
        let byte_index = index / 8;
        let bit_index = index % 8;

        if byte_index >= self.data.len() {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.cluster_count + Self::FIRST_DATA_CLUSTER - 1,
            });
        }

        if allocated {
            self.data[byte_index] |= 1 << bit_index;
        } else {
            self.data[byte_index] &= !(1 << bit_index);
        }

        Ok(())
    }

    /// Find N contiguous free clusters starting from a hint.
    ///
    /// Returns the first cluster of the contiguous range, or None if not found.
    #[cfg(feature = "write")]
    pub fn find_contiguous_free(&self, count: u32, hint: u32) -> Result<Option<u32>> {
        if count == 0 {
            return Ok(None);
        }

        let max_cluster = self.cluster_count + Self::FIRST_DATA_CLUSTER - 1;
        let start = hint.max(Self::FIRST_DATA_CLUSTER).min(max_cluster);

        // Search from hint to end
        if let Some(found) = self.find_contiguous_in_range(start, max_cluster + 1, count)? {
            return Ok(Some(found));
        }

        // Wrap around: search from beginning to hint
        if start > Self::FIRST_DATA_CLUSTER {
            if let Some(found) = self.find_contiguous_in_range(Self::FIRST_DATA_CLUSTER, start, count)? {
                return Ok(Some(found));
            }
        }

        Ok(None)
    }

    /// Find a single free cluster starting from a hint.
    #[cfg(feature = "write")]
    pub fn find_free_cluster(&self, hint: u32) -> Result<Option<u32>> {
        self.find_contiguous_free(1, hint)
    }

    /// Count the number of free clusters.
    pub fn free_cluster_count(&self) -> u32 {
        let mut count = 0u32;

        for byte in &self.data {
            // Count zero bits
            count += (8 - byte.count_ones()) as u32;
        }

        // Adjust for any padding bits at the end
        let total_bits = self.data.len() * 8;
        let extra_bits = total_bits - self.cluster_count as usize;
        count = count.saturating_sub(extra_bits as u32);

        count
    }

    /// Write the bitmap back to disk.
    #[cfg(feature = "write")]
    pub fn flush<DATA: Read + Write + Seek>(&self, data: &mut DATA, info: &ExFatInfo) -> Result<()> {
        let offset = info.cluster_to_offset(self.first_cluster);
        data.seek(SeekFrom::Start(offset))?;
        data.write_all(&self.data)?;
        Ok(())
    }

    /// Validate that a cluster number is within bounds.
    fn validate_cluster(&self, cluster: u32) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.cluster_count + Self::FIRST_DATA_CLUSTER - 1,
            });
        }
        if cluster >= self.cluster_count + Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.cluster_count + Self::FIRST_DATA_CLUSTER - 1,
            });
        }
        Ok(())
    }

    /// Find contiguous free clusters within a range.
    #[cfg(feature = "write")]
    fn find_contiguous_in_range(&self, start: u32, end: u32, count: u32) -> Result<Option<u32>> {
        let mut run_start = None;
        let mut run_length = 0u32;

        for cluster in start..end {
            if !self.is_allocated(cluster)? {
                if run_start.is_none() {
                    run_start = Some(cluster);
                }
                run_length += 1;

                if run_length >= count {
                    return Ok(run_start);
                }
            } else {
                run_start = None;
                run_length = 0;
            }
        }

        Ok(None)
    }

    /// Get the first cluster of the bitmap
    pub fn first_cluster(&self) -> u32 {
        self.first_cluster
    }

    /// Get the size of the bitmap in bytes
    pub fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_bitmap_bit_operations() {
        let mut bitmap = AllocationBitmap::new(2, 4, 32, true);
        bitmap.data = vec![0b10101010, 0b01010101, 0b11110000, 0b00001111];

        // Cluster 2 maps to bit 0 of byte 0
        assert!(!bitmap.is_allocated(2).unwrap()); // bit 0 = 0
        assert!(bitmap.is_allocated(3).unwrap());  // bit 1 = 1
        assert!(!bitmap.is_allocated(4).unwrap()); // bit 2 = 0
        assert!(bitmap.is_allocated(5).unwrap());  // bit 3 = 1
    }
}
