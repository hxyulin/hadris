//! Read operations for FAT filesystems.

io_transform! {

use core::ops::DerefMut;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::{FatError, Result};
use super::{
    fs::FatFs, dir::FileEntry,
    io::{Cluster, ClusterLike, Read, Seek, SeekFrom},
};

/// A reader for file content in a FAT filesystem.
///
/// This struct provides a `Read` implementation that follows the cluster chain
/// to read file contents.
///
/// # Buffering
///
/// When the `alloc` feature is enabled, the reader can optionally buffer data
/// to reduce the number of seek and read operations:
///
/// - [`with_buffer`](Self::with_buffer): Enable cluster-level buffering. Each cluster
///   is read entirely into memory and subsequent reads are served from the buffer.
///
/// - [`with_cached_chain`](Self::with_cached_chain): Pre-cache the entire cluster chain.
///   This is useful for small files where you want to avoid repeated FAT lookups.
pub struct FileReader<'a, DATA: Read + Seek> {
    fs: &'a FatFs<DATA>,
    cluster: Cluster<usize>,
    /// Offset within the current cluster
    offset_in_cluster: usize,
    /// Total bytes read so far
    total_read: usize,
    /// Total size of the file
    size: usize,
    /// Optional cluster buffer for reduced I/O
    #[cfg(feature = "alloc")]
    cluster_buffer: Option<Vec<u8>>,
    /// Pre-cached cluster chain (optional)
    #[cfg(feature = "alloc")]
    cached_chain: Option<Vec<u32>>,
    /// Current index in the cached chain
    #[cfg(feature = "alloc")]
    chain_index: usize,
}

impl<'a, DATA: Read + Seek> FileReader<'a, DATA> {
    /// Create a new FileReader for a file entry.
    ///
    /// Returns an error if the entry is a directory.
    pub fn new(fs: &'a FatFs<DATA>, entry: &FileEntry) -> Result<Self> {
        if entry.is_directory() {
            return Err(FatError::NotAFile);
        }

        Ok(Self {
            fs,
            cluster: entry.cluster(),
            offset_in_cluster: 0,
            total_read: 0,
            size: entry.size(),
            #[cfg(feature = "alloc")]
            cluster_buffer: None,
            #[cfg(feature = "alloc")]
            cached_chain: None,
            #[cfg(feature = "alloc")]
            chain_index: 0,
        })
    }

    /// Returns the total size of the file in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns the number of bytes remaining to be read.
    pub fn remaining(&self) -> usize {
        self.size.saturating_sub(self.total_read)
    }

    /// Enable cluster-level buffering.
    ///
    /// When enabled, each cluster is read entirely into memory on first access,
    /// and subsequent reads within that cluster are served from the buffer.
    /// This reduces the number of seek operations at the cost of memory usage.
    ///
    /// Memory usage: One cluster size (typically 4KB to 64KB).
    #[cfg(feature = "alloc")]
    pub fn with_buffer(mut self) -> Self {
        self.cluster_buffer = Some(Vec::new());
        self
    }

    /// Pre-cache the entire cluster chain.
    ///
    /// This reads the entire FAT chain for the file into memory, eliminating
    /// the need for FAT lookups during sequential reads. This is most beneficial
    /// for fragmented files or when performing many random seeks.
    ///
    /// Memory usage: 4 bytes per cluster in the file.
    #[cfg(feature = "alloc")]
    pub async fn with_cached_chain(mut self) -> Result<Self> {
        if self.cluster.0 < 2 {
            // Empty file, no chain to cache
            self.cached_chain = Some(Vec::new());
            return Ok(self);
        }

        let max_clusters = self.fs.info.max_cluster as usize;
        let mut data = self.fs.data.lock();
        let chain = self
            .fs
            .fat
            .read_chain(data.deref_mut(), self.cluster.0 as u32, max_clusters)
            .await?;
        drop(data);

        self.cached_chain = Some(chain);
        self.chain_index = 0;
        Ok(self)
    }

    /// Read data from the file.
    ///
    /// This method follows the FAT cluster chain to read file contents.
    /// If buffering is enabled, reads are served from the buffer when possible.
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Check if we've reached the end of the file
        if self.total_read >= self.size {
            return Ok(0);
        }

        let mut data = self.fs.data.lock();
        let cluster_size = data.cluster_size;

        // Check if we need to move to the next cluster
        if self.offset_in_cluster >= cluster_size {
            #[cfg(feature = "alloc")]
            {
                if let Some(ref chain) = self.cached_chain {
                    // Use cached chain
                    self.chain_index += 1;
                    if self.chain_index >= chain.len() {
                        return Ok(0); // End of file
                    }
                    self.cluster.0 = chain[self.chain_index] as usize;
                    self.offset_in_cluster = 0;
                    // Invalidate buffer for new cluster
                    if let Some(ref mut buffer) = self.cluster_buffer {
                        buffer.clear();
                    }
                } else {
                    // Fall back to FAT lookup
                    let next = self.fs.fat.next_cluster(data.deref_mut(), self.cluster.0).await?;
                    match next {
                        Some(cluster) => {
                            self.cluster.0 = cluster as usize;
                            self.offset_in_cluster = 0;
                            // Invalidate buffer for new cluster
                            if let Some(ref mut buffer) = self.cluster_buffer {
                                buffer.clear();
                            }
                        }
                        None => return Ok(0), // End of cluster chain
                    }
                }
            }

            #[cfg(not(feature = "alloc"))]
            {
                let next = self.fs.fat.next_cluster(data.deref_mut(), self.cluster.0).await?;
                match next {
                    Some(cluster) => {
                        self.cluster.0 = cluster as usize;
                        self.offset_in_cluster = 0;
                    }
                    None => return Ok(0), // End of cluster chain
                }
            }
        }

        // Calculate how much we can read
        let bytes_left_in_cluster = cluster_size - self.offset_in_cluster;
        let bytes_left_in_file = self.size - self.total_read;
        let read_max = buf.len().min(bytes_left_in_cluster).min(bytes_left_in_file);

        if read_max == 0 {
            return Ok(0);
        }

        // Read with buffering if enabled
        #[cfg(feature = "alloc")]
        let bytes_read = if let Some(ref mut buffer) = self.cluster_buffer {
            // Fill buffer if empty
            if buffer.is_empty() {
                let cluster_start = self.cluster.to_bytes(self.fs.info.data_start, cluster_size);
                data.seek(SeekFrom::Start(cluster_start as u64)).await?;

                buffer.resize(cluster_size, 0);
                data.read_exact(buffer).await?;
            }

            // Read from buffer
            let src = &buffer[self.offset_in_cluster..self.offset_in_cluster + read_max];
            buf[..read_max].copy_from_slice(src);
            read_max
        } else {
            // Direct read (no buffering)
            let seek_pos = self.cluster.to_bytes(self.fs.info.data_start, cluster_size)
                + self.offset_in_cluster;
            data.seek(SeekFrom::Start(seek_pos as u64)).await?;
            data.read(&mut buf[..read_max]).await?
        };

        #[cfg(not(feature = "alloc"))]
        let bytes_read = {
            let seek_pos = self.cluster.to_bytes(self.fs.info.data_start, cluster_size)
                + self.offset_in_cluster;
            data.seek(SeekFrom::Start(seek_pos as u64)).await?;
            data.read(&mut buf[..read_max]).await?
        };

        self.offset_in_cluster += bytes_read;
        self.total_read += bytes_read;

        Ok(bytes_read)
    }

    /// Read the entire file contents into a vector.
    #[cfg(feature = "alloc")]
    pub async fn read_to_vec(&mut self) -> Result<Vec<u8>> {
        let mut buf = alloc::vec![0u8; self.remaining()];
        let mut total = 0;
        while total < buf.len() {
            let n = self.read(&mut buf[total..]).await?;
            if n == 0 {
                break;
            }
            total += n;
        }
        buf.truncate(total);
        Ok(buf)
    }
}

/// Extension trait for FatFs to read files directly.
pub trait FatFsReadExt<DATA: Read + Seek> {
    /// Create a reader for a file entry.
    fn read_file<'a>(&'a self, entry: &FileEntry) -> Result<FileReader<'a, DATA>>;
}

impl<DATA: Read + Seek> FatFsReadExt<DATA> for FatFs<DATA> {
    fn read_file<'a>(&'a self, entry: &FileEntry) -> Result<FileReader<'a, DATA>> {
        FileReader::new(self, entry)
    }
}

} // end io_transform!
