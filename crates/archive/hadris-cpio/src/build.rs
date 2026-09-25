//! Planning shared by `plan` and the writers of both modes.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::convert::Infallible;

use hadris_fs::{
    Content, DateTime, DeviceNumber, Extent, Field, FileType, Node, PathError, Report, SetAttr,
    Tree, Warning, WarningKind,
};

use crate::error::{Detail, Error};
use crate::header;
use crate::options::{CpioOptions, Format};
use crate::raw::{self, NewcFields, NewcHeader, OdcFields, OdcHeader, PATH_MAX, TRAILER_NAME};
use hadris_fs::ErrorKind;

/// The header fields of one entry, before encoding.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fields {
    pub ino: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u64,
    pub mtime: u64,
    pub len: u64,
    pub rdev: DeviceNumber,
    pub check: u32,
}

impl Fields {
    pub const TRAILER: Self = Self {
        ino: 0,
        mode: 0,
        uid: 0,
        gid: 0,
        nlink: 1,
        mtime: 0,
        len: 0,
        rdev: DeviceNumber::new(0, 0),
        check: 0,
    };
}

/// Encodes a header of `format` into `out`, returning its length.
pub(crate) fn encode<E>(
    format: Format,
    fields: &Fields,
    namesize: usize,
    out: &mut [u8; raw::NEWC_HEADER_LEN],
) -> Result<usize, Error<E>> {
    if fields.len > format.max_file_size() {
        return Err(Detail::Field.error(ErrorKind::FileTooLarge));
    }
    let limit = || Detail::Field.error(ErrorKind::LimitExceeded);
    let small = |value: u64| u32::try_from(value).map_err(|_| limit());
    match format {
        Format::Newc | Format::Crc => {
            let header = NewcHeader::new(
                format == Format::Crc,
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
        _ => Err(Detail::Format.error(ErrorKind::Unsupported)),
    }
}

pub(crate) fn error(detail: Detail, kind: ErrorKind, path: &[u8]) -> PathError {
    PathError::from(detail.error::<Infallible>(kind)).with_path(path)
}

/// The NUL-terminated size of `path` as an entry name.
pub(crate) fn namesize(path: &[u8]) -> Result<usize, PathError> {
    if path.is_empty() || path.contains(&0) || path == TRAILER_NAME {
        return Err(error(Detail::Name, ErrorKind::InvalidInput, path));
    }
    if path.len() + 1 > PATH_MAX {
        return Err(error(Detail::Name, ErrorKind::NameTooLong, path));
    }
    Ok(path.len() + 1)
}

/// The fields a cpio header loses, in the order their warnings are listed.
const LOSSES: [(Field, &str); 4] = [
    (Field::Created, "cpio stores no creation time"),
    (Field::Accessed, "cpio stores no access time"),
    (Field::Modified, "cpio stores whole seconds"),
    (Field::Attributes, "cpio stores no attributes"),
];

/// What goes after an entry's name.
#[derive(Clone, Copy)]
pub(crate) enum Data<'a> {
    None,
    Bytes(&'a [u8]),
    Content(&'a Content),
}

/// One entry of a planned archive.
pub(crate) struct Planned<'a> {
    pub path: Vec<u8>,
    pub fields: Fields,
    pub data: Data<'a>,
}

/// Lays out entries one at a time: inode numbers, offsets, warnings and
/// extents. The streaming writer and `plan` share it, so they agree.
#[derive(Debug)]
pub(crate) struct Planner {
    pub format: Format,
    time: Option<DateTime>,
    next_ino: u64,
    offset: u64,
    entries: u64,
    lost: [u64; LOSSES.len()],
    report: Report,
}

impl Planner {
    pub fn new(options: &CpioOptions) -> Self {
        Self {
            format: options.format(),
            time: options.time(),
            next_ino: 1,
            offset: 0,
            entries: 0,
            lost: [0; LOSSES.len()],
            report: Report::new(),
        }
    }

    pub fn entries(&self) -> u64 {
        self.entries
    }

    pub fn ino(&self) -> u64 {
        self.next_ino
    }

    pub fn take_ino(&mut self) -> u64 {
        let ino = self.next_ino;
        self.next_ino += 1;
        ino
    }

    /// Mode bits, owner and modification time of `attrs`, with the
    /// defaults for `kind` where it sets none.
    pub fn fields(
        &self,
        attrs: &SetAttr,
        kind: u32,
        default_mode: u32,
        path: &[u8],
    ) -> Result<Fields, PathError> {
        let mtime = match attrs.modified().or(self.time) {
            Some(time) => u64::try_from(time.unix_seconds())
                .map_err(|_| error(Detail::Field, ErrorKind::LimitExceeded, path))?,
            None => 0,
        };
        let owner = attrs.owner();
        Ok(Fields {
            ino: 0,
            mode: kind | attrs.permissions().map_or(default_mode, |mode| mode.bits()),
            uid: owner.map_or(0, |owner| owner.uid()),
            gid: owner.map_or(0, |owner| owner.gid()),
            nlink: 1,
            mtime,
            len: 0,
            rdev: DeviceNumber::new(0, 0),
            check: 0,
        })
    }

    /// The length of the header and padded name of an entry, failing as
    /// writing it would.
    pub fn check(&self, path: &[u8], fields: &Fields) -> Result<usize, PathError> {
        let namesize = namesize(path)?;
        let mut raw = [0u8; raw::NEWC_HEADER_LEN];
        let header = encode::<Infallible>(self.format, fields, namesize, &mut raw)
            .map_err(|err| PathError::from(err).with_path(path))?;
        Ok(header + namesize + header::name_padding(self.format, namesize))
    }

    /// Checks that an entry encodes and advances past it, counting what of
    /// `attrs` cpio drops. Returns where its data starts.
    pub fn entry(
        &mut self,
        path: &[u8],
        fields: &Fields,
        attrs: &SetAttr,
    ) -> Result<u64, PathError> {
        let data = self.offset + self.check(path, fields)? as u64;
        self.offset = data + fields.len + header::data_padding(self.format, fields.len) as u64;
        self.entries += 1;
        for (index, (field, _)) in LOSSES.iter().enumerate() {
            let lost = match field {
                Field::Created => attrs.created().is_some(),
                Field::Accessed => attrs.accessed().is_some(),
                Field::Modified => attrs.modified().is_some_and(|time| time.nanoseconds() != 0),
                _ => attrs.attributes().is_some_and(|value| !value.is_empty()),
            };
            self.lost[index] += u64::from(lost);
        }
        Ok(data)
    }

    pub fn trailer(&mut self) -> Result<(), PathError> {
        let namesize = TRAILER_NAME.len() + 1;
        let mut raw = [0u8; raw::NEWC_HEADER_LEN];
        let header = encode::<Infallible>(self.format, &Fields::TRAILER, namesize, &mut raw)?;
        self.offset += (header + namesize + header::name_padding(self.format, namesize)) as u64;
        Ok(())
    }

    pub fn extent(&mut self, path: &[u8], offset: u64, len: u64) {
        self.report.push_extent(path, Extent::new(offset, len));
    }

    pub fn finish(mut self) -> Report {
        for ((field, message), count) in LOSSES.iter().zip(self.lost) {
            if count > 0 {
                self.report.push_warning(
                    Warning::new(WarningKind::Dropped(*field), message).with_count(count),
                );
            }
        }
        self.report.set_size(self.offset);
        self.report
    }

    /// The fields of `node`, with the data that follows its header.
    pub fn node<'a>(&self, node: &'a Node, path: &[u8]) -> Result<(Fields, Data<'a>), PathError> {
        let attrs = node.attrs();
        let (bits, default_mode) = match node.file_type() {
            FileType::File => (raw::S_IFREG, 0o644),
            FileType::Dir => (raw::S_IFDIR, 0o755),
            FileType::Symlink => (raw::S_IFLNK, 0o777),
            FileType::CharDevice => (raw::S_IFCHR, 0o644),
            FileType::BlockDevice => (raw::S_IFBLK, 0o644),
            FileType::Fifo => (raw::S_IFIFO, 0o644),
            FileType::Socket => (raw::S_IFSOCK, 0o755),
            _ => return Err(error(Detail::Entry, ErrorKind::Unsupported, path)),
        };
        let mut fields = self.fields(attrs, bits, default_mode, path)?;
        let mut data = Data::None;
        if let Some(content) = node.content() {
            fields.len = content.len();
            data = Data::Content(content);
        } else if let Some(target) = node.target() {
            if target.is_empty() {
                return Err(error(Detail::Entry, ErrorKind::InvalidInput, path));
            }
            fields.len = target.len() as u64;
            data = Data::Bytes(target);
        } else if node.file_type() == FileType::Dir {
            fields.nlink = 2;
        }
        if let Some(device) = node.device() {
            fields.rdev = device;
        }
        Ok((fields, data))
    }
}

/// Plans `tree` as `write` writes it: every entry, then the report.
pub(crate) fn plan_tree<'t>(
    tree: &'t Tree,
    options: &CpioOptions,
) -> Result<(Vec<Planned<'t>>, Report), PathError> {
    let mut planner = Planner::new(options);
    let mut out = Vec::new();
    let mut links: BTreeMap<usize, (u64, Vec<usize>)> = BTreeMap::new();
    let mut pending = alloc::vec![(Vec::new(), tree.root())];
    while let Some((path, entry)) = pending.pop() {
        if !path.is_empty() {
            let node = entry.node();
            let (mut fields, data) = planner.node(node, &path)?;
            if node.file_type() == FileType::File && entry.links() > 1 {
                let group = match links.get_mut(&entry.id()) {
                    Some(group) => group,
                    None => {
                        let ino = planner.take_ino();
                        links.entry(entry.id()).or_insert((ino, Vec::new()))
                    }
                };
                fields.ino = group.0;
                fields.nlink = entry.links() as u64;
                group.1.push(out.len());
                let last = group.1.len() == entry.links();
                let names = if last {
                    core::mem::take(&mut group.1)
                } else {
                    Vec::new()
                };
                let data = match last {
                    true => data,
                    false => {
                        fields.len = 0;
                        Data::None
                    }
                };
                let start = planner.entry(&path, &fields, node.attrs())?;
                out.push(Planned {
                    path: path.clone(),
                    fields,
                    data,
                });
                for index in names {
                    let name = out[index].path.clone();
                    planner.extent(&name, start, fields.len);
                }
            } else {
                fields.ino = planner.take_ino();
                let start = planner.entry(&path, &fields, node.attrs())?;
                if node.file_type() == FileType::File {
                    planner.extent(&path, start, fields.len);
                }
                out.push(Planned {
                    path: path.clone(),
                    fields,
                    data,
                });
            }
        }
        let children: Vec<_> = entry.children().collect();
        for (name, child) in children.into_iter().rev() {
            let mut child_path = path.clone();
            if !child_path.is_empty() {
                child_path.push(b'/');
            }
            child_path.extend_from_slice(name.as_bytes());
            pending.push((child_path, child));
        }
    }
    planner.trailer()?;
    Ok((out, planner.finish()))
}

/// Plans writing `tree` as a cpio archive, without I/O, and returns the
/// report `write` returns: the archive size with its trailer, what cpio
/// drops, and where each file's data starts, counted from the start of the
/// archive.
///
/// Fails as `write` does before it writes anything: a name that is empty,
/// holds a NUL or is the trailer's name fails with
/// [`ErrorKind::InvalidInput`], a name over 4095 bytes with
/// [`ErrorKind::NameTooLong`], data the format cannot size with
/// [`ErrorKind::FileTooLarge`], another value that does not fit its field
/// with [`ErrorKind::LimitExceeded`], and [`Format::Binary`] with
/// [`ErrorKind::Unsupported`]. Errors carry the tree path.
pub fn plan(tree: &Tree, options: &CpioOptions) -> Result<Report, PathError> {
    plan_tree(tree, options).map(|(_, report)| report)
}
