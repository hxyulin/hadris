//! Statistics and fragmentation analysis for FAT filesystems.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::DerefMut;

use super::super::{
    dir::{DirectoryEntry, FatDir, FileEntry},
    fat_table::{Fat, Fat12, Fat16, Fat32, FatType},
    fs::FatFs,
    io::{Read, Seek},
};
use crate::error::Result;

/// Statistics about a FAT filesystem.
#[derive(Debug, Clone)]
pub struct FatStatistics {
    /// FAT type (FAT12, FAT16, or FAT32)
    pub fat_type: FatType,
    /// Total number of clusters in the filesystem
    pub total_clusters: u32,
    /// Number of free clusters
    pub free_clusters: u32,
    /// Number of used clusters
    pub used_clusters: u32,
    /// Number of bad clusters
    pub bad_clusters: u32,
    /// Number of reserved clusters (0 and 1)
    pub reserved_clusters: u32,
    /// Cluster size in bytes
    pub cluster_size: usize,
    /// Sector size in bytes
    pub sector_size: usize,
    /// Total filesystem capacity in bytes
    pub total_capacity: u64,
    /// Used space in bytes
    pub used_space: u64,
    /// Free space in bytes
    pub free_space: u64,
    /// Total number of files (not including directories)
    pub file_count: u32,
    /// Total number of directories
    pub directory_count: u32,
}

impl FatStatistics {
    /// Calculate the percentage of used space.
    pub fn used_percentage(&self) -> f64 {
        if self.total_capacity == 0 {
            0.0
        } else {
            (self.used_space as f64 / self.total_capacity as f64) * 100.0
        }
    }

    /// Calculate the percentage of free space.
    pub fn free_percentage(&self) -> f64 {
        if self.total_capacity == 0 {
            0.0
        } else {
            (self.free_space as f64 / self.total_capacity as f64) * 100.0
        }
    }
}

/// State of a single cluster in the FAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterState {
    /// Free cluster (value 0)
    Free,
    /// Reserved cluster (clusters 0 and 1, or reserved values)
    Reserved,
    /// Bad cluster
    Bad,
    /// Used cluster, contains next cluster number
    Used(u32),
    /// End of cluster chain
    EndOfChain,
}

/// Information about file fragmentation.
#[derive(Debug, Clone)]
pub struct FileFragmentInfo {
    /// File path
    pub path: String,
    /// File size in bytes
    pub size: usize,
    /// Number of fragments (contiguous extents)
    pub fragments: u32,
    /// Starting cluster of first fragment
    pub first_cluster: u32,
}

impl FileFragmentInfo {
    /// Calculate the fragmentation ratio (1.0 = not fragmented, >1.0 = fragmented).
    pub fn fragmentation_ratio(&self, cluster_size: usize) -> f64 {
        if self.size == 0 {
            return 1.0;
        }
        let ideal_clusters = self.size.div_ceil(cluster_size);
        if ideal_clusters == 0 {
            return 1.0;
        }
        self.fragments as f64 / ideal_clusters as f64
    }
}

/// Report on filesystem fragmentation.
#[derive(Debug, Clone)]
pub struct FragmentationReport {
    /// Total number of files analyzed
    pub total_files: u32,
    /// Number of fragmented files (more than 1 fragment)
    pub fragmented_files: u32,
    /// Total number of fragments across all files
    pub total_fragments: u32,
    /// Files with the most fragmentation, sorted by fragment count descending
    pub most_fragmented: Vec<FileFragmentInfo>,
    /// Average fragments per file
    pub average_fragments: f64,
    /// Fragmentation percentage (fragmented files / total files * 100)
    pub fragmentation_percentage: f64,
}

/// Extension trait for FatFs providing analysis operations.
pub trait FatAnalysisExt<DATA: Read + Seek> {
    /// Gather statistics about the filesystem.
    ///
    /// This scans the FAT table to count free, used, and bad clusters,
    /// and optionally scans the directory tree to count files and directories.
    fn statistics(&self) -> Result<FatStatistics>;

    /// Analyze filesystem fragmentation.
    ///
    /// This scans all files in the filesystem and reports on their fragmentation.
    /// The `max_files` parameter limits how many of the most fragmented files
    /// are included in the report (default: 10).
    fn fragmentation_report(&self, max_files: usize) -> Result<FragmentationReport>;

    /// Scan the FAT table and return the state of each cluster.
    fn scan_fat(&self) -> Result<Vec<ClusterState>>;

    /// Get the cluster chain for a file.
    fn get_cluster_chain(&self, first_cluster: u32) -> Result<Vec<u32>>;
}

impl<DATA: Read + Seek> FatAnalysisExt<DATA> for FatFs<DATA> {
    fn statistics(&self) -> Result<FatStatistics> {
        let fat_type = self.fat_type();
        let cluster_size = self.info.cluster_size;
        let mut data = self.data.lock();
        let sector_size = data.sector_size;

        // Scan FAT to count cluster states
        let mut free_clusters = 0u32;
        let mut used_clusters = 0u32;
        let mut bad_clusters = 0u32;
        let total_clusters = self.info.max_cluster;

        // Skip clusters 0 and 1 (reserved)
        for cluster in 2..=total_clusters {
            let state = self.read_cluster_state(data.deref_mut(), cluster)?;
            match state {
                ClusterState::Free => free_clusters += 1,
                ClusterState::Used(_) | ClusterState::EndOfChain => used_clusters += 1,
                ClusterState::Bad => bad_clusters += 1,
                ClusterState::Reserved => {}
            }
        }

        drop(data);

        // Count files and directories by scanning directory tree
        let (file_count, directory_count) = self.count_entries()?;

        let data_clusters = total_clusters - 1; // Exclude cluster 1 (cluster 0 is reserved)
        let total_capacity = data_clusters as u64 * cluster_size as u64;
        let used_space = used_clusters as u64 * cluster_size as u64;
        let free_space = free_clusters as u64 * cluster_size as u64;

        Ok(FatStatistics {
            fat_type,
            total_clusters,
            free_clusters,
            used_clusters,
            bad_clusters,
            reserved_clusters: 2, // Clusters 0 and 1
            cluster_size,
            sector_size,
            total_capacity,
            used_space,
            free_space,
            file_count,
            directory_count,
        })
    }

    fn fragmentation_report(&self, max_files: usize) -> Result<FragmentationReport> {
        let mut all_files = Vec::new();
        self.collect_files_recursive(&self.root_dir(), String::new(), &mut all_files)?;

        let mut total_fragments = 0u32;
        let mut fragmented_files = 0u32;

        // Analyze each file
        let mut file_infos: Vec<FileFragmentInfo> = Vec::new();
        for (path, entry) in &all_files {
            if entry.is_directory() {
                continue;
            }

            let first_cluster = entry.cluster().0 as u32;
            if first_cluster < 2 {
                // Empty file
                continue;
            }

            let chain = self.get_cluster_chain(first_cluster)?;
            let fragments = count_fragments(&chain);

            total_fragments += fragments;
            if fragments > 1 {
                fragmented_files += 1;
            }

            file_infos.push(FileFragmentInfo {
                path: path.clone(),
                size: entry.size(),
                fragments,
                first_cluster,
            });
        }

        // Sort by fragment count descending
        file_infos.sort_by(|a, b| b.fragments.cmp(&a.fragments));

        // Take the most fragmented files
        let most_fragmented: Vec<FileFragmentInfo> =
            file_infos.into_iter().take(max_files).collect();

        let total_files = all_files.iter().filter(|(_, e)| e.is_file()).count() as u32;
        let average_fragments = if total_files > 0 {
            total_fragments as f64 / total_files as f64
        } else {
            0.0
        };
        let fragmentation_percentage = if total_files > 0 {
            (fragmented_files as f64 / total_files as f64) * 100.0
        } else {
            0.0
        };

        Ok(FragmentationReport {
            total_files,
            fragmented_files,
            total_fragments,
            most_fragmented,
            average_fragments,
            fragmentation_percentage,
        })
    }

    fn scan_fat(&self) -> Result<Vec<ClusterState>> {
        let mut data = self.data.lock();
        let total_clusters = self.info.max_cluster;
        let mut states = Vec::with_capacity(total_clusters as usize + 1);

        // Cluster 0 and 1 are reserved
        states.push(ClusterState::Reserved);
        states.push(ClusterState::Reserved);

        for cluster in 2..=total_clusters {
            let state = self.read_cluster_state(data.deref_mut(), cluster)?;
            states.push(state);
        }

        Ok(states)
    }

    fn get_cluster_chain(&self, first_cluster: u32) -> Result<Vec<u32>> {
        let mut chain = Vec::new();
        let mut current = first_cluster;
        let mut data = self.data.lock();
        let max_clusters = self.info.max_cluster;

        // Prevent infinite loops
        let mut iterations = 0usize;
        let max_iterations = max_clusters as usize;

        while current >= 2 && current <= max_clusters {
            chain.push(current);
            iterations += 1;

            if iterations > max_iterations {
                // Likely a loop in the FAT
                break;
            }

            match self.fat.next_cluster(data.deref_mut(), current as usize)? {
                Some(next) => current = next,
                None => break,
            }
        }

        Ok(chain)
    }
}

// Helper implementations
impl<DATA: Read + Seek> FatFs<DATA> {
    /// Read the state of a single cluster from the FAT.
    fn read_cluster_state<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: u32,
    ) -> Result<ClusterState> {
        match &self.fat {
            Fat::Fat12(fat12) => {
                let entry = fat12.read_entry(reader, cluster as usize)?;
                Ok(classify_fat12_entry(entry))
            }
            Fat::Fat16(fat16) => {
                let entry = fat16.read_entry(reader, cluster as usize)?;
                Ok(classify_fat16_entry(entry))
            }
            Fat::Fat32(fat32) => {
                let entry = fat32.read_entry(reader, cluster as usize)?;
                Ok(classify_fat32_entry(entry))
            }
        }
    }

    /// Count files and directories in the filesystem.
    fn count_entries(&self) -> Result<(u32, u32)> {
        let mut files = 0u32;
        let mut dirs = 0u32;
        self.count_entries_recursive(&self.root_dir(), &mut files, &mut dirs)?;
        Ok((files, dirs))
    }

    fn count_entries_recursive<'a>(
        &'a self,
        dir: &FatDir<'a, DATA>,
        files: &mut u32,
        dirs: &mut u32,
    ) -> Result<()> {
        for entry in dir.entries() {
            let entry = entry?;
            let DirectoryEntry::Entry(file_entry) = entry;

            let name = file_entry.name();
            if name == "." || name == ".." {
                continue;
            }

            if file_entry.is_directory() {
                *dirs += 1;
                let subdir = FatDir {
                    data: self,
                    cluster: file_entry.cluster(),
                    fixed_root: None,
                };
                self.count_entries_recursive(&subdir, files, dirs)?;
            } else {
                *files += 1;
            }
        }
        Ok(())
    }

    /// Collect all files recursively with their paths.
    fn collect_files_recursive<'a>(
        &'a self,
        dir: &FatDir<'a, DATA>,
        path_prefix: String,
        files: &mut Vec<(String, FileEntry)>,
    ) -> Result<()> {
        for entry in dir.entries() {
            let entry = entry?;
            let DirectoryEntry::Entry(file_entry) = entry;

            let name = file_entry.name();
            if name == "." || name == ".." {
                continue;
            }

            let full_path = if path_prefix.is_empty() {
                format!("/{}", name)
            } else {
                format!("{}/{}", path_prefix, name)
            };

            if file_entry.is_directory() {
                let subdir = FatDir {
                    data: self,
                    cluster: file_entry.cluster(),
                    fixed_root: None,
                };
                self.collect_files_recursive(&subdir, full_path, files)?;
            } else {
                files.push((full_path, file_entry));
            }
        }
        Ok(())
    }
}

// FAT entry readers - these need to be added to the Fat12/16/32 implementations
impl Fat12 {
    /// Read a raw FAT12 entry.
    pub fn read_entry<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        let byte_offset = self.entry_byte_offset(cluster);
        reader.seek(super::super::io::SeekFrom::Start(byte_offset as u64))?;

        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes)?;

        let value = if cluster.is_multiple_of(2) {
            u16::from(bytes[0]) | (u16::from(bytes[1] & 0x0F) << 8)
        } else {
            (u16::from(bytes[0]) >> 4) | (u16::from(bytes[1]) << 4)
        };

        Ok(value)
    }
}

impl Fat16 {
    /// Read a raw FAT16 entry.
    pub fn read_entry<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        let offset = self.entry_offset(cluster);
        reader.seek(super::super::io::SeekFrom::Start(offset as u64))?;

        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes)?;

        Ok(u16::from_le_bytes(bytes))
    }
}

impl Fat32 {
    /// Read a raw FAT32 entry.
    pub fn read_entry<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u32> {
        let offset = self.entry_offset(cluster);
        reader.seek(super::super::io::SeekFrom::Start(offset as u64))?;

        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes)?;

        Ok(u32::from_le_bytes(bytes))
    }
}

/// Classify a FAT12 entry value.
fn classify_fat12_entry(entry: u16) -> ClusterState {
    let masked = entry & 0x0FFF;
    match masked {
        0x000 => ClusterState::Free,
        0x001 => ClusterState::Reserved,
        0xFF7 => ClusterState::Bad,
        0xFF8..=0xFFF => ClusterState::EndOfChain,
        n => ClusterState::Used(n as u32),
    }
}

/// Classify a FAT16 entry value.
fn classify_fat16_entry(entry: u16) -> ClusterState {
    match entry {
        0x0000 => ClusterState::Free,
        0x0001 => ClusterState::Reserved,
        0xFFF7 => ClusterState::Bad,
        0xFFF8..=0xFFFF => ClusterState::EndOfChain,
        n => ClusterState::Used(n as u32),
    }
}

/// Classify a FAT32 entry value.
fn classify_fat32_entry(entry: u32) -> ClusterState {
    let masked = entry & 0x0FFF_FFFF;
    match masked {
        0x0000_0000 => ClusterState::Free,
        0x0000_0001 => ClusterState::Reserved,
        0x0FFF_FFF7 => ClusterState::Bad,
        0x0FFF_FFF8..=0x0FFF_FFFF => ClusterState::EndOfChain,
        n => ClusterState::Used(n),
    }
}

/// Count the number of contiguous fragments in a cluster chain.
fn count_fragments(chain: &[u32]) -> u32 {
    if chain.is_empty() {
        return 0;
    }
    if chain.len() == 1 {
        return 1;
    }

    let mut fragments = 1u32;
    for window in chain.windows(2) {
        if window[1] != window[0] + 1 {
            fragments += 1;
        }
    }
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_fragments() {
        assert_eq!(count_fragments(&[]), 0);
        assert_eq!(count_fragments(&[5]), 1);
        assert_eq!(count_fragments(&[5, 6, 7]), 1);
        assert_eq!(count_fragments(&[5, 6, 10]), 2);
        assert_eq!(count_fragments(&[5, 10, 15]), 3);
        assert_eq!(count_fragments(&[5, 6, 7, 10, 11, 15]), 3);
    }

    #[test]
    fn test_classify_fat12() {
        assert_eq!(classify_fat12_entry(0x000), ClusterState::Free);
        assert_eq!(classify_fat12_entry(0x001), ClusterState::Reserved);
        assert_eq!(classify_fat12_entry(0xFF7), ClusterState::Bad);
        assert_eq!(classify_fat12_entry(0xFF8), ClusterState::EndOfChain);
        assert_eq!(classify_fat12_entry(0xFFF), ClusterState::EndOfChain);
        assert_eq!(classify_fat12_entry(0x123), ClusterState::Used(0x123));
    }

    #[test]
    fn test_classify_fat16() {
        assert_eq!(classify_fat16_entry(0x0000), ClusterState::Free);
        assert_eq!(classify_fat16_entry(0x0001), ClusterState::Reserved);
        assert_eq!(classify_fat16_entry(0xFFF7), ClusterState::Bad);
        assert_eq!(classify_fat16_entry(0xFFF8), ClusterState::EndOfChain);
        assert_eq!(classify_fat16_entry(0xFFFF), ClusterState::EndOfChain);
        assert_eq!(classify_fat16_entry(0x1234), ClusterState::Used(0x1234));
    }

    #[test]
    fn test_classify_fat32() {
        assert_eq!(classify_fat32_entry(0x0000_0000), ClusterState::Free);
        assert_eq!(classify_fat32_entry(0x0000_0001), ClusterState::Reserved);
        assert_eq!(classify_fat32_entry(0x0FFF_FFF7), ClusterState::Bad);
        assert_eq!(classify_fat32_entry(0x0FFF_FFF8), ClusterState::EndOfChain);
        assert_eq!(classify_fat32_entry(0x0FFF_FFFF), ClusterState::EndOfChain);
        assert_eq!(
            classify_fat32_entry(0x0012_3456),
            ClusterState::Used(0x0012_3456)
        );
    }
}
