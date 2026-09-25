use hadris_fs::{
    Capabilities, CaseRule, Charset, DirCursor, DirEntry, ErrorKind, Field, FileType, FsResult,
    FsStats, Metadata, MountError, MountOptions, Name, NodeId, OpenMode, Owner, Permissions,
    Stored,
};
use hadris_storage::BlockIndex;

use super::FileSystem;
use super::storage::BlockDevice;
use crate::boot::{BootCatalog, CatalogEntry, Emulation};
use crate::error::{Detail, Error};
use crate::info::{DescriptorScan, Info, Root};
use crate::namespace::{Namespace, Namespaces};
use crate::raw::{self, DirectoryRecord, FileFlags, SECTOR_SIZE};
use crate::rock_ridge::{RockRidgeInfo, Scan};
use crate::volume_info::VolumeInfo;

/// The largest device block the reader buffers.
pub(crate) const MAX_DEVICE_BLOCK: usize = 4096;
/// Continuation areas followed for one record.
const MAX_CONTINUATIONS: usize = 16;
/// Records of one multi-extent file.
const MAX_EXTENTS: usize = 1 << 16;

io_transform! {

/// Reads `buf.len()` bytes from byte `offset` of `dev`, which holds `len`
/// bytes.
pub(crate) async fn read_bytes<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    let end = offset
        .checked_add(buf.len() as u64)
        .ok_or(Detail::OutsideImage.corrupt())?;
    if end > len {
        return Err(Detail::OutsideImage.corrupt());
    }
    let bs = u64::from(dev.block_size().get());
    let mut done = 0;
    let mut pos = offset;
    while done < buf.len() {
        let block = BlockIndex::new(pos / bs);
        let within = (pos % bs) as usize;
        let left = buf.len() - done;
        if within == 0 && left as u64 >= bs {
            let whole = left - left % bs as usize;
            dev.read_blocks(block, &mut buf[done..done + whole])
                .await?;
            done += whole;
            pos += whole as u64;
        } else {
            let mut scratch = [0u8; MAX_DEVICE_BLOCK];
            let scratch = &mut scratch[..bs as usize];
            dev.read_blocks(block, scratch).await?;
            let take = (bs as usize - within).min(left);
            buf[done..done + take].copy_from_slice(&scratch[within..within + take]);
            done += take;
            pos += take as u64;
        }
    }
    Ok(())
}

/// Reads the descriptor set and the root directory of the primary tree.
async fn read_info<D: BlockDevice>(dev: &mut D) -> Result<Info, Error<D::Error>> {
    let block = dev.block_size().get() as usize;
    if block > MAX_DEVICE_BLOCK {
        return Err(Detail::BlockSize.error(ErrorKind::Unsupported));
    }
    let len = dev.block_count().saturating_mul(block as u64);
    let mut scan = DescriptorScan::new();
    let mut sector = [0u8; SECTOR_SIZE];
    let mut index = u64::from(raw::DESCRIPTOR_START);
    loop {
        read_bytes(dev, len, index * SECTOR_SIZE as u64, &mut sector).await?;
        let first = index == u64::from(raw::DESCRIPTOR_START);
        let done = scan.feed(&sector).map_err(|detail| match detail {
            Detail::DescriptorHeader if first => detail.error(ErrorKind::NotRecognized),
            _ => detail.corrupt(),
        })?;
        if done {
            break;
        }
        index += 1;
    }
    let mut info = scan.finish().map_err(Detail::corrupt)?;
    let mut probe = View::new(info, Namespace::Primary, info.primary, len);
    info.rock_ridge = probe.detect_rock_ridge(dev).await?;
    Ok(info)
}

/// A mounted ISO 9660 image, read through one of its directory trees.
///
/// It reads the volume descriptors once, when mounted, and needs no
/// allocator. [`mount`](Self::mount) reads the most capable tree the image
/// has (Rock Ridge, then Joliet, then the enhanced tree, then the primary
/// tree); [`mount_namespace`](Self::mount_namespace) picks one. It
/// implements the read-only `hadris_fs` `FileSystem` trait.
///
/// Node ids are byte offsets of directory records: a directory's is its
/// `.` record, so a relocated directory has one id, and a file's is its
/// first record in its parent. In the Rock Ridge tree the names of a hard
/// link share one id, that of the first record in path table order with
/// the same `PX` serial number (or, without one, the same data): listing a
/// file with more than one link scans the directories before it. Every id
/// is stable, so `forget` does nothing. An id no record can have (zero, odd, or past the volume
/// and the device) fails with [`ErrorKind::InvalidHandle`]; any other id is
/// read as a record, and a damaged one fails with [`ErrorKind::Corrupt`].
/// Write methods fail with [`ErrorKind::ReadOnly`].
///
/// Names drop the `;1` version. In the primary and enhanced trees a lookup
/// that finds no exact name retries ignoring ASCII case.
///
/// ```rust,ignore
/// let mut iso = IsoFs::mount(dev, MountOptions::new())?;
/// let node = iso.resolve(b"/boot/grub/grub.cfg", Resolve::Lexical)?;
/// iso.open(node, OpenMode::Read)?;
/// let n = iso.read(node, 0, &mut buf)?;
/// ```
#[derive(Debug)]
pub struct IsoFs<D> {
    dev: D,
    view: View,
}

/// The device-independent state of a mount.
#[derive(Debug, Clone, Copy)]
struct View {
    info: Info,
    namespace: Namespace,
    root: Root,
    len: u64,
}

/// A directory: where its records are.
#[derive(Debug, Clone, Copy)]
struct Dir {
    start: u64,
    size: u32,
}

/// A record found while walking a directory.
struct Found {
    offset: u64,
    record: DirectoryRecord,
}

/// The logical block a walk has loaded.
struct Block {
    index: u64,
    data: [u8; SECTOR_SIZE],
}

impl Block {
    fn new() -> Self {
        Self {
            index: u64::MAX,
            data: [0; SECTOR_SIZE],
        }
    }
}

/// What a path table scan looks for.
#[derive(Clone, Copy)]
enum PathTableQuery {
    Extent(u64),
    Number(u64),
}

/// What the names of one hard-linked file share.
#[derive(Clone, Copy)]
enum LinkKey {
    /// The Rock Ridge `PX` serial number.
    Serial(u32),
    /// The byte offset of the data.
    Extent(u64),
}

/// A listed entry: its id and name length.
struct Listed {
    node: NodeId,
    len: usize,
}

/// A node id no record can have (zero, odd, or past both the volume and
/// the device) is an invalid handle. Any other id is read as a record, so a
/// damaged record, or one past the end of a truncated image, is corrupt.
fn handle<E>(err: Error<E>, node: NodeId, view: &View) -> hadris_fs::Error<E> {
    let volume_end = u64::from(view.info.volume_blocks) * u64::from(view.info.block_size);
    let id = node.get();
    if err.kind() == ErrorKind::Io || (id != 0 && id % 2 == 0 && id < volume_end.max(view.len)) {
        err
    } else {
        ErrorKind::InvalidHandle.into()
    }
}

impl View {
    fn new(info: Info, namespace: Namespace, root: Root, len: u64) -> Self {
        Self {
            info,
            namespace,
            root,
            len,
        }
    }

    fn bs(&self) -> u64 {
        u64::from(self.info.block_size)
    }

    fn rock_ridge(&self) -> Option<u8> {
        match self.namespace {
            Namespace::RockRidge => self.info.rock_ridge,
            _ => None,
        }
    }

    fn root_id(&self) -> NodeId {
        const FIRST: NodeId = match NodeId::new(1) {
            Some(node) => node,
            None => panic!(),
        };
        NodeId::new(u64::from(self.root.extent) * self.bs()).unwrap_or(FIRST)
    }

    fn extent_start(&self, record: &DirectoryRecord) -> Option<u64> {
        let header = record.header();
        let block = u64::from(header.extent.get()) + u64::from(header.extended_attr_record);
        block.checked_mul(self.bs())
    }

    async fn record_at<D: BlockDevice>(&self, dev: &mut D, offset: u64) -> Result<DirectoryRecord, Error<D::Error>> {
        let bs = self.bs();
        if offset == 0 {
            return Err(Detail::DirectoryRecord.corrupt());
        }
        let mut block = [0u8; SECTOR_SIZE];
        let block = &mut block[..bs as usize];
        read_bytes(dev, self.len, offset - offset % bs, block).await?;
        let within = (offset % bs) as usize;
        match DirectoryRecord::parse(&block[within..]) {
            Ok(Some(record)) => Ok(record),
            _ => Err(Detail::DirectoryRecord.corrupt()),
        }
    }

    /// The directory a node names: its `.` record, or a directory record.
    async fn dir_of<D: BlockDevice>(&self, dev: &mut D, node: NodeId) -> FsResult<Dir, D::Error> {
        let record = self.record_at(dev, node.get()).await.map_err(|err| handle(err, node, self))?;
        if !record.header().is_directory() {
            return Err(ErrorKind::NotADirectory.into());
        }
        let start = self
            .extent_start(&record)
            .ok_or(ErrorKind::Corrupt)?;
        Ok(Dir {
            start,
            size: record.header().data_len.get(),
        })
    }

    /// The record at `pos` in `dir`, skipping sector padding. Advances
    /// `pos` past it.
    async fn next_record<D: BlockDevice>(
        &self,
        dev: &mut D,
        dir: Dir,
        pos: &mut u32,
        block: &mut Block,
    ) -> Result<Option<Found>, Error<D::Error>> {
        let bs = self.bs() as u32;
        while *pos < dir.size {
            let index = u64::from(*pos / bs);
            if block.index != index {
                let data = &mut block.data[..bs as usize];
                read_bytes(dev, self.len, dir.start + index * u64::from(bs), data).await?;
                block.index = index;
            }
            let within = (*pos % bs) as usize;
            let data = &block.data[within..bs as usize];
            match DirectoryRecord::parse(data) {
                Ok(Some(record)) => {
                    let offset = dir.start + u64::from(*pos);
                    *pos += record.len() as u32;
                    return Ok(Some(Found { offset, record }));
                }
                Ok(None) => *pos = (*pos / bs + 1) * bs,
                Err(()) => return Err(Detail::DirectoryRecord.corrupt()),
            }
        }
        Ok(None)
    }

    /// Skips the continuation records of a multi-extent file whose first
    /// record `first` was just read.
    async fn skip_continuations<D: BlockDevice>(
        &self,
        dev: &mut D,
        dir: Dir,
        pos: &mut u32,
        block: &mut Block,
        first: &DirectoryRecord,
    ) -> Result<(), Error<D::Error>> {
        let mut last = *first;
        for _ in 0..MAX_EXTENTS {
            if !last.header().file_flags().contains(FileFlags::NOT_FINAL) {
                return Ok(());
            }
            let next = self
                .next_record(dev, dir, pos, block)
                .await?
                .ok_or(Detail::MultiExtent.corrupt())?;
            if next.record.name() != first.name() {
                return Err(Detail::MultiExtent.corrupt());
            }
            last = next.record;
        }
        Err(Detail::MultiExtent.corrupt())
    }

    /// Follows the system use area of `record` through its continuation
    /// areas into `scan`.
    async fn scan<D: BlockDevice>(
        &self,
        dev: &mut D,
        record: &DirectoryRecord,
        skip: u8,
        scan: &mut Scan<'_>,
    ) -> Result<(), Error<D::Error>> {
        scan.feed(record.system_use(), usize::from(skip));
        let mut area = [0u8; SECTOR_SIZE];
        for _ in 0..MAX_CONTINUATIONS {
            let Some(ce) = scan.next else {
                return Ok(());
            };
            let len = (ce.length.get() as usize).min(SECTOR_SIZE);
            let offset = u64::from(ce.block.get()) * self.bs() + u64::from(ce.offset.get());
            if len == 0 {
                return Ok(());
            }
            read_bytes(dev, self.len, offset, &mut area[..len]).await?;
            scan.feed(&area[..len], 0);
        }
        Ok(())
    }

    /// Whether the root's `.` record starts a Rock Ridge area.
    async fn detect_rock_ridge<D: BlockDevice>(&mut self, dev: &mut D) -> Result<Option<u8>, Error<D::Error>> {
        let dot = self.record_at(dev, self.root_id().get()).await?;
        let mut scan = Scan::new();
        self.scan(dev, &dot, 0, &mut scan).await?;
        Ok(match scan.sp {
            Some(skip) if scan.rrip || scan.info.mode().is_some() => Some(skip),
            _ => None,
        })
    }

    /// The id of the non-directory record at `offset`. A Rock Ridge file
    /// with more than one link gets the id of the first record of the same
    /// file in path table order, so all its names share one id: records
    /// with its `PX` serial number, or without one, at its data extent.
    async fn link_id<D: BlockDevice>(
        &self,
        dev: &mut D,
        offset: u64,
        record: &DirectoryRecord,
        info: &RockRidgeInfo,
        skip: u8,
    ) -> Result<u64, Error<D::Error>> {
        if info.links().unwrap_or(1) <= 1 {
            return Ok(offset);
        }
        let key = match info.serial().filter(|&serial| serial != 0) {
            Some(serial) => LinkKey::Serial(serial),
            None if record.header().data_len.get() > 0 && info.file_type().is_none_or(|t| t == FileType::File) => {
                match self.extent_start(record) {
                    Some(start) => LinkKey::Extent(start),
                    None => return Ok(offset),
                }
            }
            None => return Ok(offset),
        };
        Ok(self.first_link(dev, key, skip).await?.unwrap_or(offset))
    }

    /// The first record in path table order of the hard-linked file `key`
    /// names.
    async fn first_link<D: BlockDevice>(&self, dev: &mut D, key: LinkKey, skip: u8) -> Result<Option<u64>, Error<D::Error>> {
        let (table, size) = self.root.path_table;
        let base = u64::from(table) * self.bs();
        let mut pos = 0u64;
        while pos + 8 <= u64::from(size) {
            let mut header = [0u8; 8];
            read_bytes(dev, self.len, base + pos, &mut header).await?;
            let header: raw::PathTableHeader = bytemuck::cast(header);
            pos += header.record_len() as u64;
            let start = (u64::from(header.extent_le()) + u64::from(header.extended_attr_record)) * self.bs();
            let dot = self.record_at(dev, start).await?;
            let dir = Dir {
                start,
                size: dot.header().data_len.get(),
            };
            let mut at = 0;
            let mut block = Block::new();
            while let Some(found) = self.next_record(dev, dir, &mut at, &mut block).await? {
                let record = found.record;
                self.skip_continuations(dev, dir, &mut at, &mut block, &record).await?;
                if record.is_dot() || record.header().is_directory() {
                    continue;
                }
                let mut scan = Scan::new();
                self.scan(dev, &record, skip, &mut scan).await?;
                let info = scan.info;
                if info.links().unwrap_or(1) <= 1 || info.is_relocated() || info.child_link().is_some() {
                    continue;
                }
                let same = match key {
                    LinkKey::Serial(serial) => info.serial() == Some(serial),
                    LinkKey::Extent(extent) => {
                        record.header().data_len.get() > 0 && self.extent_start(&record) == Some(extent)
                    }
                };
                if same {
                    return Ok(Some(found.offset));
                }
            }
        }
        Ok(None)
    }

    /// The name, type and id a record lists under, or `None` for records
    /// the listing hides.
    async fn list<D: BlockDevice>(
        &self,
        dev: &mut D,
        found: &Found,
        out: &mut [u8],
    ) -> Result<Option<Listed>, Error<D::Error>> {
        let record = &found.record;
        let header = record.header();
        if record.is_dot() || header.file_flags().contains(FileFlags::ASSOCIATED_FILE) {
            return Ok(None);
        }
        let is_dir = header.is_directory();
        let end = self.len.max(u64::from(self.info.volume_blocks) * self.bs());
        let dir_id = |start: Option<u64>| match start {
            Some(start) if start != 0 && start < end => NodeId::new(start).ok_or(Detail::DirectoryRecord.corrupt()),
            Some(start) if start != 0 => Err(Detail::OutsideImage.corrupt()),
            _ => Err(Detail::DirectoryRecord.corrupt()),
        };
        if let Some(skip) = self.rock_ridge() {
            let mut name = [0u8; 1024];
            let mut scan = Scan::new().with_name(&mut name);
            self.scan(dev, record, skip, &mut scan).await?;
            if scan.info.is_relocated() {
                return Ok(None);
            }
            let len = match scan.name().map_err(Error::from)? {
                Some(bytes) => crate::name::sanitize(bytes, out),
                None => crate::name::sanitize(crate::name::strip_version(record.name()), out),
            }
            .ok_or(Error::from(ErrorKind::NameTooLong))?;
            let info = scan.info;
            if let Some(block) = info.child_link() {
                let start = u64::from(block).checked_mul(self.bs());
                return Ok(Some(Listed { node: dir_id(start)?, len }));
            }
            let node = if is_dir {
                dir_id(self.extent_start(record))?
            } else {
                let node = self.link_id(dev, found.offset, record, &info, skip).await?;
                NodeId::new(node).ok_or(Detail::DirectoryRecord.corrupt())?
            };
            return Ok(Some(Listed { node, len }));
        }
        let len = match self.namespace {
            Namespace::Joliet => crate::name::decode_ucs2(record.name(), out),
            _ => crate::name::sanitize(crate::name::strip_version(record.name()), out),
        }
        .ok_or(Error::from(ErrorKind::NameTooLong))?;
        let node = if is_dir {
            dir_id(self.extent_start(record))?
        } else {
            NodeId::new(found.offset).ok_or(Detail::DirectoryRecord.corrupt())?
        };
        Ok(Some(Listed { node, len }))
    }

    fn case_insensitive(&self) -> bool {
        matches!(self.namespace, Namespace::Primary | Namespace::Enhanced)
    }

    fn capabilities(&self) -> Capabilities {
        match self.namespace {
            Namespace::RockRidge => Capabilities::new(CaseRule::Sensitive, Charset::Bytes, 255)
                .with_symlinks()
                .with_hard_links()
                .with_stored(Field::Created, Stored::Partial)
                .with_stored(Field::Accessed, Stored::Partial)
                .with_stored(Field::Changed, Stored::Partial)
                .with_stored(Field::Permissions, Stored::Yes)
                .with_stored(Field::Owner, Stored::Yes)
                .with_stored(Field::Device, Stored::Yes),
            Namespace::Joliet => Capabilities::new(CaseRule::Sensitive, Charset::Unicode, 309),
            Namespace::Enhanced => Capabilities::new(CaseRule::InsensitivePreserving, Charset::Bytes, 207),
            _ => Capabilities::new(CaseRule::Insensitive, Charset::Bytes, 207),
        }
        .with_stored(Field::Modified, Stored::Yes)
    }

    /// Writes the volume identifier of this view's descriptor into `buf`.
    async fn label<'b, D: BlockDevice>(&self, dev: &mut D, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let mut sector = [0u8; SECTOR_SIZE];
        let mut len = None;
        for index in 0..self.info.descriptors {
            let offset = u64::from(raw::DESCRIPTOR_START + index) * SECTOR_SIZE as u64;
            read_bytes(dev, self.len, offset, &mut sector).await?;
            let (root, id, ucs2) = match raw::VolumeDescriptor::from_bytes(sector) {
                raw::VolumeDescriptor::Primary(pvd) => (pvd.root, pvd.volume_identifier, false),
                raw::VolumeDescriptor::Supplementary(svd) => {
                    (svd.root, svd.volume_identifier, self.namespace == Namespace::Joliet)
                }
                _ => continue,
            };
            let header = &root.header;
            if header.extent.get().checked_add(u32::from(header.extended_attr_record)) != Some(self.root.extent) {
                continue;
            }
            len = Some(if ucs2 {
                let raw = id.as_bytes();
                let mut end = raw.len() / 2;
                while end > 0 && matches!([raw[2 * end - 2], raw[2 * end - 1]], [0, b' ' | 0]) {
                    end -= 1;
                }
                crate::name::label_ucs2(&raw[..2 * end], buf).ok_or(ErrorKind::LimitExceeded)?
            } else {
                crate::name::label_latin1(id.trimmed(), buf).ok_or(ErrorKind::LimitExceeded)?
            });
            break;
        }
        match len {
            Some(0) | None => Ok(None),
            Some(len) => Ok(core::str::from_utf8(&buf[..len]).ok()),
        }
    }

    async fn lookup<D: BlockDevice>(&self, dev: &mut D, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        let dir = self.dir_of(dev, dir).await?;
        let mut pos = 0;
        let mut block = Block::new();
        let mut buf = [0u8; 1024];
        let mut folded = None;
        while let Some(found) = self.next_record(dev, dir, &mut pos, &mut block).await? {
            let listed = self.list(dev, &found, &mut buf).await?;
            self.skip_continuations(dev, dir, &mut pos, &mut block, &found.record).await?;
            let Some(listed) = listed else {
                continue;
            };
            let listed_name = &buf[..listed.len];
            if listed_name == name.as_bytes() {
                return Ok(listed.node);
            }
            if folded.is_none() && self.case_insensitive() && listed_name.eq_ignore_ascii_case(name.as_bytes()) {
                folded = Some(listed.node);
            }
        }
        folded.ok_or(ErrorKind::NotFound.into())
    }

    async fn readdir<D: BlockDevice>(&self, dev: &mut D, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        let dir = self.dir_of(dev, dir).await?;
        let Ok(mut pos) = u32::try_from(from.into_raw()) else {
            return Ok(None);
        };
        let mut block = Block::new();
        while let Some(found) = self.next_record(dev, dir, &mut pos, &mut block).await? {
            let mut out = [0u8; 1024];
            let listed = self.list(dev, &found, &mut out).await?;
            self.skip_continuations(dev, dir, &mut pos, &mut block, &found.record).await?;
            if let Some(listed) = listed {
                let meta = self.stat(dev, listed.node).await?;
                let next = DirCursor::from_raw(u64::from(pos));
                return Ok(Some(DirEntry::new(Name::new(&out[..listed.len]), listed.node, meta, next)?));
            }
        }
        Ok(None)
    }

    async fn stat<D: BlockDevice>(&self, dev: &mut D, node: NodeId) -> FsResult<Metadata, D::Error> {
        let record = self.record_at(dev, node.get()).await.map_err(|err| handle(err, node, self))?;
        let header = *record.header();
        let rr = match self.rock_ridge() {
            Some(skip) => {
                let mut scan = Scan::new();
                self.scan(dev, &record, skip, &mut scan).await?;
                Some(scan.info)
            }
            None => None,
        };
        let (mut file_type, mut len) = if header.is_directory() {
            (FileType::Dir, u64::from(header.data_len.get()))
        } else {
            let file_type = rr.and_then(|rr| rr.file_type()).unwrap_or(FileType::File);
            (file_type, self.file_len(dev, node.get(), &record).await?)
        };
        let mut times = [None, header.date_time.to_datetime(), None, None];
        let mut permissions = Permissions::new(if file_type.is_dir() { 0o555 } else { 0o444 });
        let mut owner = None;
        let mut nlink = 1;
        let mut device = None;
        if let Some(rr) = rr {
            if rr.has_times() {
                times = [rr.created(), rr.modified(), rr.accessed(), rr.changed()];
            }
            if let Some(mode) = rr.mode() {
                permissions = Permissions::new(mode);
            }
            owner = rr.owner().map(|(uid, gid)| Owner::new(uid, gid));
            nlink = u64::from(rr.links().unwrap_or(1));
            device = rr.device();
            if rr.is_symlink() {
                let mut target = [0u8; 4096];
                let mut scan = Scan::new().with_link(&mut target);
                self.scan(dev, &record, self.rock_ridge().unwrap_or(0), &mut scan).await?;
                file_type = FileType::Symlink;
                len = scan.link_len()? as u64;
            }
        }
        let mut meta = Metadata::new(file_type, permissions).with_len(len).with_nlink(nlink);
        if let Some(owner) = owner {
            meta = meta.with_owner(owner);
        }
        if let Some(device) = device.filter(|_| matches!(file_type, FileType::CharDevice | FileType::BlockDevice)) {
            meta = meta.with_device(device);
        }
        if let Some(time) = times[0] {
            meta = meta.with_created(time);
        }
        if let Some(time) = times[1] {
            meta = meta.with_modified(time);
        }
        if let Some(time) = times[2] {
            meta = meta.with_accessed(time);
        }
        if let Some(time) = times[3] {
            meta = meta.with_changed(time);
        }
        Ok(meta)
    }

    /// The whole length of the file whose first record is `record`.
    async fn file_len<D: BlockDevice>(&self, dev: &mut D, offset: u64, record: &DirectoryRecord) -> Result<u64, Error<D::Error>> {
        let mut total = u64::from(record.header().data_len.get());
        let mut current = (offset, *record);
        for _ in 0..MAX_EXTENTS {
            if !current.1.header().file_flags().contains(FileFlags::NOT_FINAL) {
                return Ok(total);
            }
            current = self.following(dev, current.0, &current.1).await?;
            if current.1.name() != record.name() {
                return Err(Detail::MultiExtent.corrupt());
            }
            total += u64::from(current.1.header().data_len.get());
        }
        Err(Detail::MultiExtent.corrupt())
    }

    /// The record after the one at `offset`, in the same block or at the
    /// start of the next.
    async fn following<D: BlockDevice>(&self, dev: &mut D, offset: u64, record: &DirectoryRecord) -> Result<(u64, DirectoryRecord), Error<D::Error>> {
        let bs = self.bs();
        let next = offset + record.len() as u64;
        if next % bs != 0 {
            let mut len = [0u8; 1];
            read_bytes(dev, self.len, next, &mut len).await?;
            if len[0] != 0 {
                return Ok((next, self.record_at(dev, next).await?));
            }
        }
        let next = (offset / bs + 1) * bs;
        Ok((next, self.record_at(dev, next).await?))
    }

    async fn read<D: BlockDevice>(&self, dev: &mut D, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let record = self.record_at(dev, node.get()).await.map_err(|err| handle(err, node, self))?;
        if record.header().is_directory() {
            return Err(ErrorKind::IsADirectory.into());
        }
        let mut current = (node.get(), record);
        let mut start = 0u64;
        for _ in 0..MAX_EXTENTS {
            let header = *current.1.header();
            if header.file_unit_size != 0 || header.interleave_gap_size != 0 || header.volume_sequence_number.get() > 1 {
                return Err(Detail::Interleaved.error(ErrorKind::Unsupported));
            }
            let len = u64::from(header.data_len.get());
            if offset < start + len {
                let within = offset - start;
                let take = usize::try_from(len - within).unwrap_or(usize::MAX).min(buf.len());
                let base = self.extent_start(&current.1).ok_or(ErrorKind::Corrupt)?;
                read_bytes(dev, self.len, base + within, &mut buf[..take]).await?;
                return Ok(take);
            }
            start += len;
            if !header.file_flags().contains(FileFlags::NOT_FINAL) {
                return Ok(0);
            }
            let next = self.following(dev, current.0, &current.1).await?;
            if next.1.name() != record.name() {
                return Err(Detail::MultiExtent.corrupt());
            }
            current = next;
        }
        Err(Detail::MultiExtent.corrupt())
    }

    async fn parent<D: BlockDevice>(&self, dev: &mut D, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let dir = self.dir_of(dev, dir).await?;
        let dot = self.record_at(dev, dir.start).await?;
        let dotdot = self.record_at(dev, dir.start + dot.len() as u64).await?;
        if let Some(skip) = self.rock_ridge() {
            let mut scan = Scan::new();
            self.scan(dev, &dotdot, skip, &mut scan).await?;
            if let Some(block) = scan.info.parent_link() {
                return NodeId::new(u64::from(block) * self.bs()).ok_or(ErrorKind::Corrupt.into());
            }
        }
        match self.extent_start(&dotdot) {
            Some(start) if start == dir.start && start != self.root_id().get() => {
                NodeId::new(self.path_table_parent(dev, dir.start).await?).ok_or(ErrorKind::Corrupt.into())
            }
            Some(start) => NodeId::new(start).ok_or(ErrorKind::Corrupt.into()),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// The parent of the directory at byte `start`, from the path table,
    /// for trees whose `..` records point at the directory itself (the
    /// Joliet trees libisofs writes).
    async fn path_table_parent<D: BlockDevice>(&self, dev: &mut D, start: u64) -> Result<u64, Error<D::Error>> {
        let parent = self.path_table_find(dev, PathTableQuery::Extent(start / self.bs())).await?;
        Ok(self.path_table_find(dev, PathTableQuery::Number(parent)).await? * self.bs())
    }

    /// Scans the little-endian path table for a directory's extent, giving
    /// its parent's number, or for a record number, giving its extent.
    async fn path_table_find<D: BlockDevice>(&self, dev: &mut D, want: PathTableQuery) -> Result<u64, Error<D::Error>> {
        let (table, size) = self.root.path_table;
        let base = u64::from(table) * self.bs();
        let mut pos = 0u64;
        let mut number = 1u64;
        while pos + 8 <= u64::from(size) {
            let mut header = [0u8; 8];
            read_bytes(dev, self.len, base + pos, &mut header).await?;
            let header: raw::PathTableHeader = bytemuck::cast(header);
            let extent = u64::from(header.extent_le()) + u64::from(header.extended_attr_record);
            match want {
                PathTableQuery::Extent(block) if extent == block => {
                    return Ok(u64::from(header.parent_le()));
                }
                PathTableQuery::Number(n) if number == n => return Ok(extent),
                _ => {}
            }
            pos += header.record_len() as u64;
            number += 1;
        }
        Err(Detail::DirectoryRecord.corrupt())
    }

    async fn readlink<D: BlockDevice>(&self, dev: &mut D, link: NodeId, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let Some(skip) = self.rock_ridge() else {
            return Err(ErrorKind::InvalidInput.into());
        };
        let record = self.record_at(dev, link.get()).await.map_err(|err| handle(err, link, self))?;
        let mut scan = Scan::new().with_link(buf);
        self.scan(dev, &record, skip, &mut scan).await?;
        if !scan.info.is_symlink() {
            return Err(ErrorKind::InvalidInput.into());
        }
        Ok(scan.link_len()?)
    }

    async fn rock_ridge_info<D: BlockDevice>(&self, dev: &mut D, node: NodeId) -> Result<Option<RockRidgeInfo>, Error<D::Error>> {
        let skip = match (self.namespace, self.info.rock_ridge) {
            (Namespace::RockRidge | Namespace::Primary, Some(skip)) => skip,
            _ => return Ok(None),
        };
        let record = self.record_at(dev, node.get()).await?;
        let mut scan = Scan::new();
        self.scan(dev, &record, skip, &mut scan).await?;
        Ok(Some(scan.info))
    }
}

impl<D: BlockDevice> IsoFs<D> {
    /// Mounts the image on `dev` with its most capable tree: Rock Ridge,
    /// then Joliet, then the enhanced tree, then the primary tree. Reads
    /// the volume descriptors from logical sector 16 and checks the primary
    /// tree for Rock Ridge. ISO 9660 has no backup structures, so
    /// [`MountOptions::backup_boot`] changes nothing, and the mount is
    /// read-only whatever `options` say.
    ///
    /// Fails with [`ErrorKind::NotRecognized`] when the first volume
    /// descriptor is not ISO 9660, with [`ErrorKind::Corrupt`] when the
    /// descriptor set is invalid, and with [`ErrorKind::Unsupported`] for
    /// device blocks above 4096 bytes. The [`MountError`] gives `dev` back.
    pub async fn mount(dev: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_namespace(dev, options, Namespace::Preferred).await
    }

    /// Mounts the image on `dev` as [`mount`](Self::mount) does, reading
    /// the tree `namespace` names. Fails with [`ErrorKind::NotFound`] and
    /// [`Detail::NoNamespace`] when the image has no such tree.
    pub async fn mount_namespace(
        mut dev: D,
        options: MountOptions,
        namespace: Namespace,
    ) -> Result<Self, MountError<D, D::Error>> {
        let _ = options;
        let info = match read_info(&mut dev).await {
            Ok(info) => info,
            Err(err) => return Err(MountError::new(err, dev)),
        };
        let len = dev.block_count().saturating_mul(u64::from(dev.block_size().get()));
        match info.tree(namespace) {
            Some((namespace, root)) => Ok(Self {
                dev,
                view: View::new(info, namespace, root, len),
            }),
            None => Err(MountError::new(Detail::NoNamespace.error(ErrorKind::NotFound), dev)),
        }
    }

    /// Gives the device back. The image is read-only, so there is nothing
    /// to sync and this never fails.
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        Ok(self.dev)
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// Borrows the device.
    pub fn device(&self) -> &D {
        &self.dev
    }

    /// The tree this mount reads; never [`Namespace::Preferred`].
    pub fn namespace(&self) -> Namespace {
        self.view.namespace
    }

    /// The trees the image has.
    pub fn namespaces(&self) -> Namespaces {
        self.view.info.namespaces()
    }

    /// What the primary volume descriptor records: the block size, the
    /// volume size, the identifiers and the dates.
    pub fn info(&self) -> &VolumeInfo {
        &self.view.info.volume
    }

    /// Reads `buf.len()` bytes from byte `offset` of the image.
    pub async fn read_raw(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error<D::Error>> {
        read_bytes(&mut self.dev, self.view.len, offset, buf).await
    }

    /// The volume descriptor `index` places after the first one, at logical
    /// sector 16, or `None` past the set terminator.
    pub async fn descriptor(&mut self, index: u32) -> Result<Option<raw::VolumeDescriptor>, Error<D::Error>> {
        if index >= self.view.info.descriptors {
            return Ok(None);
        }
        let mut sector = [0u8; SECTOR_SIZE];
        let offset = u64::from(raw::DESCRIPTOR_START + index) * SECTOR_SIZE as u64;
        self.read_raw(offset, &mut sector).await?;
        Ok(Some(raw::VolumeDescriptor::from_bytes(sector)))
    }

    /// The primary volume descriptor.
    #[cfg(feature = "alloc")]
    pub(crate) async fn primary_descriptor(&mut self) -> Result<raw::PrimaryVolumeDescriptor, Error<D::Error>> {
        let mut index = 0;
        while let Some(descriptor) = self.descriptor(index).await? {
            if let raw::VolumeDescriptor::Primary(pvd) = descriptor {
                return Ok(pvd);
            }
            index += 1;
        }
        Err(Detail::NoPrimaryDescriptor.corrupt())
    }

    /// The logical block of the boot catalog, when there is a boot record.
    #[cfg(feature = "alloc")]
    pub(crate) fn catalog_block(&self) -> Option<u32> {
        self.view.info.boot_catalog
    }

    /// Reads the El Torito boot catalog into `buf` and checks it, or
    /// returns `None` without a boot record. Its entries are parsed as
    /// they are iterated; [`boot_image`](Self::boot_image) locates each
    /// entry's image.
    ///
    /// Fails with [`ErrorKind::LimitExceeded`] when the catalog does not
    /// fit in `buf` (2048 bytes hold 63 entries), and with
    /// [`ErrorKind::Corrupt`] and [`Detail::BootCatalog`] when the
    /// validation entry is wrong or the catalog runs past 1024 entries.
    pub async fn boot_catalog<'b>(&mut self, buf: &'b mut [u8]) -> Result<Option<BootCatalog<'b>>, Error<D::Error>> {
        use crate::boot::{CatalogParser, Step};

        let Some(block) = self.view.info.boot_catalog else {
            return Ok(None);
        };
        let start = u64::from(block) * u64::from(self.view.info.block_size);
        let usable = buf.len() - buf.len() % 32;
        let mut parser = CatalogParser::new();
        let mut read = 0;
        let mut end = 0;
        while !parser.is_done() {
            if end / 32 > CatalogParser::MAX_ENTRIES + 1 {
                return Err(Detail::BootCatalog.corrupt());
            }
            if end == read {
                if read == usable {
                    return Err(ErrorKind::LimitExceeded.into());
                }
                let want = (SECTOR_SIZE - read % SECTOR_SIZE).min(usable - read);
                self.read_raw(start + read as u64, &mut buf[read..read + want]).await?;
                read += want;
            }
            let mut chunk = [0u8; 32];
            chunk.copy_from_slice(&buf[end..end + 32]);
            match parser.feed(&chunk).map_err(|()| Detail::BootCatalog.corrupt())? {
                Step::Done => break,
                Step::Entry(_) | Step::More => end += 32,
            }
        }
        let buf: &'b [u8] = buf;
        Ok(Some(BootCatalog::new(block, &buf[..end])))
    }

    /// Where the image of a boot catalog entry lies: from its load block,
    /// the size of the emulated diskette, or the 512-byte sectors the entry
    /// loads for no emulation and hard disk emulation. Read it with
    /// [`read_raw`](Self::read_raw).
    pub fn boot_image(&self, entry: &CatalogEntry) -> hadris_fs::Extent {
        let start = u64::from(entry.load_block()) * u64::from(self.view.info.block_size);
        let len = match entry.emulation() {
            Some(Emulation::Floppy12) => 1_228_800,
            Some(Emulation::Floppy144) => 1_474_560,
            Some(Emulation::Floppy288) => 2_949_120,
            _ => u64::from(entry.sector_count()) * 512,
        };
        hadris_fs::Extent::new(start, len)
    }

    /// The Rock Ridge entries of a node in the primary tree, or `None` when
    /// the image has no Rock Ridge or the mount reads another tree.
    pub async fn rock_ridge(&mut self, node: NodeId) -> Result<Option<RockRidgeInfo>, Error<D::Error>> {
        self.view.rock_ridge_info(&mut self.dev, node).await
    }

    /// Whether `dir` holds records the Rock Ridge tree hides because an
    /// `RE` entry marks them relocated, and nothing else.
    #[cfg(feature = "alloc")]
    pub(crate) async fn is_relocation_dir(&mut self, dir: NodeId) -> Result<bool, Error<D::Error>> {
        let Some(skip) = self.view.rock_ridge() else {
            return Ok(false);
        };
        let dir = self.view.dir_of(&mut self.dev, dir).await?;
        let mut pos = 0;
        let mut block = Block::new();
        let mut relocated = false;
        while let Some(found) = self.view.next_record(&mut self.dev, dir, &mut pos, &mut block).await? {
            if found.record.is_dot() {
                continue;
            }
            let mut scan = Scan::new();
            self.view.scan(&mut self.dev, &found.record, skip, &mut scan).await?;
            if !scan.info.is_relocated() {
                return Ok(false);
            }
            relocated = true;
        }
        Ok(relocated)
    }

    /// Maps a file to the device: fills `out` with its extents that end
    /// after file offset `from`, in order, and returns how many it filled.
    /// Call again from the end of the last one for more; 0 means there are
    /// none. A file of several extents (multi-extent) has one per
    /// directory record; a directory has the one extent of its records.
    pub async fn extents(&mut self, node: NodeId, from: u64, out: &mut [hadris_fs::Extent]) -> Result<usize, Error<D::Error>> {
        let record = self.view.record_at(&mut self.dev, node.get()).await?;
        let mut current = (node.get(), record);
        let mut file = 0u64;
        let mut count = 0;
        for _ in 0..MAX_EXTENTS {
            let start = self.view.extent_start(&current.1).ok_or(Detail::DirectoryRecord.corrupt())?;
            let len = u64::from(current.1.header().data_len.get());
            if file + len > from && len > 0 {
                let Some(slot) = out.get_mut(count) else {
                    return Ok(count);
                };
                *slot = hadris_fs::Extent::new(start, len).with_file_offset(file);
                count += 1;
            }
            file += len;
            if !current.1.header().file_flags().contains(FileFlags::NOT_FINAL) {
                return Ok(count);
            }
            current = self.view.following(&mut self.dev, current.0, &current.1).await?;
            if current.1.name() != record.name() {
                return Err(Detail::MultiExtent.corrupt());
            }
        }
        Err(Detail::MultiExtent.corrupt())
    }

    /// Locates a node's directory records: the record its id names (a
    /// directory's `.` record) and, for a file of several extents, the
    /// records that follow it. Read them with [`read_raw`](Self::read_raw).
    /// Returns how many it filled; fails with [`ErrorKind::LimitExceeded`]
    /// when `out` is too short.
    pub async fn records(&mut self, node: NodeId, out: &mut [hadris_fs::Extent]) -> Result<usize, Error<D::Error>> {
        let record = self.view.record_at(&mut self.dev, node.get()).await?;
        let mut current = (node.get(), record);
        for count in 0..MAX_EXTENTS {
            let len = u64::from(current.1.header().len);
            *out.get_mut(count).ok_or(ErrorKind::LimitExceeded)? = hadris_fs::Extent::new(current.0, len);
            if !current.1.header().file_flags().contains(FileFlags::NOT_FINAL) {
                return Ok(count + 1);
            }
            current = self.view.following(&mut self.dev, current.0, &current.1).await?;
        }
        Err(Detail::MultiExtent.corrupt())
    }
}


impl<D: BlockDevice> FileSystem for IsoFs<D> {
    type DeviceError = D::Error;

    /// Symlinks, hard links, permissions and owners with Rock Ridge;
    /// case-insensitive lookups in the primary and enhanced trees. Never
    /// writable.
    fn capabilities(&self) -> Capabilities {
        self.view.capabilities()
    }

    /// The id of the root's `.` record.
    fn root(&self) -> NodeId {
        self.view.root_id()
    }

    /// The volume's size; an ISO image has no free blocks.
    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        Ok(FsStats::new(u64::from(self.view.info.volume_blocks), 0, self.view.info.block_size))
    }

    /// The volume identifier of the descriptor this mount reads: UCS-2 in
    /// the Joliet tree, bytes read as Latin-1 otherwise.
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        self.view.label(&mut self.dev, buf).await
    }

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        self.view.lookup(&mut self.dev, dir, name).await
    }

    /// Does nothing: ISO node ids are stable.
    fn forget(&mut self, node: NodeId, count: u64) {
        let _ = (node, count);
    }

    /// The directory containing `dir`, from its `..` record, or its Rock
    /// Ridge `PL` entry when it was relocated.
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        self.view.parent(&mut self.dev, dir).await
    }

    /// Rock Ridge mode, owner, links and times when the mount reads Rock
    /// Ridge, the record's time as the modification time otherwise.
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        self.view.stat(&mut self.dev, node).await
    }

    /// The cursor is the byte offset of the next record in the directory.
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        self.view.readdir(&mut self.dev, dir, from).await
    }

    /// [`ErrorKind::InvalidInput`] for other nodes and outside the Rock
    /// Ridge tree.
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let len = self.view.readlink(&mut self.dev, node, buf).await?;
        Ok(&buf[..len])
    }

    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        let meta = self.view.stat(&mut self.dev, node).await?;
        match meta.file_type() {
            FileType::Dir => Err(ErrorKind::IsADirectory.into()),
            FileType::Symlink => Err(ErrorKind::Symlink.into()),
            _ if mode == OpenMode::Write => Err(ErrorKind::ReadOnly.into()),
            _ => Ok(()),
        }
    }

    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    /// Follows multi-extent records; a call reads from one extent at most.
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        self.view.read(&mut self.dev, node, offset, buf).await
    }
}
}
