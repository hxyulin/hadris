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
use crate::rock_ridge::{LinkKey, RockRidgeInfo, Scan};
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
/// A malformed Rock Ridge area on the root is read as no Rock Ridge, and
/// its error is returned beside the info.
async fn read_info<D: BlockDevice>(dev: &mut D) -> Result<(Info, Option<Error<D::Error>>), Error<D::Error>> {
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
    match probe.detect_rock_ridge(dev).await? {
        Ok(rock_ridge) => {
            info.rock_ridge = rock_ridge;
            Ok((info, None))
        }
        Err(err) => Ok((info, Some(err))),
    }
}

/// A mounted ISO 9660 image, read through one of its directory trees.
///
/// It reads the volume descriptors once, when mounted. Reading without
/// an explicitly configured cache needs no allocator. [`mount`](Self::mount) reads the most capable tree the image
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
/// Primary and Joliet names omit the `;N` version suffix. Unqualified
/// lookup and listing select the highest version independently of record order;
/// lookup with an explicit suffix selects that version. Rock Ridge alternate
/// names keep literal semicolons. In the primary and enhanced trees a lookup
/// that finds no exact name retries ignoring ASCII case. A Rock Ridge name
/// longer than [`DirEntry::MAX_NAME`] bytes lists and looks up under the
/// record's ISO 9660 identifier instead.
///
/// ```rust,no_run
/// # #[cfg(all(feature = "sync", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_iso::sync::IsoFs;
/// use hadris_fs::sync::FileSystem;
/// use hadris_fs::{MountOptions, OpenMode, Resolve};
/// # let dev = hadris_storage::MemDevice::new(vec![0; 1024 * 1024], hadris_storage::BlockSize::new(512).unwrap());
/// let mut iso = IsoFs::mount(dev, MountOptions::new())?;
/// let node = iso.resolve(b"/boot/grub/grub.cfg", Resolve::Lexical)?;
/// iso.open(node, OpenMode::Read)?;
/// let mut buf = [0; 64];
/// let n = iso.read(node, 0, &mut buf)?;
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "std")))]
/// # fn main() {}
/// ```
#[derive(Debug)]
pub struct IsoFs<D> {
    dev: D,
    view: View,
}

/// The device-independent state of a mount.
#[derive(Debug)]
struct View {
    info: Info,
    namespace: Namespace,
    root: Root,
    len: u64,
    versions: Option<(Dir, bool)>,
    #[cfg(feature = "cache")]
    cache: Option<crate::cache::ReaderCache>,
}

/// A directory: where its records are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dir {
    start: u64,
    size: u32,
}

/// A record found while walking a directory.
#[derive(Clone, Copy)]
struct Found {
    offset: u64,
    record: DirectoryRecord,
}

struct RecordChain {
    current: Found,
    count: usize,
}

impl RecordChain {
    fn new(offset: u64, record: DirectoryRecord) -> Self {
        Self {
            current: Found { offset, record },
            count: 0,
        }
    }

    async fn next<D: BlockDevice>(&mut self, view: &mut View, dev: &mut D) -> Result<Option<Found>, Error<D::Error>> {
        if self.count != 0 {
            if !self.current.record.header().file_flags().contains(FileFlags::NOT_FINAL) {
                return Ok(None);
            }
            if self.count == MAX_EXTENTS {
                return Err(Detail::MultiExtent.corrupt());
            }
            let (offset, record) = view.following(dev, self.current.offset, &self.current.record).await?;
            check_continuation(&self.current.record, &record)?;
            self.current = Found { offset, record };
        }
        self.count += 1;
        Ok(Some(self.current))
    }
}

fn check_continuation<E>(first: &DirectoryRecord, next: &DirectoryRecord) -> Result<(), Error<E>> {
    if next.name() == first.name() {
        Ok(())
    } else {
        Err(Detail::MultiExtent.corrupt())
    }
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

/// A listed entry: its id and name length.
#[derive(Clone, Copy)]
struct Listed {
    len: usize,
    rr: Option<RockRidgeInfo>,
    version: Option<u16>,
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
            versions: None,
            #[cfg(feature = "cache")]
            cache: None,
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

    async fn metadata_sector<D: BlockDevice>(&mut self, dev: &mut D, offset: u64, out: &mut [u8]) -> Result<(), Error<D::Error>> {
        #[cfg(feature = "cache")]
        if let Some(bytes) = self.cache.as_mut().and_then(|cache|cache.blocks.get(offset)) {
            out.copy_from_slice(&bytes[..out.len()]);
            return Ok(());
        }
        read_bytes(dev, self.len, offset, out).await?;
        #[cfg(feature = "cache")]
        if let Some(cache) = &mut self.cache {
            let mut bytes = [0; SECTOR_SIZE];
            bytes[..out.len()].copy_from_slice(out);
            cache.blocks.insert(offset, bytes);
        }
        Ok(())
    }

    async fn record_at<D: BlockDevice>(&mut self, dev: &mut D, offset: u64) -> Result<DirectoryRecord, Error<D::Error>> {
        #[cfg(feature = "cache")]
        if let Some(record) = self.cache.as_mut().and_then(|cache|cache.records.get(offset)) {
            return Ok(record);
        }
        let bs = self.bs();
        if offset == 0 {
            return Err(Detail::DirectoryRecord.corrupt());
        }
        let mut block = [0u8; SECTOR_SIZE];
        let block = &mut block[..bs as usize];
        self.metadata_sector(dev, offset - offset % bs, block).await?;
        let within = (offset % bs) as usize;
        match DirectoryRecord::parse(&block[within..]) {
            Ok(Some(record)) => {
                #[cfg(feature = "cache")]
                if let Some(cache) = &mut self.cache { cache.records.insert(offset, record); }
                Ok(record)
            },
            _ => Err(Detail::DirectoryRecord.corrupt()),
        }
    }

    /// The directory a node names: its `.` record, or a directory record.
    async fn dir_of<D: BlockDevice>(&mut self, dev: &mut D, node: NodeId) -> FsResult<Dir, D::Error> {
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
        &mut self,
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
                self.metadata_sector(dev, dir.start + index * u64::from(bs), data).await?;
                block.index = index;
            }
            let within = (*pos % bs) as usize;
            let take = (bs as usize - within).min((dir.size - *pos) as usize);
            let data = &block.data[within..within + take];
            match DirectoryRecord::parse(data) {
                Ok(Some(record)) => {
                    let offset = dir.start + u64::from(*pos);
                    *pos = pos
                        .checked_add(record.len() as u32)
                        .ok_or(Detail::DirectoryRecord.corrupt())?;
                    return Ok(Some(Found { offset, record }));
                }
                Ok(None) => *pos = (*pos / bs + 1).saturating_mul(bs),
                Err(()) => return Err(Detail::DirectoryRecord.corrupt()),
            }
        }
        Ok(None)
    }

    /// Skips the continuation records of a multi-extent file whose first
    /// record `first` was just read.
    async fn skip_continuations<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        dir: Dir,
        pos: &mut u32,
        block: &mut Block,
        first: &DirectoryRecord,
    ) -> Result<(), Error<D::Error>> {
        let mut last = *first;
        let mut count = 1;
        while last.header().file_flags().contains(FileFlags::NOT_FINAL) {
            if count == MAX_EXTENTS {
                return Err(Detail::MultiExtent.corrupt());
            }
            let next = self
                .next_record(dev, dir, pos, block)
                .await?
                .ok_or(Detail::MultiExtent.corrupt())?;
            check_continuation(first, &next.record)?;
            last = next.record;
            count += 1;
        }
        Ok(())
    }

    /// Follows the system use area of `record` through its continuation
    /// areas into `scan`.
    async fn scan<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        record: &DirectoryRecord,
        skip: u8,
        scan: &mut Scan<'_>,
    ) -> Result<(), Error<D::Error>> {
        scan.next = None;
        scan.feed(record.system_use(), usize::from(skip));
        let mut area = [0u8; SECTOR_SIZE];
        for count in 0..=MAX_CONTINUATIONS {
            if scan.malformed {
                return Err(Detail::SystemUse.corrupt());
            }
            let Some(ce) = scan.next.take() else {
                return Ok(());
            };
            if count == MAX_CONTINUATIONS {
                return Err(Detail::SystemUse.corrupt());
            }
            let len = u64::from(ce.length.get());
            let offset = u64::from(ce.block.get()) * self.bs() + u64::from(ce.offset.get());
            if len == 0 || u64::from(ce.offset.get()) >= self.bs() || offset.checked_add(len).is_none_or(|end| end > self.len) {
                return Err(Detail::SystemUse.corrupt());
            }
            let mut left = len;
            let mut at = offset;
            let mut held = 0;
            while left > 0 {
                let take = left.min((area.len() - held) as u64) as usize;
                read_bytes(dev, self.len, at, &mut area[held..held + take]).await?;
                at += take as u64;
                left -= take as u64;
                let used = held + take;
                let mut end = 0;
                while used - end >= 4 {
                    let entry_len = usize::from(area[end + 2]);
                    if area[end] == 0 || entry_len < 4 {
                        end = used;
                        break;
                    }
                    if entry_len > used - end {
                        break;
                    }
                    end += entry_len;
                }
                if left == 0 {
                    end = used;
                }
                if scan.feed(&area[..end], 0) {
                    break;
                }
                if scan.malformed {
                    break;
                }
                held = used - end;
                if held >= 255 {
                    return Err(Detail::SystemUse.corrupt());
                }
                area.copy_within(end..used, 0);
            }
        }
        Err(Detail::SystemUse.corrupt())
    }

    /// Whether the root's `.` record starts a Rock Ridge area. The inner
    /// error is a malformed area; device errors and an unreadable root
    /// record are the outer one.
    async fn detect_rock_ridge<D: BlockDevice>(
        &mut self,
        dev: &mut D,
    ) -> Result<Result<Option<u8>, Error<D::Error>>, Error<D::Error>> {
        let dot = self.record_at(dev, self.root_id().get()).await?;
        if !dot.system_use().starts_with(b"SP\x07\x01\xbe\xef") {
            return Ok(Ok(None));
        }
        let mut scan = Scan::new();
        match self.scan(dev, &dot, 0, &mut scan).await {
            Err(err) if err.kind() == ErrorKind::Io => return Err(err),
            Err(err) => return Ok(Err(err)),
            Ok(_) => {}
        }
        Ok(Ok(match scan.sp {
            Some(skip) if scan.rrip || scan.info.mode().is_some() => Some(skip),
            _ => None,
        }))
    }

    /// The id of the non-directory record at `offset`. A Rock Ridge file
    /// with more than one link gets the id of the first record of the same
    /// file in path table order, so all its names share one id: records
    /// with its `PX` serial number, or without one, at its data extent.
    async fn link_id<D: BlockDevice>(
        &mut self,
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

    async fn first_link<D: BlockDevice>(&mut self, dev: &mut D, key: LinkKey, skip: u8) -> Result<Option<u64>, Error<D::Error>> {
        #[cfg(feature = "cache")]
        {
            let build = self.cache.as_ref().is_some_and(|cache|cache.link_capacity>0 && !cache.links_attempted);
            if build {
                if let Some(cache) = &mut self.cache { cache.links_attempted=true; }
                // An optional index must not fail a lookup on an unrelated record.
                let _ = self.scan_links(dev, None, skip).await;
            }
            if let Some(offset) = self.cache.as_ref().and_then(|cache|cache.links.get(&key)).copied() {
                return Ok(Some(offset));
            }
        }
        self.scan_links(dev, Some(key), skip).await
    }

    /// The first record in path table order of the hard-linked file `key`
    /// names.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
    async fn scan_links<D: BlockDevice>(&mut self, dev: &mut D, key: Option<LinkKey>, skip: u8) -> Result<Option<u64>, Error<D::Error>> {
        #[cfg(feature = "cache")]
        let mut index = alloc::collections::BTreeMap::new();
        #[cfg(feature = "cache")]
        let limit = self.cache.as_ref().map_or(0, |cache|cache.link_capacity);
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
                #[cfg(feature = "cache")]
                if key.is_none() {
                    let serial = info.serial().filter(|&serial|serial != 0).map(LinkKey::Serial);
                    let extent = if record.header().data_len.get()>0 { self.extent_start(&record).map(LinkKey::Extent) } else { None };
                    for candidate in [serial, extent].into_iter().flatten() {
                        if index.len() < limit { index.entry(candidate).or_insert(found.offset); }
                    }
                    if index.len() == limit {
                        if let Some(cache) = &mut self.cache { cache.links = index; }
                        return Ok(None);
                    }
                    continue;
                }
                let same = match key {
                    Some(LinkKey::Serial(serial)) => info.serial() == Some(serial),
                    Some(LinkKey::Extent(extent)) => {
                        record.header().data_len.get() > 0 && self.extent_start(&record) == Some(extent)
                    }
                    None => false,
                };
                if same {
                    return Ok(Some(found.offset));
                }
            }
        }
        #[cfg(feature = "cache")]
        if key.is_none() {
            if let Some(cache) = &mut self.cache { cache.links = index; }
        }
        Ok(None)
    }

    /// The displayed name and metadata, or `None` for records
    /// the listing hides.
    async fn list<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        found: &Found,
        out: &mut [u8],
    ) -> Result<Option<Listed>, Error<D::Error>> {
        let record = &found.record;
        let header = record.header();
        if record.is_dot() || header.file_flags().contains(FileFlags::ASSOCIATED_FILE) {
            return Ok(None);
        }
        if let Some(skip) = self.rock_ridge() {
            let mut name = [0u8; DirEntry::MAX_NAME];
            let mut scan = Scan::new().with_name(&mut name);
            self.scan(dev, record, skip, &mut scan).await?;
            if scan.info.is_relocated() {
                return Ok(None);
            }
            if header.is_directory() && !scan.info.is_logical_directory() && self.relocation_only(dev, record).await? {
                return Ok(None);
            }
            let (name, version) = match scan.name() {
                Ok(Some(bytes)) => (bytes, None),
                _ => (crate::name::strip_version(record.name()), crate::name::version(record.name(), false)),
            };
            let len = crate::name::sanitize(name, out)
            .ok_or(Error::from(ErrorKind::NameTooLong))?;
            return Ok(Some(Listed { len, rr: Some(scan.info), version }));
        }
        let len = match self.namespace {
            Namespace::Joliet => crate::name::decode_ucs2(record.name(), out),
            _ => crate::name::sanitize(crate::name::strip_version(record.name()), out),
        }
        .ok_or(Error::from(ErrorKind::NameTooLong))?;
        Ok(Some(Listed { len, rr: None, version: crate::name::version(record.name(), self.namespace == Namespace::Joliet) }))
    }

    async fn listed_id<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        found: &Found,
        rr: Option<RockRidgeInfo>,
    ) -> Result<NodeId, Error<D::Error>> {
        let record = &found.record;
        let is_dir = record.header().is_directory();
        let end = self.len.max(u64::from(self.info.volume_blocks) * self.bs());
        let dir_id = |start: Option<u64>| match start {
            Some(start) if start != 0 && start < end => NodeId::new(start).ok_or(Detail::DirectoryRecord.corrupt()),
            Some(start) if start != 0 => Err(Detail::OutsideImage.corrupt()),
            _ => Err(Detail::DirectoryRecord.corrupt()),
        };
        if let Some(info) = rr {
            if let Some(block) = info.child_link() {
                return dir_id(u64::from(block).checked_mul(self.bs()));
            }
            if !is_dir {
                let offset = self.link_id(dev, found.offset, record, &info, self.rock_ridge().unwrap_or(0)).await?;
                return NodeId::new(offset).ok_or(Detail::DirectoryRecord.corrupt());
            }
        }
        if is_dir {
            dir_id(self.extent_start(record))
        } else {
            NodeId::new(found.offset).ok_or(Detail::DirectoryRecord.corrupt())
        }
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
    async fn label<'b, D: BlockDevice>(&mut self, dev: &mut D, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
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

    async fn relocation_only<D: BlockDevice>(&mut self, dev: &mut D, record: &DirectoryRecord) -> Result<bool, Error<D::Error>> {
        let Some(skip) = self.rock_ridge() else { return Ok(false); };
        let dir = Dir {
            start: self.extent_start(record).ok_or(Detail::DirectoryRecord.corrupt())?,
            size: record.header().data_len.get(),
        };
        let mut pos = 0;
        let mut block = Block::new();
        let mut relocated = false;
        while let Some(found) = self.next_record(dev, dir, &mut pos, &mut block).await? {
            if found.record.is_dot() { continue; }
            let mut scan = Scan::new();
            self.scan(dev, &found.record, skip, &mut scan).await?;
            if !scan.info.is_relocated() { return Ok(false); }
            relocated = true;
        }
        Ok(relocated)
    }

    async fn has_versions<D: BlockDevice>(&mut self, dev: &mut D, dir: Dir) -> Result<bool, Error<D::Error>> {
        if let Some((cached, found)) = self.versions {
            if cached == dir { return Ok(found); }
        }
        let mut pos = 0;
        let mut block = Block::new();
        let mut multiple = false;
        while let Some(found) = self.next_record(dev, dir, &mut pos, &mut block).await? {
            if crate::name::version(found.record.name(), self.namespace == Namespace::Joliet).is_some_and(|version| version > 1) {
                multiple = true;
                break;
            }
        }
        self.versions = Some((dir, multiple));
        Ok(multiple)
    }

    async fn find_name<D: BlockDevice>(&mut self, dev: &mut D, dir: Dir, name: &Name, explicit: bool) -> FsResult<Option<(Found, Listed)>, D::Error> {
        let requested = if explicit { crate::name::version(name.as_bytes(), false) } else { None };
        let multiple = requested.is_none() && self.versions.filter(|(cached, _)| *cached == dir).is_none_or(|(_, found)| found);
        let mut versions = false;
        let mut pos = 0;
        let mut block = Block::new();
        let mut buf = [0u8; 1024];
        let mut best: Option<(Found, Listed, bool)> = None;
        while let Some(found) = self.next_record(dev, dir, &mut pos, &mut block).await? {
            versions |= crate::name::version(found.record.name(), self.namespace == Namespace::Joliet).is_some_and(|version| version > 1);
            let listed = self.list(dev, &found, &mut buf).await?;
            self.skip_continuations(dev, dir, &mut pos, &mut block, &found.record).await?;
            let Some(listed) = listed else { continue; };
            let query = if listed.version.is_some() {
                if requested.is_some() && requested != listed.version { continue; }
                if requested.is_some() { crate::name::strip_version(name.as_bytes()) } else { name.as_bytes() }
            } else { name.as_bytes() };
            let bytes = &buf[..listed.len];
            let exact = bytes == query;
            if !(exact || self.case_insensitive() && bytes.eq_ignore_ascii_case(query)) { continue; }
            if exact && (!multiple || listed.version.is_none()) { return Ok(Some((found, listed))); }
            if best.as_ref().is_none_or(|(_, prior, prior_exact)| (exact, listed.version.unwrap_or(0)) > (*prior_exact, prior.version.unwrap_or(0))) {
                best = Some((found, listed, exact));
            }
        }
        self.versions = Some((dir, versions));
        Ok(best.map(|(found, listed, _)| (found, listed)))
    }

    async fn lookup<D: BlockDevice>(&mut self, dev: &mut D, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        let dir = self.dir_of(dev, dir).await?;
        match self.find_name(dev, dir, name, true).await? {
            Some((found, listed)) => self.listed_id(dev, &found, listed.rr).await,
            None => Err(ErrorKind::NotFound.into()),
        }
    }

    async fn readdir<D: BlockDevice>(&mut self, dev: &mut D, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
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
                if listed.version.is_some() && self.has_versions(dev, dir).await? {
                    let latest = self.find_name(dev, dir, Name::new(&out[..listed.len]), false).await?;
                    if latest.is_some_and(|(latest, _)| latest.offset != found.offset) { continue; }
                }
                let node = self.listed_id(dev, &found, listed.rr).await?;
                let meta = if node.get() == found.offset {
                    self.record_metadata(dev, node, &found.record, listed.rr).await?
                } else {
                    self.stat(dev, node).await?
                };
                let next = DirCursor::from_raw(u64::from(pos));
                return Ok(Some(DirEntry::new(Name::new(&out[..listed.len]), node, meta, next)?));
            }
        }
        Ok(None)
    }

    async fn stat<D: BlockDevice>(&mut self, dev: &mut D, node: NodeId) -> FsResult<Metadata, D::Error> {
        let record = self.record_at(dev, node.get()).await.map_err(|err| handle(err, node, self))?;
        let rr = match self.rock_ridge() {
            Some(skip) => {
                let mut scan = Scan::new();
                self.scan(dev, &record, skip, &mut scan).await?;
                Some(scan.info)
            }
            None => None,
        };
        self.record_metadata(dev, node, &record, rr).await
    }

    async fn record_metadata<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        node: NodeId,
        record: &DirectoryRecord,
        rr: Option<RockRidgeInfo>,
    ) -> FsResult<Metadata, D::Error> {
        let header = *record.header();
        let (mut file_type, mut len) = if header.is_directory() {
            (FileType::Dir, 0)
        } else {
            let file_type = rr.and_then(|rr| rr.file_type()).unwrap_or(FileType::File);
            (file_type, self.file_len(dev, node.get(), record).await?)
        };
        let mut times = [None, header.date_time.to_datetime(), None, None];
        let mut permissions = Permissions::new(if file_type.is_dir() { 0o555 } else { 0o444 });
        let mut owner = None;
        let mut nlink = 1;
        let mut device = None;
        if let Some(rr) = rr {
            times = [rr.created(), rr.modified().or(times[1]), rr.accessed(), rr.changed()];
            if let Some(mode) = rr.mode() {
                permissions = Permissions::new(mode);
            }
            owner = rr.owner().map(|(uid, gid)| Owner::new(uid, gid));
            nlink = u64::from(rr.links().unwrap_or(1));
            device = rr.device();
            if rr.is_symlink() {
                let mut target = [0u8; 4096];
                let mut scan = Scan::new().with_link(&mut target);
                self.scan(dev, record, self.rock_ridge().unwrap_or(0), &mut scan).await?;
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
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
    async fn file_len<D: BlockDevice>(&mut self, dev: &mut D, offset: u64, record: &DirectoryRecord) -> Result<u64, Error<D::Error>> {
        let mut total = 0;
        let mut chain = RecordChain::new(offset, *record);
        while let Some(found) = chain.next(self, dev).await? {
            total += u64::from(found.record.header().data_len.get());
        }
        Ok(total)
    }

    /// The record after the one at `offset`, in the same block or at the
    /// start of the next.
    async fn following<D: BlockDevice>(&mut self, dev: &mut D, offset: u64, record: &DirectoryRecord) -> Result<(u64, DirectoryRecord), Error<D::Error>> {
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

    async fn read<D: BlockDevice>(&mut self, dev: &mut D, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let record = self.record_at(dev, node.get()).await.map_err(|err| handle(err, node, self))?;
        if record.header().is_directory() {
            return Err(ErrorKind::IsADirectory.into());
        }
        let mut chain = RecordChain::new(node.get(), record);
        let mut start = 0u64;
        while let Some(found) = chain.next(self, dev).await? {
            let header = *found.record.header();
            if header.file_unit_size != 0 || header.interleave_gap_size != 0 || header.volume_sequence_number.get() > 1 {
                return Err(Detail::Interleaved.error(ErrorKind::Unsupported));
            }
            let len = u64::from(header.data_len.get());
            if offset < start + len {
                let within = offset - start;
                let take = usize::try_from(len - within).unwrap_or(usize::MAX).min(buf.len());
                let base = self.extent_start(&found.record).ok_or(ErrorKind::Corrupt)?;
                read_bytes(dev, self.len, base + within, &mut buf[..take]).await?;
                return Ok(take);
            }
            start += len;
        }
        Ok(0)
    }

    async fn parent<D: BlockDevice>(&mut self, dev: &mut D, dir: NodeId) -> FsResult<NodeId, D::Error> {
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
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
    async fn path_table_parent<D: BlockDevice>(&mut self, dev: &mut D, start: u64) -> Result<u64, Error<D::Error>> {
        let parent = self.path_table_find(dev, PathTableQuery::Extent(start / self.bs())).await?;
        Ok(self.path_table_find(dev, PathTableQuery::Number(parent)).await? * self.bs())
    }

    /// Scans the little-endian path table for a directory's extent, giving
    /// its parent's number, or for a record number, giving its extent.
    async fn path_table_find<D: BlockDevice>(&mut self, dev: &mut D, want: PathTableQuery) -> Result<u64, Error<D::Error>> {
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

    async fn readlink<D: BlockDevice>(&mut self, dev: &mut D, link: NodeId, buf: &mut [u8]) -> FsResult<usize, D::Error> {
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

    async fn rock_ridge_info<D: BlockDevice>(&mut self, dev: &mut D, node: NodeId) -> Result<Option<RockRidgeInfo>, Error<D::Error>> {
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
    /// device blocks above 4096 bytes. A malformed Rock Ridge area on the
    /// root directory is read as no Rock Ridge, so the next tree mounts.
    /// The [`MountError`] gives `dev` back.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
    pub async fn mount(dev: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_namespace(dev, options, Namespace::Preferred).await
    }

    /// Mounts the image on `dev` as [`mount`](Self::mount) does, reading
    /// the tree `namespace` names. Fails with [`ErrorKind::NotFound`] and
    /// [`Detail::NoNamespace`] when the image has no such tree, and for
    /// [`Namespace::RockRidge`] with the error of a malformed Rock Ridge
    /// area on the root directory.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
    pub async fn mount_namespace(
        mut dev: D,
        options: MountOptions,
        namespace: Namespace,
    ) -> Result<Self, MountError<D, D::Error>> {
        let _ = options;
        let info = match read_info(&mut dev).await {
            Ok((_, Some(err))) if namespace == Namespace::RockRidge => {
                return Err(MountError::new(err, dev));
            }
            Ok((info, _)) => info,
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

    /// Enables bounded caches for metadata sectors and parsed directory records.
    ///
    /// Payload reads remain direct. This allocates storage for the configured
    /// capacities; mounting without this method still uses no allocator.
    /// The image must remain unchanged while cached data is retained.
    #[cfg(feature = "cache")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
    pub fn with_cache(mut self, options: crate::CacheOptions) -> Self {
        self.view.cache = Some(crate::cache::ReaderCache::new(options));
        self
    }

    /// Discards cached metadata while retaining the configured storage.
    #[cfg(feature = "cache")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
    pub fn clear_cache(&mut self) {
        self.view.versions = None;
        if let Some(cache) = &mut self.view.cache { cache.clear(); }
    }

    /// Gives the device back. The image is read-only, so there is nothing
    /// to sync and this never fails.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(offset = offset, bytes = buf.len())))]
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

    /// Maps a file to the device: fills `out` with its extents that end
    /// after file offset `from`, in order, and returns how many it filled.
    /// Call again from the end of the last one for more; 0 means there are
    /// none. A file of several extents (multi-extent) has one per
    /// directory record; a directory has the one extent of its records.
    /// Each call reads the records from the first, so a larger `out`
    /// takes fewer reads.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node)))]
    pub async fn extents(&mut self, node: NodeId, from: u64, out: &mut [hadris_fs::Extent]) -> Result<usize, Error<D::Error>> {
        let mut count = 0;
        self.walk_extents(node, &mut |extent| {
            if extent.file_offset() + extent.len() <= from || extent.is_empty() {
                return true;
            }
            let Some(slot) = out.get_mut(count) else {
                return false;
            };
            *slot = extent;
            count += 1;
            true
        })
        .await?;
        Ok(count)
    }

    /// Calls `each` with every extent of a file in order, including empty
    /// ones, until it returns `false`.
    pub(crate) async fn walk_extents(
        &mut self,
        node: NodeId,
        each: &mut impl FnMut(hadris_fs::Extent) -> bool,
    ) -> Result<(), Error<D::Error>> {
        let record = self.view.record_at(&mut self.dev, node.get()).await?;
        let mut chain = RecordChain::new(node.get(), record);
        let mut file = 0u64;
        while let Some(found) = chain.next(&mut self.view, &mut self.dev).await? {
            let start = self.view.extent_start(&found.record).ok_or(Detail::DirectoryRecord.corrupt())?;
            let len = u64::from(found.record.header().data_len.get());
            if !each(hadris_fs::Extent::new(start, len).with_file_offset(file)) {
                return Ok(());
            }
            file += len;
        }
        Ok(())
    }

    /// Locates a node's directory records: the record its id names (a
    /// directory's `.` record) and, for a file of several extents, the
    /// records that follow it. Read them with [`read_raw`](Self::read_raw).
    /// Returns how many it filled; fails with [`ErrorKind::LimitExceeded`]
    /// when `out` is too short, and [`ErrorKind::Corrupt`] when continuation
    /// records have different file identifiers.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node)))]
    pub async fn records(&mut self, node: NodeId, out: &mut [hadris_fs::Extent]) -> Result<usize, Error<D::Error>> {
        let record = self.view.record_at(&mut self.dev, node.get()).await?;
        let mut chain = RecordChain::new(node.get(), record);
        let mut count = 0;
        while let Some(found) = chain.next(&mut self.view, &mut self.dev).await? {
            let len = u64::from(found.record.header().len);
            *out.get_mut(count).ok_or(ErrorKind::LimitExceeded)? = hadris_fs::Extent::new(found.offset, len);
            count += 1;
        }
        Ok(count)
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
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all))]
    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        Ok(FsStats::new(u64::from(self.view.info.volume_blocks), 0, self.view.info.block_size))
    }

    /// The volume identifier of the descriptor this mount reads: UCS-2 in
    /// the Joliet tree, bytes read as Latin-1 otherwise.
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        self.view.label(&mut self.dev, buf).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(dir = ?dir)))]
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        self.view.lookup(&mut self.dev, dir, name).await
    }

    /// Does nothing: ISO node ids are stable.
    fn forget(&mut self, node: NodeId, count: u64) {
        let _ = (node, count);
    }

    /// The directory containing `dir`, from its `..` record, or its Rock
    /// Ridge `PL` entry when it was relocated.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(dir = ?dir)))]
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        self.view.parent(&mut self.dev, dir).await
    }

    /// Rock Ridge mode, owner, links and times when the mount reads Rock
    /// Ridge, the record's time as the modification time otherwise.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node)))]
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        self.view.stat(&mut self.dev, node).await
    }

    /// The cursor is the byte offset of the next record in the directory.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(dir = ?dir)))]
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        self.view.readdir(&mut self.dev, dir, from).await
    }

    /// [`ErrorKind::InvalidInput`] for other nodes and outside the Rock
    /// Ridge tree.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node, bytes = buf.len())))]
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let len = self.view.readlink(&mut self.dev, node, buf).await?;
        Ok(&buf[..len])
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node)))]
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        let meta = self.view.stat(&mut self.dev, node).await?;
        match meta.file_type() {
            FileType::Dir => Err(ErrorKind::IsADirectory.into()),
            FileType::Symlink => Err(ErrorKind::Symlink.into()),
            _ if mode == OpenMode::Write => Err(ErrorKind::ReadOnly.into()),
            _ => Ok(()),
        }
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node)))]
    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    /// Follows multi-extent records; a call reads from one extent at most.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::iso", level = "trace", skip_all, fields(node = ?node, offset = offset, bytes = buf.len())))]
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        self.view.read(&mut self.dev, node, offset, buf).await
    }
}
}
