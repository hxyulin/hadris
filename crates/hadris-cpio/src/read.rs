use crate::entry::CpioEntryHeader;
use crate::error::{CpioError, Result};
use crate::header::{CpioMagic, RawNewcHeader, HEADER_SIZE, TRAILER_NAME};
use hadris_io::Read;

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Compute the number of padding bytes needed to align `offset` to a 4-byte boundary.
fn align4_padding(offset: u64) -> u64 {
    (4 - (offset % 4)) % 4
}

/// A decoded CPIO entry that borrows its filename from a caller-provided buffer.
///
/// This is the no-alloc variant returned by [`CpioReader::next_entry_with_buf`].
/// For an owned variant that allocates, see [`CpioEntryOwned`].
#[derive(Debug)]
pub struct CpioEntry<'a> {
    header: CpioEntryHeader,
    magic: CpioMagic,
    name: &'a [u8],
    entry_offset: u64,
}

impl<'a> CpioEntry<'a> {
    /// Returns the decoded header fields.
    pub fn header(&self) -> &CpioEntryHeader {
        &self.header
    }

    /// Returns the archive format (newc or newc+CRC).
    pub fn magic(&self) -> CpioMagic {
        self.magic
    }

    /// Returns the filename as raw bytes.
    pub fn name(&self) -> &[u8] {
        self.name
    }

    /// Returns the filename as a UTF-8 string, if valid.
    pub fn name_str(&self) -> core::result::Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.name)
    }

    /// Returns the file type extracted from the mode bits.
    pub fn file_type(&self) -> crate::mode::FileType {
        self.header.file_type()
    }

    /// Byte offset from the start of the archive where this entry's header begins.
    pub fn entry_offset(&self) -> u64 {
        self.entry_offset
    }

    /// Returns the file data size in bytes.
    pub fn file_size(&self) -> u32 {
        self.header.filesize
    }
}

/// A decoded CPIO entry that owns its filename (requires `alloc`).
///
/// This is the allocating variant returned by [`CpioReader::next_entry_alloc`].
/// For a zero-alloc variant, see [`CpioEntry`].
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct CpioEntryOwned {
    header: CpioEntryHeader,
    magic: CpioMagic,
    name: Vec<u8>,
    entry_offset: u64,
}

#[cfg(feature = "alloc")]
impl CpioEntryOwned {
    /// Returns the decoded header fields.
    pub fn header(&self) -> &CpioEntryHeader {
        &self.header
    }

    /// Returns the archive format (newc or newc+CRC).
    pub fn magic(&self) -> CpioMagic {
        self.magic
    }

    /// Returns the filename as raw bytes.
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// Returns the filename as a UTF-8 string, if valid.
    pub fn name_str(&self) -> core::result::Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(&self.name)
    }

    /// Returns the file type extracted from the mode bits.
    pub fn file_type(&self) -> crate::mode::FileType {
        self.header.file_type()
    }

    /// Byte offset from the start of the archive where this entry's header begins.
    pub fn entry_offset(&self) -> u64 {
        self.entry_offset
    }

    /// Returns the file data size in bytes.
    pub fn file_size(&self) -> u32 {
        self.header.filesize
    }
}

/// Streaming CPIO archive reader.
///
/// Reads entries sequentially from any [`Read`] source. Entries are yielded
/// one at a time; after obtaining an entry you must either read or skip its
/// data before advancing to the next entry.
///
/// Two iteration APIs are provided:
/// - [`next_entry_with_buf`](CpioReader::next_entry_with_buf) — no-alloc, uses a caller-provided buffer
/// - [`next_entry_alloc`](CpioReader::next_entry_alloc) — allocates a `Vec` for each filename (requires `alloc`)
pub struct CpioReader<R> {
    reader: R,
    offset: u64,
    finished: bool,
}

impl<R: Read> CpioReader<R> {
    /// Create a new reader wrapping the given source.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            offset: 0,
            finished: false,
        }
    }

    /// Returns the current byte offset in the archive.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Read the next entry header and name using a caller-provided buffer.
    ///
    /// Returns `Ok(None)` when the TRAILER!!! sentinel is reached.
    /// The `name_buf` must be large enough to hold the filename (without NUL terminator).
    pub fn next_entry_with_buf<'buf>(
        &mut self,
        name_buf: &'buf mut [u8],
    ) -> Result<Option<CpioEntry<'buf>>> {
        if self.finished {
            return Ok(None);
        }

        let entry_offset = self.offset;

        // Read the 110-byte header
        let raw = RawNewcHeader::parse(&mut self.reader)?;
        self.offset += HEADER_SIZE as u64;

        let magic = raw.magic().ok_or_else(|| {
            let mut found = [0u8; 6];
            found.copy_from_slice(raw.magic_bytes());
            CpioError::InvalidMagic { found }
        })?;

        let header = CpioEntryHeader::from_raw(&raw)?;
        let namesize = raw.namesize()? as usize;

        if namesize == 0 {
            return Err(CpioError::InvalidFilename);
        }

        // Read filename (including NUL terminator)
        let name_with_nul_len = namesize;
        if name_with_nul_len > name_buf.len() + 1 {
            return Err(CpioError::InvalidFilename);
        }

        // Read name bytes into a stack buffer (namesize includes the NUL)
        // We need a temporary buffer for the NUL-terminated name, then copy without NUL
        let name_len = namesize - 1; // without NUL
        if name_len > name_buf.len() {
            return Err(CpioError::InvalidFilename);
        }

        // Read the name portion
        self.reader.read_exact(&mut name_buf[..name_len])?;
        self.offset += name_len as u64;

        // Read and discard the NUL terminator
        let mut nul = [0u8; 1];
        self.reader.read_exact(&mut nul)?;
        self.offset += 1;

        // Skip padding to align (header + namesize) to 4-byte boundary
        let header_plus_name = HEADER_SIZE as u64 + namesize as u64;
        let pad = align4_padding(header_plus_name);
        if pad > 0 {
            self.skip_bytes(pad)?;
        }

        // Check for TRAILER!!!
        if &name_buf[..name_len] == TRAILER_NAME {
            self.finished = true;
            return Ok(None);
        }

        Ok(Some(CpioEntry {
            header,
            magic,
            name: &name_buf[..name_len],
            entry_offset,
        }))
    }

    /// Read entry data into `buf`. The buffer must be exactly `entry.file_size()` bytes.
    /// After reading, skips any padding to align to 4-byte boundary.
    pub fn read_entry_data(&mut self, entry: &CpioEntry<'_>, buf: &mut [u8]) -> Result<()> {
        let size = entry.file_size() as usize;
        assert_eq!(buf.len(), size, "buffer size must match entry file size");
        if size > 0 {
            self.reader.read_exact(&mut buf[..size])?;
            self.offset += size as u64;
        }
        // Skip data padding
        let pad = align4_padding(entry.file_size() as u64);
        if pad > 0 {
            self.skip_bytes(pad)?;
        }
        Ok(())
    }

    /// Skip over entry data without reading it.
    pub fn skip_entry_data(&mut self, entry: &CpioEntry<'_>) -> Result<()> {
        let size = entry.file_size() as u64;
        let pad = align4_padding(size);
        self.skip_bytes(size + pad)?;
        Ok(())
    }

    /// Skip over entry data for an owned entry.
    #[cfg(feature = "alloc")]
    pub fn skip_entry_data_owned(&mut self, entry: &CpioEntryOwned) -> Result<()> {
        let size = entry.file_size() as u64;
        let pad = align4_padding(size);
        self.skip_bytes(size + pad)?;
        Ok(())
    }

    fn skip_bytes(&mut self, mut n: u64) -> Result<()> {
        let mut discard = [0u8; 256];
        while n > 0 {
            let chunk = n.min(discard.len() as u64) as usize;
            self.reader.read_exact(&mut discard[..chunk])?;
            self.offset += chunk as u64;
            n -= chunk as u64;
        }
        Ok(())
    }

    /// Read the next entry, allocating a `Vec` for the filename.
    ///
    /// Returns `Ok(None)` when the `TRAILER!!!` sentinel is reached.
    /// After obtaining an entry, call [`read_entry_data_alloc`](Self::read_entry_data_alloc)
    /// or [`skip_entry_data_owned`](Self::skip_entry_data_owned) before calling this again.
    #[cfg(feature = "alloc")]
    pub fn next_entry_alloc(&mut self) -> Result<Option<CpioEntryOwned>> {
        if self.finished {
            return Ok(None);
        }

        let entry_offset = self.offset;

        let raw = RawNewcHeader::parse(&mut self.reader)?;
        self.offset += HEADER_SIZE as u64;

        let magic = raw.magic().ok_or_else(|| {
            let mut found = [0u8; 6];
            found.copy_from_slice(raw.magic_bytes());
            CpioError::InvalidMagic { found }
        })?;

        let header = CpioEntryHeader::from_raw(&raw)?;
        let namesize = raw.namesize()? as usize;

        if namesize == 0 {
            return Err(CpioError::InvalidFilename);
        }

        let name_len = namesize - 1;
        let mut name = vec![0u8; name_len];
        self.reader.read_exact(&mut name)?;
        self.offset += name_len as u64;

        // Read and discard NUL
        let mut nul = [0u8; 1];
        self.reader.read_exact(&mut nul)?;
        self.offset += 1;

        // Skip name padding
        let header_plus_name = HEADER_SIZE as u64 + namesize as u64;
        let pad = align4_padding(header_plus_name);
        if pad > 0 {
            self.skip_bytes(pad)?;
        }

        // Check for TRAILER!!!
        if name.as_slice() == TRAILER_NAME {
            self.finished = true;
            return Ok(None);
        }

        Ok(Some(CpioEntryOwned {
            header,
            magic,
            name,
            entry_offset,
        }))
    }

    /// Read the entry's file data into a newly allocated `Vec`.
    ///
    /// After reading, the reader is positioned at the next entry's header.
    #[cfg(feature = "alloc")]
    pub fn read_entry_data_alloc(&mut self, entry: &CpioEntryOwned) -> Result<Vec<u8>> {
        let size = entry.file_size() as usize;
        let mut buf = vec![0u8; size];
        if size > 0 {
            self.reader.read_exact(&mut buf)?;
            self.offset += size as u64;
        }
        let pad = align4_padding(entry.file_size() as u64);
        if pad > 0 {
            self.skip_bytes(pad)?;
        }
        Ok(buf)
    }
}

/// Seek support: when the reader supports seeking, allow jumping to recorded offsets.
impl<R: Read + hadris_io::Seek> CpioReader<R> {
    /// Seek to a previously-recorded entry offset to re-read that entry.
    pub fn seek_to_entry(&mut self, offset: u64) -> Result<()> {
        self.reader.seek(hadris_io::SeekFrom::Start(offset))?;
        self.offset = offset;
        self.finished = false;
        Ok(())
    }
}
