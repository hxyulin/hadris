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
    /// Reads up to `buf.len()` bytes, or fewer at end-of-file. Use a small `buf` to
    /// stream incrementally; a larger `buf` allows more bytes per call (including
    /// contiguous-cluster bulk I/O when the chain is cached). The underlying
    /// [`Read`](crate::io::Read) / [`Seek`](crate::io::Seek) implementation can enforce
    /// alignment or transfer sizes as needed. Optional per-cluster buffering applies when
    /// enabled.
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Cap by caller buffer and bytes left in the file; actual I/O may be one or many
        // steps inside read_to_buf (cached contiguous runs vs. read_one_chunk loop).
        let max = buf.len().min(self.remaining());
        if max == 0 {
            return Ok(0);
        }
        self.read_to_buf(&mut buf[..max]).await
    }

    /// Read into `buf` until the buffer is full or the end of the file is reached.
    ///
    /// At most `buf.len().min(self.remaining())` bytes are read. Returns the number of
    /// bytes written to the start of `buf`.
    ///
    /// When the cluster chain is cached (`with_cached_chain`), contiguous runs of
    /// clusters are read in one seek+read per run (one I/O per fragment) instead of
    /// one per cluster.
    async fn read_to_buf(&mut self, buf: &mut [u8]) -> Result<usize> {
        let max = buf.len().min(self.remaining());
        if max == 0 {
            return Ok(0);
        }
        let buf = &mut buf[..max];

        #[cfg(feature = "alloc")]
        {
            let mut data = self.fs.data.lock();
            let cluster_size = data.cluster_size;
            let data_start = self.fs.info.data_start;

            // Cached chain: coalesce physically consecutive clusters into one seek+read per run.
            if let Some(ref chain) = self.cached_chain {
                let buf_len = buf.len();
                // Fast path: one seek + one read per contiguous run of clusters
                let mut total = 0usize;
                let mut chain_index = self.chain_index;
                let mut offset_in_cluster = self.offset_in_cluster;

                while total < buf_len && chain_index < chain.len() {
                    let first_cluster = chain[chain_index] as usize;
                    // Count contiguous run: chain[chain_index], chain[chain_index+1], ... while consecutive
                    let mut run_len = 1usize;
                    while chain_index + run_len < chain.len()
                        && chain[chain_index + run_len] == chain[chain_index + run_len - 1] + 1
                    {
                        run_len += 1;
                    }

                    // Bytes we can read from this run (from current offset to end of run)
                    let bytes_from_first = cluster_size.saturating_sub(offset_in_cluster);
                    let bytes_from_run = bytes_from_first
                        .saturating_add(run_len.saturating_sub(1).saturating_mul(cluster_size));
                    let to_read = bytes_from_run.min(buf_len - total);

                    if to_read == 0 {
                        break;
                    }

                    let seek_pos = Cluster(first_cluster)
                        .to_bytes(data_start, cluster_size)
                        .saturating_add(offset_in_cluster);
                    data.seek(SeekFrom::Start(seek_pos as u64)).await?;
                    data.read_exact(&mut buf[total..total + to_read]).await?;

                    total += to_read;
                    self.total_read += to_read;

                    // Advance by clusters we consumed
                    let new_offset = offset_in_cluster + to_read;
                    chain_index += new_offset / cluster_size;
                    offset_in_cluster = new_offset % cluster_size;
                }

                self.chain_index = chain_index;
                self.offset_in_cluster = offset_in_cluster;
                if chain_index < chain.len() {
                    self.cluster.0 = chain[chain_index] as usize;
                }
                drop(data);
                return Ok(total);
            }

            drop(data);
        }

        // No cached chain (or alloc off): walk the FAT one cluster-sized step at a time.
        let buf_len = buf.len();
        let mut total = 0usize;
        while total < buf_len {
            let n = self.read_one_chunk(&mut buf[total..]).await?;
            if n == 0 {
                break;
            }
            total += n;
        }
        Ok(total)
    }

    /// Read at most one contiguous span within the current cluster (or less if `buf` or the
    /// file ends sooner). Used by [`read_to_buf`](Self::read_to_buf) when the chain is not
    /// cached; advances to the next FAT cluster when the current one is exhausted.
    async fn read_one_chunk(&mut self, buf: &mut [u8]) -> Result<usize> {
        // End of logical file
        if self.total_read >= self.size {
            return Ok(0);
        }

        let mut data = self.fs.data.lock();
        let cluster_size = data.cluster_size;

        // Consumed the whole cluster: follow the chain to the next data cluster.
        if self.offset_in_cluster >= cluster_size {
            #[cfg(feature = "alloc")]
            {
                if let Some(ref chain) = self.cached_chain {
                    // Next cluster from the pre-read chain (no FAT table walk).
                    self.chain_index += 1;
                    if self.chain_index >= chain.len() {
                        return Ok(0); // End of file
                    }
                    self.cluster.0 = chain[self.chain_index] as usize;
                    self.offset_in_cluster = 0;
                    // Cluster buffer holds one cluster; must reload after moving.
                    if let Some(ref mut buffer) = self.cluster_buffer {
                        buffer.clear();
                    }
                } else {
                    // Resolve next cluster from the FAT on disk.
                    let next = self.fs.fat.next_cluster(data.deref_mut(), self.cluster.0).await?;
                    match next {
                        Some(cluster) => {
                            self.cluster.0 = cluster as usize;
                            self.offset_in_cluster = 0;
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

        // How much we can copy from the current cluster in this step.
        let bytes_left_in_cluster = cluster_size - self.offset_in_cluster;
        let bytes_left_in_file = self.size - self.total_read;
        let read_max = buf.len().min(bytes_left_in_cluster).min(bytes_left_in_file);

        if read_max == 0 {
            return Ok(0);
        }

        #[cfg(feature = "alloc")]
        let bytes_read = if let Some(ref mut buffer) = self.cluster_buffer {
            // Load whole cluster once, then serve reads from RAM.
            if buffer.is_empty() {
                let cluster_start = self.cluster.to_bytes(self.fs.info.data_start, cluster_size);
                data.seek(SeekFrom::Start(cluster_start as u64)).await?;

                buffer.resize(cluster_size, 0);
                data.read_exact(buffer).await?;
            }

            let src = &buffer[self.offset_in_cluster..self.offset_in_cluster + read_max];
            buf[..read_max].copy_from_slice(src);
            read_max
        } else {
            // Direct read at file offset within this cluster.
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

    /// Read all bytes from the current read position through the end of the file.
    ///
    /// Data is read starting at this reader's current offset in the file (the same
    /// position the next [`read`](Self::read) would use—not necessarily offset 0). Bytes
    /// already consumed by earlier [`read`](Self::read) or `read_to_vec` calls are not read
    /// again. The allocation size is [`remaining`](Self::remaining); bytes are read using the
    /// same internal bulk-read path as [`read`](Self::read).
    #[cfg(feature = "alloc")]
    pub async fn read_to_vec(&mut self) -> Result<Vec<u8>> {
        // One allocation sized to what's left from the current read cursor.
        let mut buf = alloc::vec![0u8; self.remaining()];
        let n = self.read_to_buf(&mut buf).await?;
        buf.truncate(n);
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
