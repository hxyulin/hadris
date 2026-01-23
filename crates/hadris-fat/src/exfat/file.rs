//! exFAT File Reader/Writer implementation.
//!
//! Provides streaming access to file contents, handling both
//! contiguous and fragmented files.

use core::cmp::min;

use crate::error::{FatError, Result};
use crate::io::{error_from_kind, ErrorKind, Read, Seek, SeekFrom};
#[cfg(feature = "write")]
use crate::io::Write;

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
    fn read(&mut self, buf: &mut [u8]) -> crate::io::Result<usize> {
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
            let to_read = min(remaining_in_cluster, min(remaining_in_file, remaining_in_buf));

            if to_read == 0 {
                break;
            }

            // Read from the cluster
            let offset = info.cluster_to_offset(self.current_cluster) + self.cluster_offset as u64;
            self.fs.read_at(offset, &mut buf[total_read..total_read + to_read])
                .map_err(|_| error_from_kind(ErrorKind::Other))?;

            total_read += to_read;
            self.cluster_offset += to_read;
            self.position += to_read as u64;
        }

        Ok(total_read)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> crate::io::Result<()> {
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
    fn seek(&mut self, pos: SeekFrom) -> crate::io::Result<u64> {
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

    fn stream_position(&mut self) -> crate::io::Result<u64> {
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
    /// Current cluster being written
    current_cluster: u32,
    /// Byte offset within current cluster
    cluster_offset: usize,
    /// Current position within the file
    position: u64,
    /// Whether the file started as contiguous
    was_contiguous: bool,
    /// Cluster index (for contiguous files)
    cluster_index: u32,
    /// New data length (to update on finish)
    new_length: u64,
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
            current_cluster: entry.first_cluster,
            cluster_offset: 0,
            position: 0,
            was_contiguous: entry.no_fat_chain,
            cluster_index: 0,
            new_length: entry.valid_data_length,
            entry,
        })
    }

    /// Get the current position within the file.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Finish writing and update the directory entry.
    ///
    /// This must be called after writing to update the file's metadata.
    pub fn finish(self) -> Result<()> {
        // TODO: Update the directory entry with new length and timestamps
        // This requires modifying the entry in the parent directory
        Ok(())
    }
}

#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> Write for ExFatFileWriter<'_, DATA> {
    fn write(&mut self, buf: &[u8]) -> crate::io::Result<usize> {
        let info = self.fs.info();
        let cluster_size = info.bytes_per_cluster;
        let mut total_written = 0;

        while total_written < buf.len() {
            // Check if we need to move to the next cluster
            if self.cluster_offset >= cluster_size {
                // Try to get the next cluster
                let next = if self.was_contiguous {
                    // For contiguous files, try to use the next adjacent cluster
                    let next_cluster = self.current_cluster + 1;
                    if info.is_valid_cluster(next_cluster) {
                        // Check if it's free using the bitmap
                        // For now, just use the next cluster
                        Some(next_cluster)
                    } else {
                        None
                    }
                } else {
                    // Follow FAT chain or allocate new cluster
                    match self.fs.next_cluster(self.current_cluster) {
                        Ok(next) => next,
                        Err(_) => None,
                    }
                };

                match next {
                    Some(cluster) => {
                        self.current_cluster = cluster;
                        self.cluster_index += 1;
                    }
                    None => {
                        // Need to allocate a new cluster
                        // TODO: Implement cluster allocation for writes
                        return Ok(total_written);
                    }
                }
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
            self.fs.write_at(offset, &buf[total_written..total_written + to_write])
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

    fn flush(&mut self) -> crate::io::Result<()> {
        self.fs.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> crate::io::Result<()> {
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
