use alloc::vec;
use alloc::vec::Vec;

use hadris_fs::{ErrorKind, FileType, Node, PathError, Report, SetAttr, Tree};

use super::fs::ContentReader;
use super::io::Write;
use crate::build::{self, Data, Fields, Planner, error, plan_tree};
use crate::error::{Detail, Error, write_failed};
use crate::header;
use crate::options::{CpioOptions, Format};
use crate::raw::{self, TRAILER_NAME};

/// Bytes read from a content per request.
const CHUNK: usize = 64 * 1024;

/// A streaming cpio archive writer.
///
/// Entries go out in the order they are appended, and
/// [`finish`](Self::finish) writes the trailer and returns the stream with
/// the [`Report`]. Inodes are numbered from 1. After an error the archive
/// is incomplete.
///
/// ```rust,ignore
/// let mut writer = Writer::new(out, &CpioOptions::new());
/// writer.append("init", &Node::file(content).with_attrs(SetAttr::new().with_permissions(Permissions::new(0o755))))?;
/// let (out, report) = writer.finish()?;
/// ```
pub struct Writer<W> {
    out: W,
    planner: Planner,
    bytes: u64,
    open_entry: bool,
    buf: Vec<u8>,
}

impl<W> core::fmt::Debug for Writer<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Writer")
            .field("format", &self.planner.format)
            .field("entries", &self.planner.entries())
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl<W> Writer<W> {
    /// A writer of `options.format()` entries to `out`.
    pub fn new(out: W, options: &CpioOptions) -> Self {
        Self {
            out,
            planner: Planner::new(options),
            bytes: 0,
            open_entry: false,
            buf: Vec::new(),
        }
    }

    /// Entries written so far, not counting a trailer.
    pub fn entries(&self) -> u64 {
        self.planner.entries()
    }

    /// Bytes written so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes
    }

    fn check_idle(&self) -> Result<(), PathError> {
        match self.open_entry {
            true => Err(PathError::new(
                ErrorKind::InvalidInput,
                "an entry from append_file was not finished",
            )),
            false => Ok(()),
        }
    }
}

/// Checks that `data` is readable in this mode before anything is written.
fn check_data(data: &Data<'_>, path: &[u8]) -> Result<(), PathError> {
    match data {
        Data::Content(content) => ContentReader::check(content).map_err(|err| err.with_path(path)),
        _ => Ok(()),
    }
}

/// A file entry being written by [`Writer::append_file`]. It implements
/// `Write`; [`finish`](Self::finish) ends the entry once exactly the
/// announced length was written.
pub struct EntryWriter<'w, W> {
    writer: &'w mut Writer<W>,
    path: Vec<u8>,
    len: u64,
    remaining: u64,
}

impl<W> core::fmt::Debug for EntryWriter<'_, W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EntryWriter")
            .field("len", &self.len)
            .field("remaining", &self.remaining)
            .finish_non_exhaustive()
    }
}

impl<W> EntryWriter<'_, W> {
    /// Bytes still to write.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }
}

impl<W: hadris_io::ErrorType> hadris_io::ErrorType for EntryWriter<'_, W> {
    type Error = Error<W::Error>;
}

io_transform! {

/// Writes `tree` as a complete cpio archive to `out`, as `options` say, and
/// returns the report [`plan`](crate::plan) returns.
///
/// It plans first and checks that every file's content is readable in this
/// mode, so an error there writes nothing. Entries follow the tree
/// depth-first, each directory before its children, children in name
/// order; the root itself is not an entry. The names of a hard link share
/// one inode, and the last of them in that order carries the data, as GNU
/// cpio writes them. Report offsets count from the first byte this call
/// writes. `out` is flushed, not closed.
pub async fn write<W: Write>(out: W, tree: &Tree, options: &CpioOptions) -> Result<Report, PathError> {
    let (entries, report) = plan_tree(tree, options)?;
    for entry in &entries {
        check_data(&entry.data, &entry.path)?;
    }
    let mut writer = Writer::new(out, options);
    for entry in &entries {
        writer.emit(&entry.path, entry.fields, entry.data).await?;
    }
    writer.write_trailer().await?;
    Ok(report)
}

impl<W: Write> Writer<W> {
    async fn put(&mut self, data: &[u8]) -> Result<(), Error<W::Error>> {
        self.out.write_all(data).await.map_err(write_failed)?;
        self.bytes += data.len() as u64;
        Ok(())
    }

    /// Writes a header and name the planner checked, or the trailer's.
    async fn header(&mut self, path: &[u8], fields: &Fields) -> Result<(), PathError> {
        let namesize = match path == TRAILER_NAME {
            true => TRAILER_NAME.len() + 1,
            false => build::namesize(path)?,
        };
        let mut raw = [0u8; raw::NEWC_HEADER_LEN];
        let len = build::encode::<W::Error>(self.planner.format, fields, namesize, &mut raw)
            .map_err(|err| PathError::from(err).with_path(path))?;
        self.put(&raw[..len]).await?;
        self.put(path).await?;
        self.put(&[0]).await?;
        self.put(&[0u8; 3][..header::name_padding(self.planner.format, namesize)]).await?;
        Ok(())
    }

    async fn emit(&mut self, path: &[u8], mut fields: Fields, data: Data<'_>) -> Result<(), PathError> {
        if self.buf.len() < CHUNK {
            self.buf = vec![0u8; CHUNK];
        }
        let mut reader = match data {
            Data::Content(content) => Some(ContentReader::open(content).await.map_err(|err| err.with_path(path))?),
            _ => None,
        };
        if self.planner.format == Format::Crc {
            fields.check = match (&data, &mut reader) {
                (Data::Bytes(bytes), _) => header::checksum(0, bytes),
                (_, Some(reader)) => {
                    let mut buf = core::mem::take(&mut self.buf);
                    let sum = checksum(reader, fields.len, &mut buf).await;
                    self.buf = buf;
                    sum.map_err(|err| err.with_path(path))?
                }
                _ => 0,
            };
        }
        self.header(path, &fields).await?;
        match (data, &mut reader) {
            (Data::Bytes(bytes), _) => self.put(bytes).await?,
            (_, Some(reader)) => {
                let mut buf = core::mem::take(&mut self.buf);
                let copied = self.copy(reader, fields.len, &mut buf).await;
                self.buf = buf;
                copied.map_err(|err| err.with_path(path))?;
            }
            _ => {}
        }
        self.put(&[0u8; 3][..header::data_padding(self.planner.format, fields.len)]).await?;
        Ok(())
    }

    async fn copy(&mut self, reader: &mut ContentReader<'_>, len: u64, buf: &mut [u8]) -> Result<(), PathError> {
        let mut offset = 0u64;
        while offset < len {
            let take = (len - offset).min(buf.len() as u64) as usize;
            reader.read_exact_at(offset, &mut buf[..take]).await?;
            self.put(&buf[..take]).await?;
            offset += take as u64;
        }
        Ok(())
    }

    /// Appends `node` named `path`: a file, directory, symlink, device
    /// node, FIFO or socket, with the permissions, owner and modification
    /// time its attributes set.
    ///
    /// Unset fields take 0o644 permissions (0o755 for directories and
    /// sockets, 0o777 for symlinks), owner 0 and the options' time, or 0.
    /// Other times, sub-second parts and attributes are dropped and
    /// reported.
    ///
    /// Fails before writing anything of the entry, as [`plan`](crate::plan)
    /// does, and with [`ErrorKind::InvalidInput`] for an empty symlink
    /// target, and [`ErrorKind::Unsupported`] for content this mode cannot
    /// read.
    pub async fn append(&mut self, path: impl AsRef<[u8]>, node: &Node) -> Result<(), PathError> {
        let path = path.as_ref();
        self.check_idle()?;
        let (mut fields, data) = self.planner.node(node, path)?;
        check_data(&data, path)?;
        fields.ino = self.planner.ino();
        let start = self.planner.entry(path, &fields, node.attrs())?;
        self.planner.take_ino();
        if node.file_type() == FileType::File {
            self.planner.extent(path, start, fields.len);
        }
        self.emit(path, fields, data).await
    }

    /// Appends the file `node` under every name in `paths`, as a hard link
    /// group: one inode, a link count of `paths.len()`, and the data on the
    /// last name only, as GNU cpio writes them. Every name carries the
    /// node's attributes, and the report gives every name the data's
    /// extent.
    ///
    /// Fails like [`append`](Self::append) before writing anything, and
    /// with [`ErrorKind::InvalidInput`] when `paths` is empty or `node` is
    /// not a file.
    pub async fn append_hard_links<P: AsRef<[u8]>>(&mut self, paths: &[P], node: &Node) -> Result<(), PathError> {
        self.check_idle()?;
        let Some((last, first)) = paths.split_last() else {
            return Err(Detail::Entry.invalid::<W::Error>().into());
        };
        if node.file_type() != FileType::File {
            return Err(error(Detail::Entry, ErrorKind::InvalidInput, last.as_ref()));
        }
        let (mut fields, data) = self.planner.node(node, last.as_ref())?;
        check_data(&data, last.as_ref())?;
        fields.ino = self.planner.ino();
        fields.nlink = paths.len() as u64;
        let empty = Fields { len: 0, ..fields };
        for path in first {
            self.planner.check(path.as_ref(), &empty)?;
        }
        self.planner.check(last.as_ref(), &fields)?;
        for path in first {
            self.planner.entry(path.as_ref(), &empty, node.attrs())?;
        }
        let start = self.planner.entry(last.as_ref(), &fields, node.attrs())?;
        self.planner.take_ino();
        for path in paths {
            self.planner.extent(path.as_ref(), start, fields.len);
        }
        for path in first {
            self.emit(path.as_ref(), empty, Data::None).await?;
        }
        self.emit(last.as_ref(), fields, data).await
    }

    /// Starts a regular file of `len` bytes named `path`, for data produced
    /// while writing, and returns the writer of its data.
    ///
    /// The header is written now. Write exactly `len` bytes, then call
    /// [`EntryWriter::finish`]; until then every other call on this writer
    /// fails with [`ErrorKind::InvalidInput`]. Fails like
    /// [`append`](Self::append), and with [`ErrorKind::Unsupported`] for
    /// [`Format::Crc`], whose header holds a checksum of the data.
    pub async fn append_file(&mut self, path: impl AsRef<[u8]>, attrs: &SetAttr, len: u64) -> Result<EntryWriter<'_, W>, PathError> {
        let path = path.as_ref();
        self.check_idle()?;
        if self.planner.format == Format::Crc {
            return Err(error(Detail::Entry, ErrorKind::Unsupported, path));
        }
        let mut fields = self.planner.fields(attrs, raw::S_IFREG, 0o644, path)?;
        fields.len = len;
        fields.ino = self.planner.ino();
        let start = self.planner.entry(path, &fields, attrs)?;
        self.planner.take_ino();
        self.planner.extent(path, start, len);
        self.header(path, &fields).await?;
        self.open_entry = true;
        Ok(EntryWriter { writer: self, path: path.to_vec(), len, remaining: len })
    }

    async fn write_trailer(&mut self) -> Result<(), PathError> {
        self.header(TRAILER_NAME, &Fields::TRAILER).await?;
        self.out
            .flush()
            .await
            .map_err(|err| Error::device(err, "flushing the archive failed").into())
    }

    /// Writes the trailer, flushes, and returns the stream with the report:
    /// the bytes written with the trailer, what cpio dropped, and where each
    /// file's data starts, counted from the first byte this writer wrote.
    pub async fn finish(mut self) -> Result<(W, Report), PathError> {
        self.check_idle()?;
        self.planner.trailer()?;
        self.write_trailer().await?;
        Ok((self.out, self.planner.finish()))
    }
}

impl<W: Write> EntryWriter<'_, W> {
    /// Ends the entry with its padding. Fails with
    /// [`ErrorKind::InvalidInput`] naming the entry when fewer bytes than
    /// its length were written.
    pub async fn finish(self) -> Result<(), PathError> {
        if self.remaining != 0 {
            return Err(error(Detail::Entry, ErrorKind::InvalidInput, &self.path));
        }
        let format = self.writer.planner.format;
        self.writer.put(&[0u8; 3][..header::data_padding(format, self.len)]).await?;
        self.writer.open_entry = false;
        Ok(())
    }
}

impl<W: Write> Write for EntryWriter<'_, W> {
    /// Writes data of the entry. Fails with [`ErrorKind::InvalidInput`]
    /// past its length.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.len() as u64 > self.remaining {
            return Err(Detail::Entry.invalid());
        }
        let written = self.writer.out.write(buf).await.map_err(|err| Error::device(err, "writing the archive failed"))?;
        self.remaining -= written as u64;
        self.writer.bytes += written as u64;
        Ok(written)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.writer.out.flush().await.map_err(|err| Error::device(err, "flushing the archive failed"))
    }
}

/// The byte sum of the first `len` bytes of `reader`.
async fn checksum(reader: &mut ContentReader<'_>, len: u64, buf: &mut [u8]) -> Result<u32, PathError> {
    let mut sum = 0u32;
    let mut offset = 0u64;
    while offset < len {
        let take = (len - offset).min(buf.len() as u64) as usize;
        reader.read_exact_at(offset, &mut buf[..take]).await?;
        sum = header::checksum(sum, &buf[..take]);
        offset += take as u64;
    }
    Ok(sum)
}

}
