//! exFAT File Reader/Writer implementation.
//!
//! Provides streaming access to file contents, handling both
//! contiguous and fragmented files.

use core::cmp::min;

use crate::error::{FatError, Result};
#[cfg(feature = "write")]
use crate::io::Write;
use crate::io::{ErrorKind, Read, Seek, SeekFrom, error_from_kind};

use super::entry::ExFatFileEntry;
use super::fs::ExFatFs;

/// A reader for exFAT file contents.
pub struct ExFatFileReader<'a, DATA: Read + Seek> {
    /// Reference to the filesystem
    fs: &'a ExFatFs<DATA>,
    /// First cluster of the file
    first_cluster: u32,
    /// Current cluster being read
    current_cluster: u32,
    /// Byte offset within current cluster
    cluster_offset: usize,
    /// Current position within the file
    position: u64,
    /// Valid data length (actual file content size)
    valid_length: u64,
    /// Total data length (allocated size)
    #[allow(dead_code)]
    data_length: u64,
    /// Whether the file is stored contiguously
    is_contiguous: bool,
    /// Cluster index (for contiguous files)
    cluster_index: u32,
}

impl<'a, DATA: Read + Seek> ExFatFileReader<'a, DATA> {
    /// Create a new file reader from a file entry.
    pub fn new(fs: &'a ExFatFs<DATA>, entry: &ExFatFileEntry) -> Result<Self> {
        if entry.is_directory() {
            return Err(FatError::NotAFile);
        }

        Ok(Self {
            fs,
            first_cluster: entry.first_cluster,
            current_cluster: entry.first_cluster,
            cluster_offset: 0,
            position: 0,
            valid_length: entry.valid_data_length,
            data_length: entry.data_length,
            is_contiguous: entry.no_fat_chain,
            cluster_index: 0,
        })
    }

    /// Get the current position within the file.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Get the file size (valid data length).
    pub fn size(&self) -> u64 {
        self.valid_length
    }

    /// Get remaining bytes to read.
    pub fn remaining(&self) -> u64 {
        self.valid_length.saturating_sub(self.position)
    }
}

impl<DATA: Read + Seek> Read for ExFatFileReader<'_, DATA> {
    fn read(&mut self, buf: &mut [u8]) -> crate::io::IoResult<usize> {
        if self.position >= self.valid_length {
            return Ok(0);
        }

        let info = self.fs.info();
        let cluster_size = info.bytes_per_cluster;
        let mut total_read = 0;

        while total_read < buf.len() && self.position < self.valid_length {
            // Check if we need to move to the next cluster
            if self.cluster_offset >= cluster_size {
                if self.is_contiguous {
                    // Contiguous file: just increment cluster index
                    self.cluster_index += 1;
                    self.current_cluster = self.first_cluster + self.cluster_index;
                } else {
                    // Fragmented file: follow FAT chain
                    match self.fs.next_cluster(self.current_cluster) {
                        Ok(Some(next)) => self.current_cluster = next,
                        Ok(None) => break, // End of chain
                        Err(_e) => return Err(error_from_kind(ErrorKind::Other)),
                    }
                }
                self.cluster_offset = 0;
            }

            // Calculate how much to read from this cluster
            let remaining_in_cluster = cluster_size - self.cluster_offset;
            let remaining_in_file = (self.valid_length - self.position) as usize;
            let remaining_in_buf = buf.len() - total_read;
            let to_read = min(
                remaining_in_cluster,
                min(remaining_in_file, remaining_in_buf),
            );

            if to_read == 0 {
                break;
            }

            // Read from the cluster
            let offset = info.cluster_to_offset(self.current_cluster) + self.cluster_offset as u64;
            self.fs
                .read_at(offset, &mut buf[total_read..total_read + to_read])
                .map_err(|_| error_from_kind(ErrorKind::Other))?;

            total_read += to_read;
            self.cluster_offset += to_read;
            self.position += to_read as u64;
        }

        Ok(total_read)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> crate::io::IoResult<()> {
        let mut total_read = 0;
        while total_read < buf.len() {
            match self.read(&mut buf[total_read..])? {
                0 => return Err(error_from_kind(ErrorKind::UnexpectedEof)),
                n => total_read += n,
            }
        }
        Ok(())
    }
}

impl<DATA: Read + Seek> Seek for ExFatFileReader<'_, DATA> {
    fn seek(&mut self, pos: SeekFrom) -> crate::io::IoResult<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.valid_length as i64 + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };

        if new_pos < 0 {
            return Err(error_from_kind(ErrorKind::InvalidInput));
        }

        let new_pos = new_pos as u64;
        if new_pos > self.valid_length {
            return Err(error_from_kind(ErrorKind::InvalidInput));
        }

        // Calculate the new cluster position
        let info = self.fs.info();
        let cluster_size = info.bytes_per_cluster as u64;
        let cluster_index = (new_pos / cluster_size) as u32;
        let cluster_offset = (new_pos % cluster_size) as usize;

        if self.is_contiguous {
            // For contiguous files, we can calculate the cluster directly
            self.current_cluster = self.first_cluster + cluster_index;
            self.cluster_index = cluster_index;
        } else {
            // For fragmented files, we need to follow the FAT chain
            // This is inefficient for large seeks, but necessary
            self.current_cluster = self.first_cluster;
            for _ in 0..cluster_index {
                match self.fs.next_cluster(self.current_cluster) {
                    Ok(Some(next)) => self.current_cluster = next,
                    Ok(None) => return Err(error_from_kind(ErrorKind::InvalidInput)),
                    Err(_) => return Err(error_from_kind(ErrorKind::Other)),
                }
            }
        }

        self.cluster_offset = cluster_offset;
        self.position = new_pos;

        Ok(new_pos)
    }

    fn stream_position(&mut self) -> crate::io::IoResult<u64> {
        Ok(self.position)
    }
}

/// A writer for exFAT file contents.
#[cfg(feature = "write")]
pub struct ExFatFileWriter<'a, DATA: Read + Write + Seek> {
    /// Reference to the filesystem
    fs: &'a ExFatFs<DATA>,
    /// The file entry being written to
    entry: ExFatFileEntry,
    /// First cluster (may change if file was empty)
    first_cluster: u32,
    /// Current cluster being written
    current_cluster: u32,
    /// Previous cluster (for linking in FAT)
    prev_cluster: Option<u32>,
    /// Byte offset within current cluster
    cluster_offset: usize,
    /// Current position within the file
    position: u64,
    /// Whether the file is still contiguous
    is_contiguous: bool,
    /// Cluster index (for contiguous files)
    cluster_index: u32,
    /// New data length (to update on finish)
    new_length: u64,
    /// Allocated data length
    allocated_length: u64,
}

#[cfg(feature = "write")]
impl<'a, DATA: Read + Write + Seek> ExFatFileWriter<'a, DATA> {
    /// Create a new file writer from a file entry.
    pub fn new(fs: &'a ExFatFs<DATA>, entry: ExFatFileEntry) -> Result<Self> {
        if entry.is_directory() {
            return Err(FatError::NotAFile);
        }

        Ok(Self {
            fs,
            first_cluster: entry.first_cluster,
            current_cluster: entry.first_cluster,
            prev_cluster: None,
            cluster_offset: 0,
            position: 0,
            is_contiguous: entry.no_fat_chain,
            cluster_index: 0,
            new_length: entry.valid_data_length,
            allocated_length: entry.data_length,
            entry,
        })
    }

    /// Get the current position within the file.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Get the number of bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.new_length
    }

    /// Allocate a new cluster, linking it to the previous one if needed.
    fn allocate_next_cluster(&mut self) -> Result<u32> {
        let hint = self.current_cluster.saturating_add(1);
        let new_cluster = self.fs.allocate_cluster(hint)?;

        // If we have a previous cluster, we're no longer contiguous
        // (unless the new cluster is adjacent)
        if let Some(prev) = self.prev_cluster {
            if new_cluster != prev + 1 {
                self.is_contiguous = false;
            }
        }

        // Update allocated length
        let cluster_size = self.fs.info().bytes_per_cluster as u64;
        self.allocated_length += cluster_size;

        Ok(new_cluster)
    }

    /// Finish writing and update the directory entry.
    ///
    /// This must be called after writing to update the file's metadata
    /// (size, data length, first cluster) and recalculate the entry set checksum.
    pub fn finish(self) -> Result<()> {
        self.fs.update_entry_size(
            &self.entry,
            self.new_length,
            self.allocated_length,
            self.first_cluster,
            self.is_contiguous,
        )?;
        self.fs.sync_bitmap()?;
        let _ = self.fs.flush();
        Ok(())
    }
}

#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> Write for ExFatFileWriter<'_, DATA> {
    fn write(&mut self, buf: &[u8]) -> crate::io::IoResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let info = self.fs.info();
        let cluster_size = info.bytes_per_cluster;
        let mut total_written = 0;

        while total_written < buf.len() {
            // Check if we need a cluster (empty file or need to advance)
            if self.current_cluster == 0 || self.cluster_offset >= cluster_size {
                // Allocate or get next cluster
                let new_cluster = if self.current_cluster == 0 {
                    // Empty file - allocate first cluster
                    match self.fs.allocate_cluster(2) {
                        Ok(c) => {
                            self.first_cluster = c;
                            self.allocated_length = cluster_size as u64;
                            c
                        }
                        Err(_) => return Ok(total_written),
                    }
                } else if self.is_contiguous {
                    // Try to use next adjacent cluster
                    let next = self.current_cluster + 1;
                    if info.is_valid_cluster(next) {
                        // Check if the next cluster is available
                        match self.fs.is_cluster_allocated(next) {
                            Ok(false) => {
                                // Allocate it
                                match self.fs.allocate_cluster(next) {
                                    Ok(c) if c == next => {
                                        self.allocated_length += cluster_size as u64;
                                        c
                                    }
                                    Ok(_) | Err(_) => {
                                        // Not contiguous anymore - need to convert to FAT chain
                                        self.is_contiguous = false;
                                        match self.allocate_next_cluster() {
                                            Ok(c) => c,
                                            Err(_) => return Ok(total_written),
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Already allocated - convert to FAT chain
                                self.is_contiguous = false;
                                match self.allocate_next_cluster() {
                                    Ok(c) => c,
                                    Err(_) => return Ok(total_written),
                                }
                            }
                        }
                    } else {
                        // Past end of volume
                        return Ok(total_written);
                    }
                } else {
                    // Follow FAT chain or allocate new
                    match self.fs.next_cluster(self.current_cluster) {
                        Ok(Some(next)) => next,
                        Ok(None) | Err(_) => match self.allocate_next_cluster() {
                            Ok(c) => c,
                            Err(_) => return Ok(total_written),
                        },
                    }
                };

                self.prev_cluster = if self.current_cluster != 0 {
                    Some(self.current_cluster)
                } else {
                    None
                };
                self.current_cluster = new_cluster;
                self.cluster_index += 1;
                self.cluster_offset = 0;
            }

            // Calculate how much to write to this cluster
            let remaining_in_cluster = cluster_size - self.cluster_offset;
            let remaining_in_buf = buf.len() - total_written;
            let to_write = min(remaining_in_cluster, remaining_in_buf);

            if to_write == 0 {
                break;
            }

            // Write to the cluster
            let offset = info.cluster_to_offset(self.current_cluster) + self.cluster_offset as u64;
            self.fs
                .write_at(offset, &buf[total_written..total_written + to_write])
                .map_err(|_| error_from_kind(ErrorKind::Other))?;

            total_written += to_write;
            self.cluster_offset += to_write;
            self.position += to_write as u64;

            if self.position > self.new_length {
                self.new_length = self.position;
            }
        }

        Ok(total_written)
    }

    fn flush(&mut self) -> crate::io::IoResult<()> {
        self.fs.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> crate::io::IoResult<()> {
        let mut written = 0;
        while written < buf.len() {
            match self.write(&buf[written..])? {
                0 => return Err(error_from_kind(ErrorKind::WriteZero)),
                n => written += n,
            }
        }
        Ok(())
    }
}
