use alloc::{vec, vec::Vec};

use super::{DirEntry, Extent, IsoImage, Read, Seek, io};

/// Iterator over file chunks.
///
/// Yields chunks of up to `N` bytes from a file, handling multi-extent files.
///
pub struct FileChunkIterator<'a, DATA: Read + Seek, const N: usize> {
    pub(crate) image: &'a IsoImage<DATA>,
    pub(crate) extents: alloc::vec::Vec<Extent>,
    pub(crate) current_extent: usize,
    pub(crate) offset_in_extent: usize,
    pub(crate) bytes_remaining: u64,
    pub(crate) total_size: u64,
}

io_transform! {
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
    pub async fn next_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.bytes_remaining == 0 {
            return Ok(None);
        }
        if N == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunk size must be greater than zero",
            ));
        }

        let chunk_size = N.min(usize::try_from(self.bytes_remaining).unwrap_or(usize::MAX));
        let mut buffer = vec![0; chunk_size];
        let mut read = 0;

        while read < chunk_size {
            let extent = match self.extents.get(self.current_extent) {
                Some(e) => e,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "file extent chain ended before its declared size",
                    ));
                }
            };

            let remaining_in_extent = (extent.length as usize).saturating_sub(self.offset_in_extent);
            let to_read = (chunk_size - read).min(remaining_in_extent);

            let byte_offset = (extent.sector.0 as u64 * 2048) + self.offset_in_extent as u64;
            self.image
                .read_bytes_at(byte_offset, &mut buffer[read..read + to_read])
                .await?;

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
} // io_transform!

sync_only! {
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, sync::Arc, vec};
    use crate::{
        IsoImage,
        directory::FileFlags,
        read::PathSeparator,
        write::{options::*, *},
    };
    use std::io::Cursor;

    fn write_both_endian(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
    }

    #[test]
    fn should_read_multi_extent_file_in_chunks() {
        const CHUNK_SIZE: usize = 3000;
        const EXTENT_SIZE: usize = 4096;
        const SIZE: usize = EXTENT_SIZE * 2;

        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![File::File {
                name: Arc::new("TESTFILE".into()),
                contents: vec![0xAA; SIZE],
            }],
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

        let cursor = Cursor::new(vec![0u8; 1024 * 1024]);
        let output = IsoImageWriter::create(cursor, input, options).unwrap();
        let mut bytes = output.into_inner();
        let root_record = 16 * 2048 + 156;
        let root_sector = u32::from_le_bytes(
            bytes[root_record + 2..root_record + 6]
                .try_into()
                .unwrap(),
        );
        let mut file_record = root_sector as usize * 2048;
        file_record += bytes[file_record] as usize;
        file_record += bytes[file_record] as usize;
        let record_len = bytes[file_record] as usize;
        let original_extent = u32::from_le_bytes(
            bytes[file_record + 2..file_record + 6]
                .try_into()
                .unwrap(),
        );
        let mut continuation = bytes[file_record..file_record + record_len].to_vec();

        bytes[file_record + 25] |= FileFlags::NOT_FINAL.bits();
        write_both_endian(&mut bytes, file_record + 10, EXTENT_SIZE as u32);
        write_both_endian(&mut continuation, 2, original_extent + 2);
        write_both_endian(&mut continuation, 10, EXTENT_SIZE as u32);
        continuation[25] &= !FileFlags::NOT_FINAL.bits();
        let continuation_start = file_record + record_len;
        bytes[continuation_start..continuation_start + record_len]
            .copy_from_slice(&continuation);

        let image = IsoImage::open(Cursor::new(bytes)).expect("Failed to parse ISO image");
        let root_dir = image.root_dir();
        let iso_dir = root_dir.iter(&image);

        let mut entries = iso_dir.entries();
        entries.next().unwrap().expect("Failed to parse iso dir");
        entries.next().unwrap().expect("Failed to parse iso dir");
        let file = entries.next().unwrap().expect("Failed to parse iso file");
        assert_eq!(file.additional_extents.len(), 1);

        let mut zero_sized = image.read_file_chunked::<0>(&file).unwrap();
        assert_eq!(
            zero_sized.next_chunk().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let mut iter = image.read_file_chunked::<CHUNK_SIZE>(&file).unwrap();
        let mut total_read = 0;
        let mut chunk_count = 0;

        while let Some(chunk) = iter.next_chunk().unwrap() {
            chunk_count += 1;
            total_read += chunk.len();

            assert!(chunk.iter().all(|&b| b == 0xAA));
        }

        assert_eq!(total_read, SIZE);
        let expected_chunks = SIZE.div_ceil(CHUNK_SIZE);
        assert_eq!(chunk_count, expected_chunks);
        assert_eq!(iter.position(), SIZE as u64);
        assert_eq!(iter.total_size(), SIZE as u64);
    }
}
}
