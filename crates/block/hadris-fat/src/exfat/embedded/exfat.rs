use core::ops::ControlFlow;

use hadris_fat_raw::exfat::io::{DirWalk, ExFat as Volume, Extent};
use hadris_fat_raw::exfat::{self as raw, Geometry, RawEntry};
use hadris_fat_raw::io::{BlockBuf, ChainPos};
use hadris_fat_raw::name::{eq_folded, utf16_to_utf8};
use hadris_fs::{
    DirCursor, ErrorKind, FsResult, FsStats, Metadata, MountError, OpenOptions, SeekFrom,
};

use super::storage::BlockDevice;
use super::{exio, rawio};
use crate::embedded::{BLOCK, MountToken};
use crate::exfat::embedded::{
    Dir, Entry, File, FileSlot, Head, Node, Options, le16, le32, le64, metadata,
};

/// An empty block buffer of the embedded block size.
const EMPTY_BLOCK: BlockBuf<[u8; BLOCK]> = match BlockBuf::new(BLOCK) {
    Some(block) => block,
    None => panic!("BLOCK fits"),
};

/// An entry set read from a directory: its head, slot and name.
struct Named {
    head: Head,
    slot: u32,
    name: [u16; raw::MAX_NAME_UNITS],
    len: usize,
}

impl Named {
    const fn new() -> Self {
        Self {
            head: Head {
                primary: [0; raw::ENTRY_SIZE],
                stream: [0; raw::ENTRY_SIZE],
            },
            slot: 0,
            name: [0; raw::MAX_NAME_UNITS],
            len: 0,
        }
    }

    fn name(&self) -> &[u16] {
        &self.name[..self.len]
    }
}

/// A name of one path component: not empty, and without `/` or NUL.
fn check_name(name: &str) -> Result<(), ErrorKind> {
    if name.is_empty() || name.contains(['/', '\0']) {
        return Err(ErrorKind::InvalidInput);
    }
    Ok(())
}

io_transform! {

/// A read-only exFAT volume for firmware, with `FILES` file slots.
///
/// See the [module docs](crate::exfat::embedded) for the model. An `ExFat`
/// holds the device, one 512-byte block buffer, the geometry, the
/// [`Options`] and the file slots, needs no allocator and never writes to
/// the device.
pub struct ExFat<'mount, D, const FILES: usize = 4> {
    owner: &'mount MountToken,
    dev: D,
    geo: Geometry,
    block: BlockBuf<[u8; BLOCK]>,
    options: Options,
    files: [FileSlot; FILES],
    next_generation: u16,
}

impl<D, const FILES: usize> core::fmt::Debug for ExFat<'_, D, FILES> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExFat")
            .field("cluster_size", &self.geo.cluster_size())
            .field("cluster_count", &self.geo.cluster_count())
            .finish_non_exhaustive()
    }
}

impl<'mount, D: BlockDevice, const FILES: usize> ExFat<'mount, D, FILES> {
    /// Mounts the volume on `dev` with [`Options::new`], reserving `owner`
    /// until the volume and all its files are no longer used.
    pub async fn mount(dev: D, owner: &'mount mut MountToken) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_with(dev, owner, Options::new()).await
    }

    /// Mounts the volume on `dev`, from the backup boot region when the
    /// main one is damaged. Only the fold and UTC offset of `options`
    /// apply.
    ///
    /// Fails with [`ErrorKind::Unsupported`] when the device's blocks are
    /// not 512 bytes, with [`ErrorKind::NotRecognized`] when the first
    /// sector does not name exFAT, and with [`ErrorKind::Corrupt`] when
    /// neither boot region is valid or the volume is larger than the
    /// device. The [`MountError`] gives `dev` back.
    pub async fn mount_with(mut dev: D, owner: &'mount mut MountToken, options: Options) -> Result<Self, MountError<D, D::Error>> {
        const { assert!(FILES <= u8::MAX as usize, "at most 255 file slots") };
        if dev.block_size().get() as usize != BLOCK {
            return Err(MountError::new(ErrorKind::Unsupported.into(), dev));
        }
        let mut block = EMPTY_BLOCK;
        let geo = match exio::read_boot(&mut dev, &mut block).await {
            Ok((geo, _)) => geo,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        Ok(Self {
            owner,
            dev,
            geo,
            block,
            options,
            files: [FileSlot::FREE; FILES],
            next_generation: 0,
        })
    }

    /// Gives the device back. The volume was never written, so there is
    /// nothing to sync.
    pub fn unmount(self) -> D {
        self.dev
    }

    /// The geometry from the boot sector.
    pub fn info(&self) -> &Geometry {
        &self.geo
    }

    /// The options the volume was mounted with.
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Whether `VolumeDirty` is set: the volume was not cleanly unmounted.
    pub fn was_dirty(&self) -> bool {
        self.geo.flags() & raw::VOLUME_DIRTY != 0
    }

    /// The root directory.
    pub fn root(&self) -> Dir {
        Dir::ROOT
    }

    /// Opens the subdirectory `name` of `parent`; `.` is `parent`. exFAT
    /// directories do not record their parent, so `..` fails with
    /// [`ErrorKind::Unsupported`]. Fails with [`ErrorKind::NotFound`] or
    /// [`ErrorKind::NotADirectory`].
    pub async fn open_dir(&mut self, parent: Dir, name: &str) -> FsResult<Dir, D::Error> {
        match name {
            "." => return Ok(parent),
            ".." => return Err(ErrorKind::Unsupported.into()),
            _ => {}
        }
        let mut found = Named::new();
        self.lookup(parent, name, &mut found).await?;
        if !found.head.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        if !self.geo.is_cluster(found.head.first()) {
            return Err(ErrorKind::Corrupt.into());
        }
        Ok(found.head.as_dir())
    }

    /// Lists `dir` from `from` (`DirCursor::START` for the beginning),
    /// calling `each` with every file and directory until it returns
    /// `ControlFlow::Break`. Continue a listing from the
    /// [`next_cursor`](Entry::next_cursor) of the last entry seen.
    pub async fn list(
        &mut self,
        dir: Dir,
        from: DirCursor,
        mut each: impl FnMut(&Entry<'_>) -> ControlFlow<()>,
    ) -> FsResult<(), D::Error> {
        let Ok(mut slot) = u32::try_from(from.into_raw()) else {
            return Ok(());
        };
        let zone = self.options.utc_offset();
        let mut walk = DirWalk::new(dir.extent(self.geo.root()));
        let mut named = Named::new();
        while self.next_set(&mut walk, &mut slot, &mut named).await? {
            let entry = Entry::new(named.name(), named.head, Node::new(dir, named.slot), zone);
            if each(&entry).is_break() {
                break;
            }
        }
        Ok(())
    }

    /// Opens the file `name` in `dir` for reading in a free slot. Options
    /// that write fail with [`ErrorKind::ReadOnly`]. Fails with
    /// [`ErrorKind::LimitExceeded`] when every slot is taken and
    /// [`ErrorKind::IsADirectory`] for a directory.
    pub async fn open(&mut self, dir: Dir, name: &str, options: OpenOptions) -> FsResult<File<'mount>, D::Error> {
        let index = self.check_open(options)?;
        let mut found = Named::new();
        self.lookup(dir, name, &mut found).await?;
        self.open_head(index, found.head)
    }

    /// Opens the file a listed [`Entry`] names, without a lookup. The
    /// position is valid until its directory changes; one that no longer
    /// holds a file fails with [`ErrorKind::NotFound`].
    pub async fn open_node(&mut self, node: Node, options: OpenOptions) -> FsResult<File<'mount>, D::Error> {
        let index = self.check_open(options)?;
        let mut walk = DirWalk::new(node.dir().extent(self.geo.root()));
        let mut slot = node.slot();
        let mut found = Named::new();
        if !self.next_set(&mut walk, &mut slot, &mut found).await? || found.slot != node.slot() {
            return Err(ErrorKind::NotFound.into());
        }
        self.open_head(index, found.head)
    }

    /// Reads from the file's position and advances it. Returns 0 at the
    /// end. Bytes past `ValidDataLength` read as zeros.
    pub async fn read(&mut self, file: &File<'_>, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let index = self.slot(file)?;
        let state = self.files[index];
        let Some(head) = state.head else {
            return Err(ErrorKind::InvalidHandle.into());
        };
        let (pos, len, valid) = (state.pos, head.len(), head.valid());
        if pos >= len || buf.is_empty() {
            return Ok(0);
        }
        let count = (len - pos).min(buf.len() as u64) as usize;
        let cluster_size = self.geo.cluster_size();
        let mut at = state.at;
        let mut done = 0;
        while done < count {
            let offset = pos + done as u64;
            if offset >= valid {
                buf[done..count].fill(0);
                break;
            }
            let want = u32::try_from(offset >> self.geo.cluster_shift()).map_err(|_| ErrorKind::Corrupt)?;
            at = self.locate(&head, at, want).await?;
            let within = offset & (cluster_size - 1);
            let n = (cluster_size - within).min((count - done) as u64).min(valid - offset) as usize;
            let disk = self.geo.cluster_offset(at.cluster()).ok_or(ErrorKind::Corrupt)? + within;
            rawio::read_bytes(&mut self.dev, &mut self.block, disk, &mut buf[done..done + n]).await?;
            done += n;
        }
        let slot = &mut self.files[index];
        slot.pos = pos + count as u64;
        slot.at = at;
        Ok(count)
    }

    /// Moves the file's position and returns it. A position before the
    /// start fails with [`ErrorKind::InvalidInput`].
    pub fn seek(&mut self, file: &File<'_>, pos: SeekFrom) -> FsResult<u64, D::Error> {
        let index = self.slot(file)?;
        let state = self.files[index];
        let len = state.head.map_or(0, |head| head.len());
        let pos = pos.resolve(state.pos, len).ok_or(ErrorKind::InvalidInput)?;
        self.files[index].pos = pos;
        Ok(pos)
    }

    /// Closes the file and frees its slot.
    pub fn close(&mut self, file: File<'_>) -> FsResult<(), D::Error> {
        let index = self.slot(&file)?;
        self.files[index] = FileSlot::FREE;
        Ok(())
    }

    /// The metadata of `name` in `dir`.
    pub async fn metadata(&mut self, dir: Dir, name: &str) -> FsResult<Metadata, D::Error> {
        let mut found = Named::new();
        self.lookup(dir, name, &mut found).await?;
        Ok(metadata(&found.head, self.options.utc_offset()))
    }

    /// The volume label from the root directory, or `None` when it has
    /// none, as UTF-8 in `buf`. Fails with [`ErrorKind::LimitExceeded`]
    /// when `buf` is too short.
    pub async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let Some(entry) = self.system_entry(raw::ENTRY_LABEL).await? else {
            return Ok(None);
        };
        let count = (entry[1] as usize).min(raw::MAX_LABEL_UNITS);
        let units = (0..count).map(|at| le16(&entry, 2 + at * 2));
        let len = utf16_to_utf8(units, buf).ok_or(ErrorKind::LimitExceeded)?;
        Ok(Some(core::str::from_utf8(&buf[..len]).map_err(|_| ErrorKind::Corrupt)?))
    }

    /// Total and free clusters, counted from the active Allocation Bitmap.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        let bitmap = self.system_entry(raw::ENTRY_BITMAP).await?.ok_or(ErrorKind::Corrupt)?;
        let extent = Extent::chain(le32(&bitmap, 20), le64(&bitmap, 24));
        let mut vol = Volume::new(self.geo, extent, None);
        let free = exio::count_free(&mut self.dev, &mut self.block, &mut vol).await?;
        let cluster_size = u32::try_from(self.geo.cluster_size()).map_err(|_| ErrorKind::Corrupt)?;
        Ok(FsStats::new(self.geo.cluster_count() as u64, free as u64, cluster_size))
    }

    /// The slot `file` names, or [`ErrorKind::InvalidHandle`].
    fn slot(&self, file: &File<'_>) -> Result<usize, ErrorKind> {
        if !file.belongs_to(self.owner) {
            return Err(ErrorKind::InvalidHandle);
        }
        match self.files.get(file.slot()) {
            Some(slot) if slot.head.is_some() && slot.generation == file.generation() => Ok(file.slot()),
            _ => Err(ErrorKind::InvalidHandle),
        }
    }

    /// Checks that `options` only read and returns a free slot.
    fn check_open(&self, options: OpenOptions) -> Result<usize, ErrorKind> {
        options.validate().map_err(ErrorKind::from)?;
        if options.is_write() {
            return Err(ErrorKind::ReadOnly);
        }
        self.files.iter().position(|slot| slot.head.is_none()).ok_or(ErrorKind::LimitExceeded)
    }

    fn open_head(&mut self, index: usize, head: Head) -> FsResult<File<'mount>, D::Error> {
        if head.is_dir() {
            return Err(ErrorKind::IsADirectory.into());
        }
        let generation = self.next_generation;
        self.next_generation = generation.wrapping_add(1);
        self.files[index] = FileSlot {
            head: Some(head),
            pos: 0,
            at: ChainPos::NONE,
            generation,
        };
        Ok(File::new(self.owner, index as u8, generation))
    }


    /// The position of cluster `want` of a file, walked from `from` when it
    /// is not past `want`.
    async fn locate(&mut self, head: &Head, from: ChainPos, want: u32) -> FsResult<ChainPos, D::Error> {
        let first = head.first();
        if !self.geo.is_cluster(first) {
            return Err(ErrorKind::Corrupt.into());
        }
        if head.contiguous() {
            let cluster = first.checked_add(want).filter(|&cluster| self.geo.is_cluster(cluster));
            return Ok(ChainPos::new(want, cluster.ok_or(ErrorKind::Corrupt)?));
        }
        let mut at = if from.is_known() && from.index() <= want { from } else { ChainPos::start(first) };
        while at.index() < want {
            let next = exio::next(&mut self.dev, &mut self.block, &self.geo, at.cluster())
                .await?
                .ok_or(ErrorKind::Corrupt)?;
            if !at.advance(next) {
                return Err(ErrorKind::Corrupt.into());
            }
        }
        Ok(at)
    }

    /// Finds `name` in `dir` into `found`.
    async fn lookup(&mut self, dir: Dir, name: &str, found: &mut Named) -> FsResult<(), D::Error> {
        check_name(name)?;
        let fold = self.options.fold();
        let mut walk = DirWalk::new(dir.extent(self.geo.root()));
        let mut slot = 0;
        while self.next_set(&mut walk, &mut slot, found).await? {
            if eq_folded(name.encode_utf16(), found.name().iter().copied(), fold) {
                return Ok(());
            }
        }
        Err(ErrorKind::NotFound.into())
    }

    /// The first entry of type `kind` in the root directory; for an
    /// Allocation Bitmap, that of the active FAT.
    async fn system_entry(&mut self, kind: u8) -> FsResult<Option<RawEntry>, D::Error> {
        let mut walk = DirWalk::new(Dir::ROOT.extent(self.geo.root()));
        let mut slot = 0;
        while let Some(offset) = exio::slot_offset(&mut self.dev, &mut self.block, &self.geo, &mut walk, slot).await? {
            let mut entry = [0u8; raw::ENTRY_SIZE];
            rawio::read_bytes(&mut self.dev, &mut self.block, offset, &mut entry).await?;
            match entry[0] {
                raw::ENTRY_END => break,
                found if found == kind && (kind != raw::ENTRY_BITMAP || entry[1] & 1 == self.geo.active()) => {
                    return Ok(Some(entry));
                }
                _ => {}
            }
            slot += 1;
        }
        Ok(None)
    }

    /// Scans from `slot` to the next valid File entry set, reads it into
    /// `out`, and leaves `slot` after it.
    async fn next_set(&mut self, walk: &mut DirWalk, slot: &mut u32, out: &mut Named) -> FsResult<bool, D::Error> {
        while let Some(offset) = exio::slot_offset(&mut self.dev, &mut self.block, &self.geo, walk, *slot).await? {
            let at = *slot;
            let mut entry = [0u8; raw::ENTRY_SIZE];
            rawio::read_bytes(&mut self.dev, &mut self.block, offset, &mut entry).await?;
            if entry[0] == raw::ENTRY_END {
                return Ok(false);
            }
            if entry[0] == raw::ENTRY_FILE && self.read_set(walk, at, entry, out).await? {
                *slot = at + 1 + entry[1] as u32;
                return Ok(true);
            }
            *slot = at + 1;
        }
        Ok(false)
    }

    /// Reads the entry set whose File entry `primary` is directory slot
    /// `slot` into `out`. False when the entries are not a valid set: a
    /// Stream Extension, the File Name entries its name length needs,
    /// secondary entries in use, and a matching checksum.
    async fn read_set(&mut self, walk: &mut DirWalk, slot: u32, primary: RawEntry, out: &mut Named) -> FsResult<bool, D::Error> {
        let count = 1 + primary[1] as usize;
        if !(3..=raw::MAX_SET).contains(&count) {
            return Ok(false);
        }
        let mut sum = raw::set_checksum_step(0, 0, &primary);
        let mut names = 0;
        for index in 1..count {
            let Some(at) = exio::slot_offset(&mut self.dev, &mut self.block, &self.geo, walk, slot + index as u32).await? else {
                return Ok(false);
            };
            let mut entry = [0u8; raw::ENTRY_SIZE];
            rawio::read_bytes(&mut self.dev, &mut self.block, at, &mut entry).await?;
            sum = raw::set_checksum_step(sum, index, &entry);
            if index == 1 {
                if entry[0] != raw::ENTRY_STREAM || entry[3] == 0 {
                    return Ok(false);
                }
                names = (entry[3] as usize).div_ceil(raw::NAME_UNITS_PER_ENTRY);
                if 2 + names > count {
                    return Ok(false);
                }
                out.head.stream = entry;
                out.len = entry[3] as usize;
            } else if index < 2 + names {
                if entry[0] != raw::ENTRY_NAME {
                    return Ok(false);
                }
                let base = (index - 2) * raw::NAME_UNITS_PER_ENTRY;
                for unit in 0..raw::NAME_UNITS_PER_ENTRY {
                    if base + unit < out.len {
                        out.name[base + unit] = le16(&entry, 2 + unit * 2);
                    }
                }
            } else if entry[0] & raw::CATEGORY_SECONDARY == 0 || entry[0] & raw::IN_USE == 0 {
                return Ok(false);
            }
        }
        out.head.primary = primary;
        out.slot = slot;
        Ok(sum == le16(&primary, 2))
    }
}

}
