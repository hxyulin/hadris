use hadris_fs::{DateTime, DeviceNumber, ErrorKind, FileType, Metadata, Owner, Permissions};

use super::io::Read;
use crate::error::{Detail, Error, read_failed};
use crate::header::{self, Header};
use crate::options::{Format, ReaderOptions};
use crate::raw::{PATH_MAX, TRAILER_NAME};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Expecting a header or the end of the input.
    Entries,
    /// A trailer was read.
    Trailer,
    /// The input ended, or reading failed.
    Done,
}

/// A streaming cpio archive reader.
///
/// It reads `newc`, `newc` with checksums, `odc` and old binary archives,
/// one entry at a time, from any `hadris_io` `Read` stream, without an
/// allocator. [`next_entry`](Self::next_entry) returns an [`Entry`] that
/// borrows the reader and implements `Read` over the entry's data; data
/// left unread is skipped by the next call. After an error it returns no
/// more entries.
///
/// ```rust,ignore
/// let mut reader = CpioReader::new(input);
/// while let Some(mut entry) = reader.next_entry()? {
///     println!("{}", entry.path_str().unwrap_or("?"));
/// }
/// ```
pub struct CpioReader<R, B = [u8; PATH_MAX]> {
    reader: R,
    options: ReaderOptions,
    state: State,
    offset: u64,
    header: Header,
    name: B,
    pending: Option<u8>,
    header_offset: u64,
    data_offset: u64,
    name_len: usize,
    remaining: u64,
    padding: usize,
    sum: Option<u32>,
}

impl<R, B> core::fmt::Debug for CpioReader<R, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CpioReader")
            .field("offset", &self.offset)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl<R> CpioReader<R> {
    /// A reader with the default [`ReaderOptions`].
    pub fn new(reader: R) -> Self {
        Self::with_options(reader, ReaderOptions::new())
    }

    /// A reader with `options`.
    pub fn with_options(reader: R, options: ReaderOptions) -> Self {
        Self::with_buffer(reader, [0; PATH_MAX], options)
    }
}

impl<R, B> CpioReader<R, B> {
    /// Uses caller-provided name storage, including space for the terminating NUL.
    ///
    /// `B` must expose the same storage through `AsRef<[u8]>` and `AsMut<[u8]>`.
    /// Reading requires both traits, plus `Send` in async mode. Reading the
    /// `TRAILER!!!` marker requires at least 11 bytes of storage.
    pub fn with_buffer(reader: R, buf: B, options: ReaderOptions) -> Self {
        Self {
            reader,
            options,
            state: State::Entries,
            offset: 0,
            header: Header::EMPTY,
            name: buf,
            pending: None,
            header_offset: 0,
            data_offset: 0,
            name_len: 0,
            remaining: 0,
            padding: 0,
            sum: None,
        }
    }

    /// Bytes consumed from the stream so far, including a peeked segment byte.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the underlying stream at its physical position.
    ///
    /// This discards any byte peeked by [`next_segment`](Self::next_segment).
    /// Use [`into_parts`](Self::into_parts) to preserve that byte and recover the buffer.
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Returns the stream, name buffer and any byte peeked by `next_segment`.
    /// Replay the optional byte before reading the returned stream.
    pub fn into_parts(self) -> (R, B, Option<u8>) {
        (self.reader, self.name, self.pending)
    }
}

/// An entry of a cpio archive, borrowing its [`CpioReader`].
///
/// It implements `Read` over the entry's data: a file's contents or a
/// symlink's target. Dropping it leaves the rest of the data for the next
/// [`CpioReader::next_entry`] to skip.
pub struct Entry<'a, R, B = [u8; PATH_MAX]> {
    reader: &'a mut CpioReader<R, B>,
}

impl<R, B: AsRef<[u8]>> core::fmt::Debug for Entry<'_, R, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Entry")
            .field("path", &self.path())
            .field("format", &self.format())
            .field("mode", &self.mode())
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<R, B: AsRef<[u8]>> Entry<'_, R, B> {
    fn header(&self) -> &Header {
        &self.reader.header
    }

    /// The name, as stored: usually a relative path without a leading `/`.
    pub fn path(&self) -> &[u8] {
        &self.reader.name.as_ref()[..self.reader.name_len]
    }

    /// The name as UTF-8.
    pub fn path_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.path())
    }

    /// Header start, in bytes from the start of the supplied stream.
    pub fn offset(&self) -> u64 {
        self.reader.header_offset
    }

    /// Data start after name padding, in bytes from the start of the supplied stream.
    pub fn data_offset(&self) -> u64 {
        self.reader.data_offset
    }

    /// The header format of this entry.
    pub fn format(&self) -> Format {
        self.header().format
    }

    /// What the entry is.
    pub fn file_type(&self) -> FileType {
        header::file_type(self.header().mode).unwrap_or(FileType::File)
    }

    /// The whole mode: file type and permission bits.
    pub fn mode(&self) -> u32 {
        self.header().mode
    }

    /// The permission bits.
    pub fn permissions(&self) -> Permissions {
        Permissions::new(self.header().mode)
    }

    /// The inode number, shared by the names of a hard link group.
    pub fn ino(&self) -> u64 {
        self.header().ino
    }

    /// The owner user id.
    pub fn uid(&self) -> u32 {
        self.header().uid
    }

    /// The owner group id.
    pub fn gid(&self) -> u32 {
        self.header().gid
    }

    /// The number of links.
    pub fn nlink(&self) -> u32 {
        self.header().nlink
    }

    /// The modification time in seconds since the Unix epoch.
    pub fn mtime(&self) -> u64 {
        self.header().mtime
    }

    /// The modification time.
    pub fn modified(&self) -> Option<DateTime> {
        DateTime::from_unix_seconds(i64::try_from(self.header().mtime).ok()?).ok()
    }

    /// The length of the data. Zero for all but one name of a hard link
    /// group written by GNU cpio or Hadris.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.header().len
    }

    /// The device holding the file on the system that wrote the archive.
    pub fn dev(&self) -> DeviceNumber {
        self.header().dev
    }

    /// The device number of a device node.
    pub fn rdev(&self) -> DeviceNumber {
        self.header().rdev
    }

    /// The checksum field: the byte sum of the data for [`Format::Crc`],
    /// zero otherwise.
    pub fn check(&self) -> u32 {
        self.header().check
    }

    /// Type, length, modification time, permissions, owner and links.
    pub fn metadata(&self) -> Metadata {
        let meta = Metadata::new(self.file_type(), self.permissions())
            .with_len(self.len())
            .with_owner(Owner::new(self.uid(), self.gid()))
            .with_nlink(u64::from(self.nlink()));
        match self.modified() {
            Some(time) => meta.with_modified(time),
            None => meta,
        }
    }

    /// Bytes of data not read yet.
    pub fn remaining(&self) -> u64 {
        self.reader.remaining
    }
}

impl<R: hadris_io::ErrorType, B> hadris_io::ErrorType for Entry<'_, R, B> {
    type Error = Error<R::Error>;
}

io_transform! {

async fn fill<R: Read>(reader: &mut R, offset: &mut u64, buf: &mut [u8]) -> Result<(), Error<R::Error>> {
    reader.read_exact(buf).await.map_err(read_failed)?;
    *offset += buf.len() as u64;
    Ok(())
}

impl<R: Read, B: AsRef<[u8]> + AsMut<[u8]> + super::io::MaybeSend> CpioReader<R, B> {
    /// The next entry, or `None` after a trailer or at the end of the
    /// input.
    ///
    /// Fails with [`ErrorKind::Corrupt`](hadris_fs::ErrorKind::Corrupt) for a malformed header, name,
    /// padding or checksum, or an archive cut off inside an entry, and with
    /// [`Detail::Trailer`] when a trailer is required and missing. Names whose
    /// encoded size (including NUL) exceeds the buffer return `LimitExceeded`.
    pub async fn next_entry(&mut self) -> Result<Option<Entry<'_, R, B>>, Error<R::Error>> {
        match self.advance().await {
            Ok(true) => Ok(Some(Entry { reader: self })),
            Ok(false) => Ok(None),
            Err(err) => {
                self.state = State::Done;
                Err(err)
            }
        }
    }

    async fn fill(&mut self, buf: &mut [u8]) -> Result<(), Error<R::Error>> {
        fill(&mut self.reader, &mut self.offset, buf).await
    }

    /// Starts the next concatenated segment after `next_entry` returned `None`.
    ///
    /// Skips zero padding and peeks one byte. `true` means bytes follow, not that
    /// the next header is valid; `next_entry` validates it. Returns `false` at EOF.
    /// Calling this before reaching a trailer or EOF returns `InvalidInput`.
    /// After a successful call, read that segment before calling this again.
    pub async fn next_segment(&mut self) -> Result<bool, Error<R::Error>> {
        if self.state == State::Done {
            return Ok(false);
        }
        if self.state != State::Trailer {
            return Err(Error::new(ErrorKind::InvalidInput, "current cpio segment has not ended"));
        }
        loop {
            match self.first_byte().await {
                Ok(Some(0)) => {}
                Ok(Some(byte)) => {
                    self.pending = Some(byte);
                    self.state = State::Entries;
                    return Ok(true);
                }
                Ok(None) => {
                    self.state = State::Done;
                    return Ok(false);
                }
                Err(err) => {
                    self.state = State::Done;
                    return Err(err);
                }
            }
        }
    }

    async fn first_byte(&mut self) -> Result<Option<u8>, Error<R::Error>> {
        if let Some(byte) = self.pending.take() {
            return Ok(Some(byte));
        }
        let mut byte = [0u8; 1];
        if self.reader.read(&mut byte).await.map_err(|err| Error::device(err, "reading the archive failed"))? == 0 {
            return Ok(None);
        }
        self.offset += 1;
        Ok(Some(byte[0]))
    }

    async fn skip_padding(&mut self, len: usize) -> Result<(), Error<R::Error>> {
        let mut pad = [0u8; 3];
        let pad = &mut pad[..len];
        self.fill(pad).await?;
        if pad.iter().any(|byte| *byte != 0) {
            return Err(Detail::Padding.corrupt());
        }
        Ok(())
    }

    /// Reads the rest of the current entry's data and its padding.
    async fn finish_entry(&mut self) -> Result<(), Error<R::Error>> {
        let mut buf = [0u8; 512];
        while self.remaining > 0 {
            let take = self.remaining.min(buf.len() as u64) as usize;
            self.fill(&mut buf[..take]).await?;
            self.account(&buf[..take])?;
        }
        self.verify()?;
        let padding = core::mem::take(&mut self.padding);
        self.skip_padding(padding).await
    }

    fn account(&mut self, data: &[u8]) -> Result<(), Error<R::Error>> {
        self.remaining -= data.len() as u64;
        if let Some(sum) = &mut self.sum {
            *sum = header::checksum(*sum, data);
        }
        if self.remaining == 0 {
            self.verify()?;
        }
        Ok(())
    }

    fn verify(&mut self) -> Result<(), Error<R::Error>> {
        match self.sum.take() {
            Some(sum) if sum != self.header.check => Err(Detail::Checksum.corrupt()),
            _ => Ok(()),
        }
    }

    async fn advance(&mut self) -> Result<bool, Error<R::Error>> {
        if matches!(self.state, State::Entries) {
            self.finish_entry().await?;
        }
        if !matches!(self.state, State::Entries) {
            return Ok(false);
        }
        let at_start = self.offset == 0;
        let Some(first) = self.first_byte().await? else {
            if self.state == State::Entries && self.options.strict_trailer() {
                return Err(Detail::Trailer.corrupt());
            }
            self.state = State::Done;
            return Ok(false);
        };
        self.header_offset = self.offset - 1;
        let mut raw = [0u8; crate::raw::NEWC_HEADER_LEN];
        raw[0] = first;
        self.fill(&mut raw[1..6]).await?;
        let start: [u8; 6] = [raw[0], raw[1], raw[2], raw[3], raw[4], raw[5]];
        let not_cpio = match at_start {
            true => ErrorKind::NotRecognized,
            false => ErrorKind::Corrupt,
        };
        let format = header::detect(&start).ok_or(Detail::Magic.error(not_cpio))?;
        let len = header::header_len(format);
        self.fill(&mut raw[6..len]).await?;
        let header = header::decode(format, &raw[..len]).ok_or(Detail::Field.corrupt())?;
        if format == Format::Newc && header.check != 0 {
            return Err(Detail::Check.corrupt());
        }
        if header::file_type(header.mode).is_none() && header.mode != 0 {
            return Err(Detail::Field.corrupt());
        }
        if header.namesize < 2 {
            return Err(Detail::Name.corrupt());
        }
        if header.namesize > self.name.as_mut().len() || header.namesize > self.name.as_ref().len() {
            return Err(Detail::Name.error(ErrorKind::LimitExceeded));
        }
        fill(&mut self.reader, &mut self.offset, &mut self.name.as_mut()[..header.namesize]).await?;
        let name = &self.name.as_ref()[..header.namesize];
        let name_len = name.iter().position(|byte| *byte == 0).unwrap_or(header.namesize);
        if name_len == 0 || name_len == header.namesize || name[name_len..].iter().any(|byte| *byte != 0) {
            return Err(Detail::Name.corrupt());
        }
        self.skip_padding(header::name_padding(format, header.namesize)).await?;
        if &self.name.as_ref()[..name_len] == TRAILER_NAME {
            if header.len != 0 {
                return Err(Detail::Trailer.corrupt());
            }
            if format == Format::Crc && header.check != 0 {
                return Err(Detail::Checksum.corrupt());
            }
            self.state = State::Trailer;
            return Ok(false);
        }
        if header.mode == 0 {
            return Err(Detail::Field.corrupt());
        }
        self.data_offset = self.offset;
        self.name_len = name_len;
        self.header = header;
        self.remaining = header.len;
        self.padding = header::data_padding(format, header.len);
        self.sum = (format == Format::Crc).then_some(0);
        if header.len == 0 {
            self.verify()?;
        }
        Ok(true)
    }

    async fn read_data(&mut self, buf: &mut [u8]) -> Result<usize, Error<R::Error>> {
        if self.remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let take = usize::try_from(self.remaining).unwrap_or(usize::MAX).min(buf.len());
        let read = self.reader.read(&mut buf[..take]).await.map_err(|err| Error::device(err, "reading the archive failed"))?;
        if read == 0 {
            return Err(Detail::Truncated.corrupt());
        }
        self.offset += read as u64;
        self.account(&buf[..read])?;
        Ok(read)
    }
}

impl<R: Read, B: AsRef<[u8]> + AsMut<[u8]> + super::io::MaybeSend> Read for Entry<'_, R, B> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let result = self.reader.read_data(buf).await;
        if result.is_err() {
            self.reader.state = State::Done;
        }
        result
    }
}

}
