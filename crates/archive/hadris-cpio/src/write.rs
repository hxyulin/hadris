use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hadris_fs::tree::{Content, NodeKind, Tree, Warning, WarningKind};
use hadris_fs::{DeviceKind, DeviceNumber, ErrorKind, SetMetadata};

use super::fs::ContentReader;
use super::io::Write;
use crate::entry::{NewEntry, Report, dropped};
use crate::error::{Detail, Error};
use crate::header;
use crate::options::{CpioOptions, Format};
use crate::raw::{self, NewcFields, NewcHeader, OdcFields, OdcHeader, PATH_MAX, TRAILER_NAME};

/// Bytes read from a content per request.
const CHUNK: usize = 64 * 1024;

/// The header fields of one entry, before encoding.
#[derive(Debug, Clone, Copy)]
struct Fields {
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u64,
    mtime: u64,
    len: u64,
    rdev: DeviceNumber,
    check: u32,
}

/// A streaming cpio archive writer.
///
/// Entries go out in the order they are appended; [`finish`](Self::finish)
/// writes the trailer and returns the stream. Inodes are numbered from 1.
///
/// ```rust,ignore
/// let mut writer = CpioWriter::new(out, &CpioOptions::default());
/// writer.append("init", &SetMetadata::new().with_mode(Mode::new(0o755)), NewEntry::File(&content))?;
/// let out = writer.finish()?;
/// ```
pub struct CpioWriter<W> {
    out: W,
    format: Format,
    next_ino: u64,
    entries: u64,
    bytes: u64,
    warnings: Vec<Warning>,
    buf: Vec<u8>,
}

impl<W> core::fmt::Debug for CpioWriter<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CpioWriter")
            .field("format", &self.format)
            .field("entries", &self.entries)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl<W> CpioWriter<W> {
    /// A writer of `options.format()` entries to `out`.
    pub fn new(out: W, options: &CpioOptions) -> Self {
        Self {
            out,
            format: options.format(),
            next_ino: 1,
            entries: 0,
            bytes: 0,
            warnings: Vec::new(),
            buf: Vec::new(),
        }
    }

    /// Entries written so far, not counting a trailer.
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Bytes written so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes
    }

    /// Metadata dropped so far, one warning per entry.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    fn check_name(&self, path: &str) -> Result<usize, ErrorKind> {
        let name = path.as_bytes();
        if name.is_empty() || name.contains(&0) || name == TRAILER_NAME {
            return Err(ErrorKind::InvalidInput);
        }
        if name.len() + 1 > PATH_MAX {
            return Err(ErrorKind::NameTooLong);
        }
        Ok(name.len() + 1)
    }

    fn take_ino(&mut self) -> u64 {
        let ino = self.next_ino;
        self.next_ino += 1;
        ino
    }

    fn warn(&mut self, path: &str, meta: &SetMetadata) {
        if let Some(message) = dropped(meta) {
            self.warnings
                .push(Warning::new(path, WarningKind::IgnoredMetadata, message));
        }
    }
}

/// Mode bits, owner and modification time of `meta`, with the defaults for
/// `kind` where it sets none.
fn base_fields<E>(meta: &SetMetadata, kind: u32, default_mode: u32) -> Result<Fields, Error<E>> {
    let mtime = match meta.times().modified() {
        Some(time) => u64::try_from(time.unix_seconds())
            .map_err(|_| Error::new(ErrorKind::LimitExceeded, Detail::Field))?,
        None => 0,
    };
    Ok(Fields {
        ino: 0,
        mode: kind | meta.mode().map_or(default_mode, |mode| mode.bits()),
        uid: meta.uid().unwrap_or(0),
        gid: meta.gid().unwrap_or(0),
        nlink: 1,
        mtime,
        len: 0,
        rdev: DeviceNumber::new(0, 0),
        check: 0,
    })
}

/// Encodes a header of `format` into `out`, returning its length.
fn encode<E>(
    format: Format,
    fields: &Fields,
    namesize: usize,
    out: &mut [u8; raw::NEWC_HEADER_LEN],
) -> Result<usize, Error<E>> {
    if fields.len > format.max_file_size() {
        return Err(Error::new(ErrorKind::FileTooLarge, Detail::Field));
    }
    let limit = || Error::new(ErrorKind::LimitExceeded, Detail::Field);
    let small = |value: u64| u32::try_from(value).map_err(|_| limit());
    match format {
        Format::Newc | Format::NewcCrc => {
            let header = NewcHeader::new(
                format == Format::NewcCrc,
                &NewcFields {
                    ino: small(fields.ino)?,
                    mode: fields.mode,
                    uid: fields.uid,
                    gid: fields.gid,
                    nlink: small(fields.nlink)?,
                    mtime: small(fields.mtime)?,
                    filesize: small(fields.len)?,
                    devmajor: 0,
                    devminor: 0,
                    rdevmajor: fields.rdev.major(),
                    rdevminor: fields.rdev.minor(),
                    namesize: small(namesize as u64)?,
                    check: fields.check,
                },
            );
            out.copy_from_slice(&header.0);
            Ok(raw::NEWC_HEADER_LEN)
        }
        Format::Odc => {
            let header = OdcHeader::new(&OdcFields {
                dev: 0,
                ino: small(fields.ino)?,
                mode: fields.mode,
                uid: fields.uid,
                gid: fields.gid,
                nlink: small(fields.nlink)?,
                rdev: header::join_dev(fields.rdev).ok_or_else(limit)?,
                mtime: fields.mtime,
                namesize: small(namesize as u64)?,
                filesize: fields.len,
            })
            .ok_or_else(limit)?;
            out[..raw::ODC_HEADER_LEN].copy_from_slice(&header.0);
            Ok(raw::ODC_HEADER_LEN)
        }
        _ => Err(Error::new(ErrorKind::Unsupported, Detail::Format)),
    }
}

/// The data of an entry being written.
enum Data<'a, 'c> {
    Bytes(&'a [u8]),
    Content(&'a mut ContentReader<'c>),
}

io_transform! {

/// Writes `tree` as a complete cpio archive to `out`, as `options` say, and
/// returns the [`Report`].
///
/// Entries follow the tree depth-first, each directory before its
/// children, children in the tree's name order; the root itself is not an
/// entry. The names of a hard link share one inode, and the last of them
/// in that order carries the data, as GNU cpio writes them.
pub async fn write<W: Write>(out: W, tree: &Tree, options: &CpioOptions) -> Result<Report, Error<W::Error>> {
    let mut writer = CpioWriter::new(out, options);
    writer.write_tree(tree).await?;
    writer.write_trailer().await?;
    Ok(Report::new(writer.entries, writer.bytes, writer.warnings))
}

impl<W: Write> CpioWriter<W> {
    async fn put(&mut self, data: &[u8]) -> Result<(), Error<W::Error>> {
        self.out.write_all(data).await.map_err(Error::write)?;
        self.bytes += data.len() as u64;
        Ok(())
    }

    async fn emit(&mut self, path: &str, mut fields: Fields, mut data: Option<Data<'_, '_>>) -> Result<(), Error<W::Error>> {
        let namesize = self.check_name(path).map_err(|kind| Error::new(kind, Detail::Name))?;
        if self.buf.len() < CHUNK {
            self.buf = vec![0u8; CHUNK];
        }
        if self.format == Format::NewcCrc {
            fields.check = match &mut data {
                Some(Data::Bytes(bytes)) => header::checksum(0, bytes),
                Some(Data::Content(reader)) => {
                    let len = reader.len();
                    let mut buf = core::mem::take(&mut self.buf);
                    let sum = checksum(reader, len, &mut buf).await;
                    self.buf = buf;
                    sum.map_err(Error::content)?
                }
                None => 0,
            };
        }
        let mut raw = [0u8; raw::NEWC_HEADER_LEN];
        let len = encode(self.format, &fields, namesize, &mut raw)?;
        self.put(&raw[..len]).await?;
        self.put(path.as_bytes()).await?;
        self.put(&[0]).await?;
        self.put(&[0u8; 3][..header::name_padding(self.format, namesize)]).await?;
        match data {
            Some(Data::Bytes(bytes)) => self.put(bytes).await?,
            Some(Data::Content(reader)) => {
                let mut buf = core::mem::take(&mut self.buf);
                let copied = self.copy(reader, fields.len, &mut buf).await;
                self.buf = buf;
                copied?;
            }
            None => {}
        }
        self.put(&[0u8; 3][..header::data_padding(self.format, fields.len)]).await?;
        self.entries += 1;
        Ok(())
    }

    async fn copy(&mut self, reader: &mut ContentReader<'_>, len: u64, buf: &mut [u8]) -> Result<(), Error<W::Error>> {
        let mut offset = 0u64;
        while offset < len {
            let take = (len - offset).min(buf.len() as u64) as usize;
            reader.read_exact_at(offset, &mut buf[..take]).await.map_err(Error::content)?;
            self.put(&buf[..take]).await?;
            offset += take as u64;
        }
        Ok(())
    }

    /// Appends one entry named `path`, with the mode, owner and
    /// modification time of `meta`.
    ///
    /// Unset fields take 0o644 permissions (0o755 for directories, 0o777
    /// for symlinks), owner 0 and modification time 0. Times other than
    /// the modification time and attributes are dropped with a warning.
    ///
    /// Fails before writing anything of the entry: [`ErrorKind::InvalidInput`]
    /// for an empty name, one with a NUL or the trailer's name, or an empty
    /// symlink target; [`ErrorKind::NameTooLong`] for names over 4095
    /// bytes; [`ErrorKind::FileTooLarge`] for data the format cannot size;
    /// [`ErrorKind::LimitExceeded`] for another value that does not fit its
    /// field; [`ErrorKind::Unsupported`] for [`Format::Binary`].
    pub async fn append(&mut self, path: &str, meta: &SetMetadata, entry: NewEntry<'_>) -> Result<(), Error<W::Error>> {
        match entry {
            NewEntry::File(content) => self.append_hard_links(&[path], meta, content).await,
            NewEntry::Dir => {
                let mut fields = base_fields(meta, raw::S_IFDIR, 0o755)?;
                fields.nlink = 2;
                self.append_fields(path, meta, fields, None).await
            }
            NewEntry::Symlink(target) => {
                if target.is_empty() {
                    return Err(Error::invalid(Detail::Entry));
                }
                let mut fields = base_fields(meta, raw::S_IFLNK, 0o777)?;
                fields.len = target.len() as u64;
                self.append_fields(path, meta, fields, Some(Data::Bytes(target))).await
            }
            NewEntry::Device(kind, number) => {
                let bits = match kind {
                    DeviceKind::Block => raw::S_IFBLK,
                    DeviceKind::Char => raw::S_IFCHR,
                    _ => return Err(Error::new(ErrorKind::Unsupported, Detail::Entry)),
                };
                let mut fields = base_fields(meta, bits, 0o644)?;
                fields.rdev = number;
                self.append_fields(path, meta, fields, None).await
            }
            NewEntry::Fifo => {
                let fields = base_fields(meta, raw::S_IFIFO, 0o644)?;
                self.append_fields(path, meta, fields, None).await
            }
            NewEntry::Socket => {
                let fields = base_fields(meta, raw::S_IFSOCK, 0o755)?;
                self.append_fields(path, meta, fields, None).await
            }
        }
    }

    async fn append_fields(&mut self, path: &str, meta: &SetMetadata, mut fields: Fields, data: Option<Data<'_, '_>>) -> Result<(), Error<W::Error>> {
        fields.ino = self.next_ino;
        self.emit(path, fields, data).await?;
        self.take_ino();
        self.warn(path, meta);
        Ok(())
    }

    /// Appends a regular file under every name in `paths`, as a hard link
    /// group: one inode, a link count of `paths.len()`, and the data on the
    /// last name only, as GNU cpio writes them. Every name carries `meta`.
    ///
    /// Fails like [`append`](Self::append), and with
    /// [`ErrorKind::InvalidInput`] when `paths` is empty. A content that
    /// cannot be read fails with [`Detail::Content`].
    pub async fn append_hard_links(&mut self, paths: &[&str], meta: &SetMetadata, content: &Content) -> Result<(), Error<W::Error>> {
        let Some((last, first)) = paths.split_last() else {
            return Err(Error::invalid(Detail::Entry));
        };
        for path in paths {
            self.check_name(path).map_err(|kind| Error::new(kind, Detail::Name))?;
        }
        let mut fields = base_fields(meta, raw::S_IFREG, 0o644)?;
        fields.ino = self.next_ino;
        fields.nlink = paths.len() as u64;
        for path in first {
            self.emit(path, fields, None).await?;
            self.warn(path, meta);
        }
        self.file(last, fields, meta, content).await?;
        self.take_ino();
        Ok(())
    }

    async fn file(&mut self, path: &str, mut fields: Fields, meta: &SetMetadata, content: &Content) -> Result<(), Error<W::Error>> {
        let mut reader = ContentReader::open(content).await.map_err(Error::content)?;
        fields.len = reader.len();
        self.emit(path, fields, Some(Data::Content(&mut reader))).await?;
        self.warn(path, meta);
        Ok(())
    }

    /// Appends every entry of `tree`, as [`write()`] orders them, and
    /// returns what this call wrote. The trailer is not written.
    pub async fn write_tree(&mut self, tree: &Tree) -> Result<Report, Error<W::Error>> {
        let (entries, bytes, warnings) = (self.entries, self.bytes, self.warnings.len());
        let mut links: BTreeMap<usize, (u64, usize)> = BTreeMap::new();
        for (path, node) in preorder(tree) {
            let meta = node.metadata();
            match node.kind() {
                NodeKind::File(content) if node.links() > 1 => {
                    let ino = match links.get(&node.id()) {
                        Some(&(ino, _)) => ino,
                        None => self.take_ino(),
                    };
                    let seen = links.get(&node.id()).map_or(0, |&(_, seen)| seen) + 1;
                    links.insert(node.id(), (ino, seen));
                    let mut fields = base_fields(meta, raw::S_IFREG, 0o644)?;
                    fields.ino = ino;
                    fields.nlink = node.links() as u64;
                    if seen == node.links() {
                        self.file(&path, fields, meta, content).await?;
                    } else {
                        self.emit(&path, fields, None).await?;
                        self.warn(&path, meta);
                    }
                }
                NodeKind::File(content) => self.append(&path, meta, NewEntry::File(content)).await?,
                NodeKind::Dir => self.append(&path, meta, NewEntry::Dir).await?,
                NodeKind::Symlink(target) => self.append(&path, meta, NewEntry::Symlink(target)).await?,
                NodeKind::Device(kind, number) => self.append(&path, meta, NewEntry::Device(kind, number)).await?,
                _ => return Err(Error::new(ErrorKind::Unsupported, Detail::Entry)),
            }
        }
        Ok(Report::new(self.entries - entries, self.bytes - bytes, self.warnings[warnings..].to_vec()))
    }

    async fn write_trailer(&mut self) -> Result<(), Error<W::Error>> {
        let fields = Fields { ino: 0, mode: 0, uid: 0, gid: 0, nlink: 1, mtime: 0, len: 0, rdev: DeviceNumber::new(0, 0), check: 0 };
        let namesize = TRAILER_NAME.len() + 1;
        let mut raw = [0u8; raw::NEWC_HEADER_LEN];
        let len = encode(self.format, &fields, namesize, &mut raw)?;
        self.put(&raw[..len]).await?;
        self.put(TRAILER_NAME).await?;
        self.put(&[0]).await?;
        self.put(&[0u8; 3][..header::name_padding(self.format, namesize)]).await?;
        self.out.flush().await.map_err(Error::device)
    }

    /// Writes the trailer, flushes and returns the stream.
    pub async fn finish(mut self) -> Result<W, Error<W::Error>> {
        self.write_trailer().await?;
        Ok(self.out)
    }
}

/// The byte sum of the first `len` bytes of `reader`.
async fn checksum(reader: &mut ContentReader<'_>, len: u64, buf: &mut [u8]) -> Result<u32, hadris_fs::AnyError> {
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

/// Every node of `tree` but the root, depth-first, each directory before
/// its children, with its `/`-separated path.
fn preorder(tree: &Tree) -> Vec<(String, hadris_fs::tree::TreeNode<'_>)> {
    let mut out = Vec::new();
    let mut pending = vec![(String::new(), tree.root())];
    while let Some((path, node)) = pending.pop() {
        if !path.is_empty() {
            out.push((path.clone(), node));
        }
        let children: Vec<_> = node.children().collect();
        for (name, child) in children.into_iter().rev() {
            let child_path = if path.is_empty() {
                String::from(name)
            } else {
                alloc::format!("{path}/{name}")
            };
            pending.push((child_path, child));
        }
    }
    out
}
