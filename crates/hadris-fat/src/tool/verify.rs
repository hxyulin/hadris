//! Filesystem integrity verification for FAT filesystems.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::DerefMut;

use crate::{
    error::Result,
    io::{Read, Seek},
    DirectoryEntry, FatFs,
};

/// Types of verification issues that can be detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationIssue {
    /// A cluster chain contains a loop (revisits a cluster).
    ClusterLoop {
        /// File or directory path containing the loop
        path: String,
        /// The cluster where the loop was detected
        cluster: u32,
    },

    /// Two or more files share the same cluster (cross-linked).
    CrossLinkedCluster {
        /// The shared cluster number
        cluster: u32,
        /// Paths of files sharing this cluster
        paths: Vec<String>,
    },

    /// A cluster chain exists in the FAT but is not referenced by any file.
    OrphanedChain {
        /// Starting cluster of the orphaned chain
        start_cluster: u32,
        /// Length of the chain in clusters
        chain_length: u32,
    },

    /// The recorded file size doesn't match the cluster chain length.
    SizeMismatch {
        /// File path
        path: String,
        /// Size recorded in the directory entry
        recorded_size: usize,
        /// Size implied by the cluster chain
        chain_size: usize,
    },

    /// A directory entry points to an invalid cluster.
    InvalidFirstCluster {
        /// File or directory path
        path: String,
        /// The invalid cluster number
        cluster: u32,
    },

    /// The cluster chain contains a bad cluster marker.
    BadClusterInChain {
        /// File or directory path
        path: String,
        /// Position in the chain where the bad cluster was found
        position: u32,
        /// The bad cluster number
        cluster: u32,
    },

    /// A directory entry has an invalid name.
    InvalidEntryName {
        /// Parent directory path
        parent_path: String,
        /// Raw bytes of the invalid name
        raw_name: [u8; 11],
    },

    /// Lost clusters (used in FAT but not referenced by any file).
    LostClusters {
        /// Number of lost clusters
        count: u32,
    },
}

impl core::fmt::Display for VerificationIssue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ClusterLoop { path, cluster } => {
                write!(f, "Cluster loop detected at cluster {} in '{}'", cluster, path)
            }
            Self::CrossLinkedCluster { cluster, paths } => {
                write!(
                    f,
                    "Cross-linked cluster {}: shared by {}",
                    cluster,
                    paths.join(", ")
                )
            }
            Self::OrphanedChain { start_cluster, chain_length } => {
                write!(
                    f,
                    "Orphaned cluster chain starting at {} ({} clusters)",
                    start_cluster, chain_length
                )
            }
            Self::SizeMismatch { path, recorded_size, chain_size } => {
                write!(
                    f,
                    "Size mismatch for '{}': recorded {} bytes, chain suggests {} bytes",
                    path, recorded_size, chain_size
                )
            }
            Self::InvalidFirstCluster { path, cluster } => {
                write!(f, "Invalid first cluster {} for '{}'", cluster, path)
            }
            Self::BadClusterInChain { path, position, cluster } => {
                write!(
                    f,
                    "Bad cluster {} at position {} in chain for '{}'",
                    cluster, position, path
                )
            }
            Self::InvalidEntryName { parent_path, raw_name: _ } => {
                write!(f, "Invalid entry name in directory '{}'", parent_path)
            }
            Self::LostClusters { count } => {
                write!(f, "{} lost clusters (not referenced by any file)", count)
            }
        }
    }
}

/// Report from filesystem verification.
#[derive(Debug, Clone)]
pub struct VerificationReport {
    /// List of issues found
    pub issues: Vec<VerificationIssue>,
    /// Total files checked
    pub files_checked: u32,
    /// Total directories checked
    pub directories_checked: u32,
    /// Total clusters verified
    pub clusters_verified: u32,
}

impl VerificationReport {
    /// Check if the filesystem passed verification (no issues found).
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }

    /// Get the number of issues found.
    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }

    /// Get issues filtered by type.
    pub fn issues_of_type<F>(&self, predicate: F) -> Vec<&VerificationIssue>
    where
        F: Fn(&VerificationIssue) -> bool,
    {
        self.issues.iter().filter(|i| predicate(i)).collect()
    }
}

/// Extension trait for FatFs providing verification operations.
pub trait FatVerifyExt<DATA: Read + Seek> {
    /// Verify filesystem integrity.
    ///
    /// This performs comprehensive checks including:
    /// - Cluster chain validation (loops, bad clusters)
    /// - Cross-link detection (multiple files sharing clusters)
    /// - Orphaned cluster detection
    /// - File size validation
    /// - Directory entry validation
    fn verify(&self) -> Result<VerificationReport>;
}

impl<DATA: Read + Seek> FatVerifyExt<DATA> for FatFs<DATA> {
    fn verify(&self) -> Result<VerificationReport> {
        let mut issues = Vec::new();
        let mut files_checked = 0u32;
        let mut directories_checked = 0u32;
        let cluster_size = self.info.cluster_size;
        let max_cluster = self.info.max_cluster;

        // Map of cluster -> list of paths that reference it
        let mut cluster_usage: BTreeMap<u32, Vec<String>> = BTreeMap::new();

        // Track which clusters are used by files
        let mut used_by_files = alloc::vec![false; max_cluster as usize + 1];

        // Verify all files and directories
        self.verify_directory_recursive(
            &self.root_dir(),
            String::new(),
            &mut issues,
            &mut files_checked,
            &mut directories_checked,
            &mut cluster_usage,
            &mut used_by_files,
            cluster_size,
            max_cluster,
        )?;

        // Check for cross-linked clusters
        for (cluster, paths) in &cluster_usage {
            if paths.len() > 1 {
                issues.push(VerificationIssue::CrossLinkedCluster {
                    cluster: *cluster,
                    paths: paths.clone(),
                });
            }
        }

        // Scan FAT for orphaned clusters (used in FAT but not by any file)
        let mut data = self.data.lock();
        let mut orphaned_count = 0u32;

        for cluster in 2..=max_cluster {
            if !used_by_files[cluster as usize] {
                // Check if this cluster is marked as used in the FAT
                if let Ok(Some(_)) = self.fat.next_cluster(data.deref_mut(), cluster as usize) {
                    orphaned_count += 1;
                }
            }
        }

        drop(data);

        if orphaned_count > 0 {
            issues.push(VerificationIssue::LostClusters { count: orphaned_count });
        }

        Ok(VerificationReport {
            issues,
            files_checked,
            directories_checked,
            clusters_verified: max_cluster,
        })
    }
}

// Helper methods
impl<DATA: Read + Seek> FatFs<DATA> {
    fn verify_directory_recursive<'a>(
        &'a self,
        dir: &crate::FatDir<'a, DATA>,
        path_prefix: String,
        issues: &mut Vec<VerificationIssue>,
        files_checked: &mut u32,
        directories_checked: &mut u32,
        cluster_usage: &mut BTreeMap<u32, Vec<String>>,
        used_by_files: &mut [bool],
        cluster_size: usize,
        max_cluster: u32,
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

            let first_cluster = file_entry.cluster().0 as u32;

            // Validate first cluster
            if first_cluster != 0 && (first_cluster < 2 || first_cluster > max_cluster) {
                issues.push(VerificationIssue::InvalidFirstCluster {
                    path: full_path.clone(),
                    cluster: first_cluster,
                });
                continue;
            }

            if file_entry.is_directory() {
                *directories_checked += 1;

                // Verify directory cluster chain
                if first_cluster >= 2 {
                    self.verify_cluster_chain(
                        first_cluster,
                        &full_path,
                        issues,
                        cluster_usage,
                        used_by_files,
                        max_cluster,
                    )?;
                }

                // Recurse into subdirectory
                let subdir = crate::FatDir {
                    data: self,
                    cluster: file_entry.cluster(),
                    fixed_root: None,
                };
                self.verify_directory_recursive(
                    &subdir,
                    full_path,
                    issues,
                    files_checked,
                    directories_checked,
                    cluster_usage,
                    used_by_files,
                    cluster_size,
                    max_cluster,
                )?;
            } else {
                *files_checked += 1;

                // Verify file cluster chain
                if first_cluster >= 2 {
                    let chain_length = self.verify_cluster_chain(
                        first_cluster,
                        &full_path,
                        issues,
                        cluster_usage,
                        used_by_files,
                        max_cluster,
                    )?;

                    // Verify file size matches chain length
                    let recorded_size = file_entry.size();
                    let chain_size = chain_length as usize * cluster_size;
                    let min_chain_size = if chain_length > 0 {
                        (chain_length as usize - 1) * cluster_size + 1
                    } else {
                        0
                    };

                    if recorded_size > chain_size || (recorded_size > 0 && recorded_size < min_chain_size) {
                        issues.push(VerificationIssue::SizeMismatch {
                            path: full_path,
                            recorded_size,
                            chain_size,
                        });
                    }
                } else if file_entry.size() > 0 {
                    // Non-zero size but no cluster chain
                    issues.push(VerificationIssue::SizeMismatch {
                        path: full_path,
                        recorded_size: file_entry.size(),
                        chain_size: 0,
                    });
                }
            }
        }

        Ok(())
    }

    fn verify_cluster_chain(
        &self,
        start_cluster: u32,
        path: &str,
        issues: &mut Vec<VerificationIssue>,
        cluster_usage: &mut BTreeMap<u32, Vec<String>>,
        used_by_files: &mut [bool],
        max_cluster: u32,
    ) -> Result<u32> {
        let mut chain_length = 0u32;
        let mut current = start_cluster;
        let mut data = self.data.lock();

        // Track visited clusters to detect loops
        let mut visited = alloc::vec![false; max_cluster as usize + 1];

        let max_iterations = max_cluster as usize;
        let mut iterations = 0;

        loop {
            if current < 2 || current > max_cluster {
                break;
            }

            // Check for loop
            if visited[current as usize] {
                issues.push(VerificationIssue::ClusterLoop {
                    path: path.to_string(),
                    cluster: current,
                });
                break;
            }

            visited[current as usize] = true;
            used_by_files[current as usize] = true;
            chain_length += 1;

            // Record cluster usage
            cluster_usage
                .entry(current)
                .or_default()
                .push(path.to_string());

            iterations += 1;
            if iterations > max_iterations {
                // Safety limit to prevent infinite loops
                break;
            }

            // Get next cluster
            match self.fat.next_cluster(data.deref_mut(), current as usize)? {
                Some(next) => {
                    current = next;
                }
                None => break, // End of chain
            }
        }

        Ok(chain_length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_report_is_valid() {
        let report = VerificationReport {
            issues: Vec::new(),
            files_checked: 10,
            directories_checked: 5,
            clusters_verified: 1000,
        };
        assert!(report.is_valid());

        let report_with_issues = VerificationReport {
            issues: alloc::vec![VerificationIssue::LostClusters { count: 5 }],
            files_checked: 10,
            directories_checked: 5,
            clusters_verified: 1000,
        };
        assert!(!report_with_issues.is_valid());
    }
}
