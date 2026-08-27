use alloc::vec::Vec;
use hadris_io::{Read, Seek};

use crate::{IsoImage, io, read::{DirEntry, Extent}};

/// Iterator over file chunks.
///
/// Yields chunks of up to `N` bytes from a file, handling multi-extent files.
///
/// # Example
/// ```
/// use hadris_iso::IsoImage;
/// use hadris_io::Cursor;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let image = IsoImage::open(cursor)?;
/// let entry = image.find_path("large_file.bin").await?.unwrap();
///
/// // Read file in 4KB chunks
/// let mut iter = image.read_file_chunked::<4096>(&entry).await?;
/// while let Some(chunk) = iter.next_chunk()? {
///     // Process chunk (up to 4096 bytes)
///     println!("Read {} bytes", chunk.len());
/// }
/// # Ok(())
/// # }
/// ```
pub struct FileChunkIterator<'a, DATA: Read + Seek, const N: usize> {
    pub(crate) image: &'a IsoImage<DATA>,
    pub(crate) extents: alloc::vec::Vec<Extent>,
    pub(crate) current_extent: usize,
    pub(crate) offset_in_extent: usize,
    pub(crate) bytes_remaining: u64,
    pub(crate) total_size: u64,
}

impl<'a, DATA: Read + Seek, const N: usize> FileChunkIterator<'a, DATA, N> {
    /// Creates a new chunked iterator for a file entry.
    pub(crate) fn new(image: &'a IsoImage<DATA>, entry: &DirEntry) -> Self {
        let extents: Vec<Extent> = entry.extents().collect();
        let bytes_remaining = entry.total_size();
        let total_size = entry.total_size();

        Self {
            image,
            extents,
            current_extent: 0,
            offset_in_extent: 0,
            bytes_remaining,
            total_size
        }
    }

    /// Returns the next chunk of data.
    ///
    /// Returns `Ok(Some(Vec<u8>))` with up to N bytes, `Ok(None)` at EOF.
    pub fn next_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.bytes_remaining == 0 {
            return Ok(None);
        }

        let chunk_size = N.min(self.bytes_remaining as usize);
        let mut buffer = Vec::with_capacity(chunk_size);
        buffer.resize(chunk_size, 0);
        let mut read = 0;

        while read < chunk_size {
            let extent = match self.extents.get(self.current_extent) {
                Some(e) => e,
                None => break,
            };

            let remaining_in_extent = (extent.length as usize).saturating_sub(self.offset_in_extent);
            let to_read = (chunk_size - read).min(remaining_in_extent);

            let byte_offset = (extent.sector.0 as u64 * 2048) + self.offset_in_extent as u64;
            self.image.read_bytes_at(byte_offset, &mut buffer[read..read + to_read])?;

            read += to_read;
            self.offset_in_extent += to_read;
            self.bytes_remaining -= to_read as u64;

            if self.offset_in_extent >= extent.length as usize {
                self.current_extent += 1;
                self.offset_in_extent = 0;
            }
        }

        buffer.truncate(read);
        Ok(Some(buffer))
    }

    /// Returns the total size of the file in bytes.
    pub fn total_size(&self) -> u64 {
        self.total_size 
    }

    /// Returns the current position in the file (bytes read so far).
    pub fn position(&self) -> u64 {
        self.total_size - self.bytes_remaining
    }
}


#[cfg(test)]
mod tests {
    use core::assert_eq;
    use std::{io::Cursor};
    use crate::{IsoImage, read::PathSeparator, write::{options::*, *}};
    use super::*;
    use alloc::{string::ToString, sync::Arc, vec};
    use hadris_io::SeekFrom;

    #[test]
    fn should_read_multi_extent_file_in_chunks() {
        const CHUNK_SIZE: usize = 4096;
        const SIZE: usize = 1024 * 1024 * 1024 * 4;

        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![
                    File::File {
                        name: Arc::new("TESTFILE".into()),
                        contents: FileContent::Test { size: SIZE, pattern: 0xAA
                    }
                }
            ],
        };
        let options = IsoFormatOptions {
            volume_name: "EMPTY".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                ..Default::default()
            },
            strict_charset: false,
        };

        // ISO metadata overhead: volume descriptors, path tables, directory records, etc.
        // 21 sectors of 2048 bytes = 43,008 bytes (~42 KiB)
        let cursor = Cursor::new(vec![0u8; SIZE + 2048 * 21]);
        let mut output = IsoImageWriter::create(cursor, input, options).unwrap();
     
        output.seek(SeekFrom::Start(0)).expect("Failed to verify ISO image");
        let image = IsoImage::open(output).expect("Failed to parse ISO image");
        
        let root_dir = image.root_dir();
        let iso_dir = root_dir.iter(&image);

        let mut entries = iso_dir.entries();
        entries.next().unwrap().expect("Failed to parse iso dir");
        entries.next().unwrap().expect("Failed to parse iso dir");
        let file = entries.next().unwrap().expect("Failed to parse iso file");
        assert!(!file.additional_extents.is_empty());

        let mut iter = image.read_file_chunked::<CHUNK_SIZE>(&file).unwrap();
        let mut total_read = 0;
        let mut chunk_count = 0;

        while let Some(chunk) = iter.next_chunk().unwrap() {
            chunk_count += 1;
            total_read += chunk.len();

            assert!(chunk.iter().all(|&b| b == 0xAA), 
                "Chunk {} contains unexpected bytes", chunk_count);
        }

        assert_eq!(total_read, SIZE, "Total bytes read should match file size");
        
        let expected_chunks = SIZE.div_ceil(CHUNK_SIZE);
        assert_eq!(chunk_count, expected_chunks, 
            "Chunk count should match expected");
        
        assert_eq!(iter.position(), SIZE as u64, "Position should be at EOF");
        assert_eq!(iter.total_size(), SIZE as u64, "Total size should remain unchanged")
    }

}
