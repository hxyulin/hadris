use core::ops::ControlFlow;

use hadris_fat_raw::io::{BlockBuf, ChainPos, DirStart, DirWalk, Fat as Volume};
use hadris_fat_raw::lfn::Assembler;
use hadris_fat_raw::{self as raw, LongEntry, RootLocation, ShortEntry, Slot, date, short_name};
use hadris_fs::{
    DateTime, DirCursor, ErrorKind, FsResult, FsStats, Metadata, MountError, OpenOptions, SeekFrom,
    SetAttr,
};

use super::rawio;
use super::storage::BlockDevice;
use crate::embedded::{
    BLOCK, Dir, Entry, FLAG_APPEND, FLAG_DIRTY, FLAG_READ, FLAG_WRITE, File, FileSlot, MAX_DEPTH,
    MAX_FILE_SIZE, Node, Options, Owner, Pending, Run, generation_base, short_units,
};
use crate::names::{
    CANDIDATES, NewName, apply_attributes, is_exact, matches, read_only_bit, set_read_only, stamp,
};
use crate::{FatKind, Geometry};

const ENTRY_SIZE: u64 = raw::ENTRY_SIZE as u64;

/// An empty block buffer of the embedded block size.
const EMPTY_BLOCK: BlockBuf<[u8; BLOCK]> = match BlockBuf::new(BLOCK) {
    Some(block) => block,
    None => panic!("BLOCK fits"),
};

/// A visible short entry found by a directory scan.
struct Found {
    slot: u32,
    offset: u64,
    entry: ShortEntry,
    /// The slot of the last long-name fragment that started a name before
    /// this entry, meaningful only when the name assembled.
    long_start: u32,
}

/// An entry and the slots its name occupies.
struct Located {
    /// The first slot of the entry: its first long-name fragment when it
    /// has a valid long name.
    first: u32,
    slot: u32,
    offset: u64,
    entry: ShortEntry,
    /// Whether the query equals the entry's name exactly.
    exact: bool,
}

/// What a directory scan looks for.
#[derive(Clone, Copy)]
enum Query<'q> {
    /// An entry with this name.
    Name(&'q str),
    /// Any visible entry.
    Any,
    /// The subdirectory starting at this cluster.
    Dir(u32),
}

/// Where a new entry goes.
struct Plan {
    /// The first slot of the run.
    start: u32,
    slots: u32,
    /// Clusters to append to the directory first, after `tail`.
    grow: u32,
    tail: u32,
    short: [u8; 11],
    nt_case: u8,
}

/// A name of one path component: not empty, and without `/` or NUL.
fn check_name(name: &str) -> Result<(), ErrorKind> {
    if name.is_empty() || name.contains(['/', '\0']) {
        return Err(ErrorKind::InvalidInput);
    }
    Ok(())
}

/// A name that is neither `.` nor `..`.
fn check_entry_name(name: &str) -> Result<(), ErrorKind> {
    check_name(name)?;
    match name {
        "." | ".." => Err(ErrorKind::InvalidInput),
        _ => Ok(()),
    }
}

io_transform! {

/// A FAT12, FAT16 or FAT32 volume for firmware, with `FILES` file slots.
///
/// See the [module docs](crate::embedded) for the model. A `Fat` holds the
/// device, one 512-byte block buffer, the geometry, the [`Options`] and the
/// file slots, and needs no allocator. Methods that change the volume fail
/// with [`ErrorKind::ReadOnly`] on a read-only mount, and a device that
/// refuses a write makes the mount read-only.
pub struct Fat<D, const FILES: usize = 4> {
    dev: D,
    fat: Volume,
    block: BlockBuf<[u8; BLOCK]>,
    options: Options,
    files: [FileSlot; FILES],
    pending: Pending,
    run: Option<Run>,
    next_generation: u16,
    read_only: bool,
    was_dirty: bool,
}

impl<D, const FILES: usize> core::fmt::Debug for Fat<D, FILES> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Fat")
            .field("kind", &self.fat.geometry().kind())
            .field("cluster_size", &self.fat.geometry().cluster_size())
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl<D: BlockDevice, const FILES: usize> Fat<D, FILES> {
    /// Mounts the volume on `dev` with [`Options::new`].
    pub async fn mount(dev: D) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_with(dev, Options::new()).await
    }

    /// Mounts the volume on `dev`.
    ///
    /// Fails with [`ErrorKind::Unsupported`] when the device's blocks are
    /// not 512 bytes, with [`ErrorKind::NotRecognized`] when the first
    /// sector has no FAT BIOS parameter block, and with
    /// [`ErrorKind::Corrupt`] when the boot sector is not a valid FAT12,
    /// FAT16 or FAT32 boot sector or describes a volume larger than the
    /// device. The [`MountError`] gives `dev` back.
    pub async fn mount_with(mut dev: D, options: Options) -> Result<Self, MountError<D, D::Error>> {
        const { assert!(FILES <= u8::MAX as usize, "at most 255 file slots") };
        if dev.block_size().get() as usize != BLOCK {
            return Err(MountError::new(ErrorKind::Unsupported.into(), dev));
        }
        let mut block = EMPTY_BLOCK;
        let geo = match rawio::read_geometry(&mut dev, &mut block).await {
            Ok(geo) => geo,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        let fat = match rawio::read_fat(&mut dev, &mut block, geo).await {
            Ok(fat) => fat,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        let kind = geo.kind();
        let mut entry = [0u8; 4];
        let at = geo.fat_copy(geo.active_fat()) + kind.entry_offset(1);
        let was_dirty = match kind.clean_bit() {
            0 => false,
            clean => match rawio::read_bytes(&mut dev, &mut block, at, &mut entry[..kind.entry_len()]).await {
                Ok(()) => kind.decode(1, &entry) & clean == 0,
                Err(error) => return Err(MountError::new(error, dev)),
            },
        };
        let serial = geo.volume_serial().unwrap_or(0);
        Ok(Self {
            read_only: options.is_read_only() || !dev.writable(),
            dev,
            fat,
            block,
            options,
            files: [FileSlot::FREE; FILES],
            pending: Pending::NONE,
            run: None,
            next_generation: generation_base(serial),
            was_dirty,
        })
    }

    /// Syncs the volume and gives the device back. When the sync fails
    /// the [`MountError`] holds its error and the device.
    pub async fn unmount(mut self) -> Result<D, MountError<D, D::Error>> {
        match self.sync().await {
            Ok(()) => Ok(self.dev),
            Err(error) => Err(MountError::new(error, self.dev)),
        }
    }

    /// Gives the device back without syncing.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// The geometry from the boot sector.
    pub fn info(&self) -> &Geometry {
        self.fat.geometry()
    }

    /// The options the volume was mounted with.
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Whether the volume is read-only: mounted so, on a device that is
    /// not writable, or after the device refused a write.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether the volume was not cleanly unmounted: the clean bit of FAT
    /// entry 1 was clear at mount. Always false on FAT12.
    pub fn was_dirty(&self) -> bool {
        self.was_dirty
    }

    /// The root directory.
    pub fn root(&self) -> Dir {
        Dir::ROOT
    }

    /// Opens the subdirectory `name` of `parent`. `.` is `parent` and `..`
    /// its parent. Fails with [`ErrorKind::NotFound`] or
    /// [`ErrorKind::NotADirectory`].
    pub async fn open_dir(&mut self, parent: Dir, name: &str) -> FsResult<Dir, D::Error> {
        let start = self.start(parent)?;
        match name {
            "." => return Ok(parent),
            ".." if parent.is_root() => return Ok(Dir::ROOT),
            ".." => return Ok(self.dot_dot(parent.first()).await?.map_or(Dir::ROOT, Dir::new)),
            _ => {}
        }
        check_name(name)?;
        let found = self.find(start, Query::Name(name)).await?.ok_or(ErrorKind::NotFound)?;
        if !found.entry.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        self.child(&found.entry)
    }

    /// Lists `dir` from `from` (`DirCursor::START` for the beginning),
    /// calling `each` with every entry but `.` and `..` until it returns
    /// `ControlFlow::Break`. Continue a listing from the
    /// [`next_cursor`](Entry::next_cursor) of the last entry seen.
    pub async fn list(
        &mut self,
        dir: Dir,
        from: DirCursor,
        mut each: impl FnMut(&Entry<'_>) -> ControlFlow<()>,
    ) -> FsResult<(), D::Error> {
        let start = self.start(dir)?;
        let Ok(mut slot) = u32::try_from(from.into_raw()) else {
            return Ok(());
        };
        let code_page = self.options.code_page();
        let zone = self.options.utc_offset();
        let mut walk = DirWalk::new(start);
        let mut long = Assembler::new();
        let mut short = [0u16; 24];
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            let name = match long.finish(found.entry.lfn_checksum()) {
                Some(units) if !units.is_empty() => units,
                _ => {
                    let len = short_units(&found.entry, code_page, &mut short);
                    &short[..len]
                }
            };
            let entry = Entry::new(name, found.entry, found.offset, found.slot as u64 + 1, zone);
            if each(&entry).is_break() {
                break;
            }
        }
        Ok(())
    }

    /// Opens the file `name` in `dir` in a free slot. `options` follow
    /// `std::fs::OpenOptions`: `create` and `create_new` create the file,
    /// `truncate` empties it, and `append` makes every write go to the end.
    ///
    /// Fails with [`ErrorKind::LimitExceeded`] when every slot is taken,
    /// [`ErrorKind::IsADirectory`] for a directory,
    /// [`ErrorKind::InvalidInput`] for contradictory options, and as
    /// `create_dir` does for a name FAT cannot hold.
    pub async fn open(&mut self, dir: Dir, name: &str, options: OpenOptions) -> FsResult<File, D::Error> {
        options.validate().map_err(ErrorKind::from)?;
        if options.is_write() {
            self.writable()?;
        }
        let index = self.free_slot()?;
        let start = self.start(dir)?;
        check_entry_name(name)?;
        let (offset, entry) = match self.find(start, Query::Name(name)).await? {
            Some(_) if options.is_create_new() => return Err(ErrorKind::AlreadyExists.into()),
            Some(found) => (found.offset, found.entry),
            None if options.is_create() || options.is_create_new() => {
                self.prepare().await?;
                self.create_entry(start, name, false).await?
            }
            None => return Err(ErrorKind::NotFound.into()),
        };
        self.open_entry(index, offset, &entry, options).await
    }

    /// Opens the file a listed [`Entry`] names, without a lookup. The
    /// position is valid until its directory changes; a position that no
    /// longer holds a file fails with [`ErrorKind::NotFound`].
    /// `create_new` fails with [`ErrorKind::AlreadyExists`].
    pub async fn open_node(&mut self, node: Node, options: OpenOptions) -> FsResult<File, D::Error> {
        options.validate().map_err(ErrorKind::from)?;
        if options.is_create_new() {
            return Err(ErrorKind::AlreadyExists.into());
        }
        if options.is_write() {
            self.writable()?;
        }
        let index = self.free_slot()?;
        let offset = node.offset();
        let valid = offset % ENTRY_SIZE == 0 && offset >= self.fat.geometry().fat_start() && offset < self.fat.geometry().data_end();
        if !valid {
            return Err(ErrorKind::NotFound.into());
        }
        let entry = match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
            Slot::Short(entry) if entry.is_visible() => entry,
            _ => return Err(ErrorKind::NotFound.into()),
        };
        self.open_entry(index, offset, &entry, options).await
    }

    /// Reads from the file's position and advances it. Returns 0 at the
    /// end. Fails with [`ErrorKind::InvalidInput`] when the file was not
    /// opened for reading.
    pub async fn read(&mut self, file: &File, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let index = self.slot(file)?;
        let state = self.files[index];
        if !state.has(FLAG_READ) {
            return Err(ErrorKind::InvalidInput.into());
        }
        let (pos, size) = (state.pos as u64, state.size as u64);
        if pos >= size || buf.is_empty() {
            return Ok(0);
        }
        let count = (size - pos).min(buf.len() as u64) as usize;
        let cluster_size = self.fat.geometry().cluster_size() as u64;
        let mut hint = state.at;
        let mut done = 0;
        while done < count {
            let at = pos + done as u64;
            let want = (at / cluster_size) as u32;
            hint = rawio::walk(&mut self.dev, &mut self.block, &self.fat, state.first, hint, want).await?;
            if hint.index() < want {
                return Err(ErrorKind::Corrupt.into());
            }
            let within = at % cluster_size;
            let offset = self.fat.cluster_at(hint.cluster())? + within;
            let n = ((cluster_size - within) as usize).min(count - done);
            let n = rawio::run(&mut self.dev, &mut self.block, &self.fat, &mut hint, n, count - done).await?;
            rawio::read_bytes(&mut self.dev, &mut self.block, offset, &mut buf[done..done + n]).await?;
            done += n;
        }
        let slot = &mut self.files[index];
        slot.pos = (pos + count as u64) as u32;
        slot.at = hint;
        Ok(count)
    }

    /// Writes at the file's position, or at its end in append mode, and
    /// advances the position. A gap past the end reads as zeros. Returns
    /// the bytes written, fewer than `buf.len()` only at the 4 GiB - 1
    /// FAT size limit; a position at the limit fails with
    /// [`ErrorKind::FileTooLarge`], and a full volume with
    /// [`ErrorKind::NoSpace`]. The new size reaches the directory entry at
    /// `flush`, `close`, `sync` or `unmount`.
    pub async fn write(&mut self, file: &File, buf: &[u8]) -> FsResult<usize, D::Error> {
        let index = self.slot(file)?;
        if !self.files[index].has(FLAG_WRITE) {
            return Err(ErrorKind::InvalidInput.into());
        }
        self.prepare().await?;
        let state = self.files[index];
        let pos = if state.has(FLAG_APPEND) { state.size } else { state.pos } as u64;
        if buf.is_empty() {
            return Ok(0);
        }
        if pos >= MAX_FILE_SIZE {
            return Err(ErrorKind::FileTooLarge.into());
        }
        let count = (MAX_FILE_SIZE - pos).min(buf.len() as u64) as usize;
        self.write_at(index, pos, Some(&buf[..count]), count).await?;
        self.files[index].pos = (pos + count as u64) as u32;
        Ok(count)
    }

    /// Moves the file's position and returns it. A position before the
    /// start fails with [`ErrorKind::InvalidInput`], one past the 4 GiB - 1
    /// limit with [`ErrorKind::FileTooLarge`].
    pub fn seek(&mut self, file: &File, pos: SeekFrom) -> FsResult<u64, D::Error> {
        let index = self.slot(file)?;
        let state = self.files[index];
        let pos = pos.resolve(state.pos as u64, state.size as u64).ok_or(ErrorKind::InvalidInput)?;
        if pos > MAX_FILE_SIZE {
            return Err(ErrorKind::FileTooLarge.into());
        }
        self.files[index].pos = pos as u32;
        Ok(pos)
    }

    /// Truncates or extends the file; growth reads as zeros. Shrinking
    /// writes the new size to the entry at once and frees the clusters
    /// past it. Fails with [`ErrorKind::InvalidInput`] when the file was
    /// not opened for writing and [`ErrorKind::FileTooLarge`] past
    /// 4 GiB - 1.
    pub async fn set_len(&mut self, file: &File, len: u64) -> FsResult<(), D::Error> {
        let index = self.slot(file)?;
        if !self.files[index].has(FLAG_WRITE) {
            return Err(ErrorKind::InvalidInput.into());
        }
        if len > MAX_FILE_SIZE {
            return Err(ErrorKind::FileTooLarge.into());
        }
        self.prepare().await?;
        self.resize(index, len).await
    }

    /// Writes the file's size and modification time to its entry and
    /// flushes the device.
    pub async fn flush(&mut self, file: &File) -> FsResult<(), D::Error> {
        let index = self.slot(file)?;
        self.publish(index).await?;
        self.flush_device().await
    }

    /// Closes the file: writes its size and modification time to its
    /// entry, without flushing the device, and frees its slot. When the
    /// write fails the slot stays taken until `sync` or `unmount` writes
    /// it.
    pub async fn close(&mut self, file: File) -> FsResult<(), D::Error> {
        let index = self.slot(&file)?;
        self.publish(index).await?;
        self.files[index] = FileSlot::FREE;
        Ok(())
    }

    /// Creates the directory `name` in `dir`. Fails with
    /// [`ErrorKind::AlreadyExists`] when a long or short name matches,
    /// [`ErrorKind::InvalidInput`] for a name FAT cannot hold (control
    /// characters, `"*/:<>?\|`, or a trailing dot or space),
    /// [`ErrorKind::NameTooLong`] past 255 UTF-16 units, and
    /// [`ErrorKind::NoSpace`] when a FAT12/16 root directory is full.
    pub async fn create_dir(&mut self, dir: Dir, name: &str) -> FsResult<Dir, D::Error> {
        self.prepare().await?;
        let start = self.start(dir)?;
        check_entry_name(name)?;
        let (_, entry) = self.create_entry(start, name, true).await?;
        Ok(Dir::new(entry.first_cluster(self.kind())))
    }

    /// Opens or creates each directory of `path`, a `/`-separated path
    /// relative to `dir`, and returns the last. Empty and `.` components
    /// are skipped; `..` fails with [`ErrorKind::InvalidInput`], and a file
    /// on the way with [`ErrorKind::NotADirectory`].
    pub async fn create_dir_all(&mut self, dir: Dir, path: &str) -> FsResult<Dir, D::Error> {
        let mut current = dir;
        for part in path.split('/').filter(|part| !part.is_empty() && *part != ".") {
            check_entry_name(part)?;
            let start = self.start(current)?;
            current = match self.find(start, Query::Name(part)).await? {
                Some(found) if found.entry.is_dir() => self.child(&found.entry)?,
                Some(_) => return Err(ErrorKind::NotADirectory.into()),
                None => {
                    self.prepare().await?;
                    let (_, entry) = self.create_entry(start, part, true).await?;
                    Dir::new(entry.first_cluster(self.kind()))
                }
            };
        }
        Ok(current)
    }

    /// The metadata of `name` in `dir`. An open file's size is the one its
    /// slots hold.
    pub async fn metadata(&mut self, dir: Dir, name: &str) -> FsResult<Metadata, D::Error> {
        let found = self.lookup(dir, name).await?;
        let size = self
            .open_slot(found.offset)
            .map_or(found.entry.size(), |index| self.files[index].size);
        Ok(crate::names::metadata(&found.entry, found.entry.is_dir(), size as u64, self.options.utc_offset()))
    }

    /// Changes the attributes, times and read-only permission of `name` in
    /// `dir`. Permissions other than those FAT reports and an owner fail
    /// with [`ErrorKind::Unsupported`].
    pub async fn set_attr(&mut self, dir: Dir, name: &str, changes: &SetAttr) -> FsResult<(), D::Error> {
        self.prepare().await?;
        if changes.owner().is_some() {
            return Err(ErrorKind::Unsupported.into());
        }
        let found = self.lookup(dir, name).await?;
        let read_only = match changes.permissions() {
            Some(wanted) => Some(read_only_bit(found.entry.is_dir(), wanted).ok_or(ErrorKind::Unsupported)?),
            None => None,
        };
        self.publish_entry(found.offset).await?;
        let mut entry = self.read_short(found.offset).await?;
        let zone = self.options.utc_offset();
        if let Some(attributes) = changes.attributes() {
            apply_attributes(&mut entry, attributes);
        }
        if let Some(read_only) = read_only {
            set_read_only(&mut entry, read_only);
        }
        if let Some(time) = changes.created() {
            let (date, time, tenths) = date::encode(time, zone);
            entry.set_created(date, time, tenths);
        }
        if let Some(time) = changes.modified() {
            let (date, time, _) = date::encode(time, zone);
            entry.set_modified(date, time);
        }
        if let Some(time) = changes.accessed() {
            entry.set_accessed_date(date::encode(time, zone).0);
        }
        self.put(found.offset, &entry.encode()).await
    }

    /// Removes the file `name` from `dir` and frees its clusters. Fails
    /// with [`ErrorKind::IsADirectory`] for a directory and
    /// [`ErrorKind::Busy`] while the file is open.
    pub async fn remove_file(&mut self, dir: Dir, name: &str) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let start = self.start(dir)?;
        let found = self.lookup(dir, name).await?;
        if found.entry.is_dir() {
            return Err(ErrorKind::IsADirectory.into());
        }
        self.remove_located(start, &found).await
    }

    /// Removes the empty directory `name` from `dir`. Fails with
    /// [`ErrorKind::NotADirectory`] for a file and
    /// [`ErrorKind::DirectoryNotEmpty`] for a directory with entries.
    pub async fn remove_dir(&mut self, dir: Dir, name: &str) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let start = self.start(dir)?;
        let found = self.lookup(dir, name).await?;
        if !found.entry.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        let first = self.fat.check_cluster(found.entry.first_cluster(self.kind()))?;
        if self.find(DirStart::Chain(first), Query::Any).await?.is_some() {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        self.remove_located(start, &found).await
    }

    /// Removes the directory `name` from `dir` with everything in it,
    /// without a stack: it descends to the deepest directory, empties it,
    /// and climbs back through `..`. Fails with
    /// [`ErrorKind::LimitExceeded`] past 1024 levels and with
    /// [`ErrorKind::Busy`] at an open file, having removed what it reached
    /// before.
    pub async fn remove_dir_all(&mut self, dir: Dir, name: &str) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let start = self.start(dir)?;
        let top = self.lookup(dir, name).await?;
        if !top.entry.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        let kind = self.kind();
        let top_first = self.fat.check_cluster(top.entry.first_cluster(kind))?;
        let mut current = top_first;
        let mut depth = 0;
        loop {
            match self.find(DirStart::Chain(current), Query::Any).await? {
                Some(child) if child.entry.is_dir() => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err(ErrorKind::LimitExceeded.into());
                    }
                    current = self.fat.check_cluster(child.entry.first_cluster(kind))?;
                }
                Some(child) => self.remove_located(DirStart::Chain(current), &child).await?,
                None if current == top_first => break,
                None => {
                    let parent = self.dot_dot(current).await?.ok_or(ErrorKind::Corrupt)?;
                    let entry = self
                        .find(DirStart::Chain(parent), Query::Dir(current))
                        .await?
                        .ok_or(ErrorKind::Corrupt)?;
                    self.remove_located(DirStart::Chain(parent), &entry).await?;
                    current = parent;
                    depth -= 1;
                }
            }
        }
        self.remove_located(start, &top).await
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`. Open files follow
    /// the move. Fails with [`ErrorKind::AlreadyExists`] when `to` names
    /// another entry, and with [`ErrorKind::InvalidInput`] when a directory
    /// would move into itself. A change of case only is allowed.
    ///
    /// The new entry is written before the old one is cleared, so an
    /// interruption can leave the node under both names.
    pub async fn rename(&mut self, from_dir: Dir, from: &str, to_dir: Dir, to: &str) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let from_start = self.start(from_dir)?;
        let to_start = self.start(to_dir)?;
        check_entry_name(from)?;
        check_entry_name(to)?;
        let new = NewName::new(to, self.options.code_page(), self.options.fold())?;
        let src = self.find(from_start, Query::Name(from)).await?.ok_or(ErrorKind::NotFound)?;
        self.publish_entry(src.offset).await?;
        let mut moved = self.read_short(src.offset).await?;
        let kind = self.kind();
        let is_dir = moved.is_dir();
        if is_dir {
            let first = self.fat.check_cluster(moved.first_cluster(kind))?;
            if self.is_within(to_start, first).await? {
                return Err(ErrorKind::InvalidInput.into());
            }
        }
        match self.find(to_start, Query::Name(to)).await? {
            Some(target) if target.offset == src.offset && target.exact => return Ok(()),
            Some(target) if target.offset != src.offset => return Err(ErrorKind::AlreadyExists.into()),
            _ => {}
        }
        let skip = (from_start == to_start).then_some(src.offset);
        let plan = self.plan(to_start, to, false, &new, skip).await?;
        self.grow(&plan).await?;
        moved.set_name(plan.short);
        moved.set_nt_case(plan.nt_case);
        if !is_dir {
            moved.set_attributes(moved.attributes() | raw::ATTR_ARCHIVE);
        }
        let offset = self.insert_entry(to_start, &new, &plan, &moved).await?;
        for slot in self.files.iter_mut() {
            if !slot.is_free() && slot.entry == src.offset {
                slot.entry = offset;
            }
        }
        let (old_parent, new_parent) = (self.parent_cluster(from_start), self.parent_cluster(to_start));
        if is_dir && old_parent != new_parent {
            self.set_dot_dot(moved.first_cluster(kind), new_parent).await?;
        }
        self.run = Some(Run { dir: from_start, first: src.first, short: src.slot });
        self.put(src.offset, &[raw::ENTRY_FREE]).await?;
        self.clear(from_start, src.first, src.slot).await?;
        self.run = None;
        Ok(())
    }

    /// The volume label from the label entry of the root directory, or
    /// `None` when it has none, decoded through the code page into `buf`.
    /// Fails with [`ErrorKind::LimitExceeded`] when `buf` is too short.
    pub async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let mut walk = DirWalk::new(self.fat.root());
        let mut slot = 0;
        let mut label = None;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.is_label() => {
                    label = Some(entry.name());
                    break;
                }
                _ => {}
            }
            slot += 1;
        }
        let Some(label) = label else {
            return Ok(None);
        };
        let code_page = self.options.code_page();
        let mut text = [0u8; short_name::DISPLAY_MAX];
        let len = short_name::display_label(&label, |byte| code_page.decode(byte), &mut text);
        let out = buf.get_mut(..len).ok_or(ErrorKind::LimitExceeded)?;
        out.copy_from_slice(&text[..len]);
        Ok(Some(core::str::from_utf8(out).map_err(|_| ErrorKind::Corrupt)?))
    }

    /// Total and free clusters. The free count comes from FSInfo or one
    /// scan of the FAT, and is kept from then on.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        let free = match self.fat.free_clusters() {
            Some(free) => free,
            None => rawio::count_free(&mut self.dev, &mut self.block, &mut self.fat).await?,
        };
        let total = self.fat.geometry().max_cluster() - 1;
        Ok(FsStats::new(total as u64, free as u64, self.fat.geometry().cluster_size()))
    }

    /// Finishes what an interrupted operation left, writes the sizes of
    /// open and dropped files and the FAT32 FSInfo free count, and flushes
    /// the device. Does nothing on a read-only volume.
    pub async fn sync(&mut self) -> FsResult<(), D::Error> {
        if self.read_only {
            return Ok(());
        }
        self.recover().await?;
        for index in 0..FILES {
            self.publish(index).await?;
        }
        let written = rawio::write_fs_info(&mut self.dev, &mut self.block, &mut self.fat).await;
        self.note(written)?;
        self.flush_device().await
    }

    fn kind(&self) -> FatKind {
        self.fat.geometry().kind()
    }

    fn now(&self) -> DateTime {
        (self.options.clock())()
    }

    fn writable(&self) -> Result<(), ErrorKind> {
        if self.read_only { Err(ErrorKind::ReadOnly) } else { Ok(()) }
    }

    /// The slot `file` names, or [`ErrorKind::InvalidHandle`].
    fn slot(&self, file: &File) -> Result<usize, ErrorKind> {
        match self.files.get(file.slot()) {
            Some(slot) if !slot.is_free() && slot.generation == file.generation() => Ok(file.slot()),
            _ => Err(ErrorKind::InvalidHandle),
        }
    }

    fn free_slot(&self) -> Result<usize, ErrorKind> {
        self.files.iter().position(FileSlot::is_free).ok_or(ErrorKind::LimitExceeded)
    }

    /// A slot open on the entry at `offset`.
    fn open_slot(&self, offset: u64) -> Option<usize> {
        self.files.iter().position(|slot| !slot.is_free() && slot.entry == offset)
    }

    fn start(&self, dir: Dir) -> Result<DirStart, ErrorKind> {
        if dir.is_root() {
            return Ok(self.fat.root());
        }
        Ok(DirStart::Chain(self.fat.check_cluster(dir.first())?))
    }

    /// The directory a subdirectory entry names.
    fn child(&self, entry: &ShortEntry) -> FsResult<Dir, D::Error> {
        let first = self.fat.check_cluster(entry.first_cluster(self.kind()))?;
        if self.fat.geometry().root() == RootLocation::Cluster(first) {
            return Err(ErrorKind::Corrupt.into());
        }
        Ok(Dir::new(first))
    }

    /// The parent cluster a `..` entry records for a child of `dir`.
    fn parent_cluster(&self, dir: DirStart) -> u32 {
        match dir {
            DirStart::Chain(cluster) if self.fat.geometry().root() != RootLocation::Cluster(cluster) => cluster,
            _ => 0,
        }
    }

    async fn lookup(&mut self, dir: Dir, name: &str) -> FsResult<Located, D::Error> {
        let start = self.start(dir)?;
        check_entry_name(name)?;
        Ok(self.find(start, Query::Name(name)).await?.ok_or(ErrorKind::NotFound)?)
    }

    async fn open_entry(&mut self, index: usize, offset: u64, entry: &ShortEntry, options: OpenOptions) -> FsResult<File, D::Error> {
        if entry.is_dir() {
            return Err(ErrorKind::IsADirectory.into());
        }
        let mut flags = 0;
        for (on, flag) in [(options.is_read(), FLAG_READ), (options.is_write(), FLAG_WRITE), (options.is_append(), FLAG_APPEND)] {
            if on {
                flags |= flag;
            }
        }
        let mut slot = FileSlot {
            entry: offset,
            first: entry.first_cluster(self.kind()),
            size: entry.size(),
            pos: 0,
            at: ChainPos::NONE,
            generation: self.next_generation,
            flags,
        };
        if let Some(other) = self.open_slot(offset) {
            let other = self.files[other];
            slot.first = other.first;
            slot.size = other.size;
            slot.flags |= other.flags & FLAG_DIRTY;
        }
        self.next_generation = self.next_generation.wrapping_add(1);
        self.files[index] = slot;
        if options.is_truncate() && slot.size != 0 {
            let truncated = match self.prepare().await {
                Ok(()) => self.resize(index, 0).await,
                Err(err) => Err(err),
            };
            if let Err(err) = truncated {
                self.files[index] = FileSlot::FREE;
                return Err(err);
            }
        }
        Ok(File::new(index as u8, slot.generation))
    }

    /// Records a file's new first cluster, size and chain position in
    /// every slot open on it.
    fn record(&mut self, index: usize, first: u32, size: u32, at: ChainPos) {
        let entry = self.files[index].entry;
        for slot in self.files.iter_mut() {
            if !slot.is_free() && slot.entry == entry {
                if slot.first != first {
                    slot.at = ChainPos::NONE;
                }
                slot.first = first;
                slot.size = size;
                slot.flags |= FLAG_DIRTY;
            }
        }
        self.files[index].at = at;
    }

    /// Writes the size, first cluster and modification time a slot holds
    /// to its entry, when the entry lacks them.
    async fn publish(&mut self, index: usize) -> FsResult<(), D::Error> {
        let state = self.files[index];
        if state.is_free() || !state.has(FLAG_DIRTY) || self.read_only {
            return Ok(());
        }
        let mut entry = self.read_short(state.entry).await?;
        entry.set_size(state.size);
        entry.set_first_cluster(self.kind(), state.first);
        let (date, time, _) = date::encode(self.now(), self.options.utc_offset());
        entry.set_modified(date, time);
        entry.set_accessed_date(date);
        entry.set_attributes(entry.attributes() | raw::ATTR_ARCHIVE);
        self.put(state.entry, &entry.encode()).await?;
        for slot in self.files.iter_mut() {
            if !slot.is_free() && slot.entry == state.entry {
                slot.flags &= !FLAG_DIRTY;
            }
        }
        Ok(())
    }

    /// Publishes the slots open on the entry at `offset`.
    async fn publish_entry(&mut self, offset: u64) -> FsResult<(), D::Error> {
        match self.open_slot(offset) {
            Some(index) => self.publish(index).await,
            None => Ok(()),
        }
    }

    /// Writes `count` bytes of `data`, or zeros, at `pos` of the file in
    /// slot `index`, growing it and zero-filling a gap past its end.
    async fn write_at(&mut self, index: usize, pos: u64, data: Option<&[u8]>, count: usize) -> FsResult<(), D::Error> {
        let state = self.files[index];
        let end = pos + count as u64;
        let (first, hint) = self.cover(&state, end).await?;
        let old = state.size as u64;
        let filled = if pos > old {
            self.fill(first, hint, old, None, (pos - old) as usize).await
        } else {
            Ok(hint)
        };
        let written = match filled {
            Ok(hint) => self.fill(first, hint, pos, data, count).await,
            Err(err) => Err(err),
        };
        let hint = match written {
            Ok(hint) => hint,
            Err(err) => {
                let _ = self.recover().await;
                return Err(err);
            }
        };
        self.record(index, first, old.max(end) as u32, hint);
        self.pending = Pending::NONE;
        Ok(())
    }

    /// Changes the size of the file in slot `index` to `len`.
    async fn resize(&mut self, index: usize, len: u64) -> FsResult<(), D::Error> {
        let state = self.files[index];
        let old = state.size as u64;
        if len > old {
            return self.write_at(index, old, None, (len - old) as usize).await;
        }
        if len == old {
            return Ok(());
        }
        let keep = len.div_ceil(self.fat.geometry().cluster_size() as u64) as u32;
        let first = if keep == 0 { 0 } else { state.first };
        if keep == 0 && state.first != 0 {
            self.pending = Pending::chain(state.first, Owner::Entry(state.entry));
        }
        let mut entry = self.read_short(state.entry).await?;
        entry.set_size(len as u32);
        entry.set_first_cluster(self.kind(), first);
        let (date, time, _) = date::encode(self.now(), self.options.utc_offset());
        entry.set_modified(date, time);
        entry.set_accessed_date(date);
        entry.set_attributes(entry.attributes() | raw::ATTR_ARCHIVE);
        self.put(state.entry, &entry.encode()).await?;
        self.record(index, first, len as u32, ChainPos::NONE);
        for slot in self.files.iter_mut() {
            if !slot.is_free() && slot.entry == state.entry {
                slot.flags &= !FLAG_DIRTY;
                slot.at = ChainPos::NONE;
            }
        }
        if state.first == 0 {
            self.pending = Pending::NONE;
            return Ok(());
        }
        if keep == 0 {
            return self.free_chain(state.first).await;
        }
        let reached = rawio::walk(&mut self.dev, &mut self.block, &self.fat, state.first, ChainPos::NONE, keep - 1).await?;
        let last = reached.cluster();
        if reached.index() == keep - 1
            && let Some(next) = rawio::next(&mut self.dev, &mut self.block, &self.fat, last).await?
        {
            self.pending = Pending::chain(next, Owner::Tail(last));
            let end = self.kind().end_of_chain();
            self.set_fat(last, end).await?;
            self.free_chain(next).await?;
        }
        Ok(())
    }

    /// Extends a file's chain to hold `end` bytes, linking the new
    /// clusters, which stay pending until the caller records the size.
    /// Returns the first cluster and a position for writing.
    async fn cover(&mut self, state: &FileSlot, end: u64) -> FsResult<(u32, ChainPos), D::Error> {
        let need = end.div_ceil(self.fat.geometry().cluster_size() as u64) as u32;
        if need == 0 {
            return Ok((state.first, state.at));
        }
        if state.first == 0 {
            self.pending = Pending::chain(0, Owner::First(state.entry));
            let head = self.allocate(need).await?;
            let linked = match self.read_short(state.entry).await {
                Ok(mut entry) => {
                    entry.set_first_cluster(self.kind(), head);
                    self.put(state.entry, &entry.encode()).await
                }
                Err(err) => Err(err),
            };
            if let Err(err) = linked {
                let _ = self.recover().await;
                return Err(err);
            }
            return Ok((head, ChainPos::NONE));
        }
        let reached = rawio::walk(&mut self.dev, &mut self.block, &self.fat, state.first, state.at, need - 1).await?;
        if reached.index() + 1 >= need {
            return Ok((state.first, reached));
        }
        let tail = reached.cluster();
        self.pending = Pending::chain(0, Owner::Tail(tail));
        let added = self.allocate(need - 1 - reached.index()).await?;
        if let Err(err) = self.set_fat(tail, added).await {
            let _ = self.recover().await;
            return Err(err);
        }
        Ok((state.first, reached))
    }

    /// Allocates a chain of `count` clusters into the pending chain and
    /// returns its first cluster. On failure what was taken is freed.
    async fn allocate(&mut self, count: u32) -> FsResult<u32, D::Error> {
        let result = rawio::allocate_run(&mut self.dev, &mut self.block, &mut self.fat, Some(&mut self.pending.held), count).await;
        match self.note(result) {
            Ok(head) => Ok(head),
            Err(err) => {
                let _ = self.recover().await;
                Err(err)
            }
        }
    }

    /// Allocates a chain of `count` zeroed clusters one by one into the
    /// pending chain and returns its first cluster.
    async fn allocate_zeroed(&mut self, count: u32) -> FsResult<u32, D::Error> {
        let cluster_size = self.fat.geometry().cluster_size() as usize;
        let (mut first, mut last) = (0, 0);
        for _ in 0..count {
            let result = rawio::allocate(&mut self.dev, &mut self.block, &mut self.fat, Some(&mut self.pending.held)).await;
            let cluster = self.note(result)?;
            let at = self.fat.cluster_at(cluster)?;
            let zeroed = rawio::write_zeros(&mut self.dev, &mut self.block, at, cluster_size).await;
            self.note(zeroed)?;
            if last != 0 {
                self.set_fat(last, cluster).await?;
                self.pending.held.set_extra(0);
            }
            if first == 0 {
                first = cluster;
            }
            last = cluster;
        }
        Ok(first)
    }

    /// Frees the chain at `first`, which nothing links any more, and
    /// clears the pending chain.
    async fn free_chain(&mut self, first: u32) -> FsResult<(), D::Error> {
        self.pending.owner = Owner::None;
        let result = rawio::free_chain(&mut self.dev, &mut self.block, &mut self.fat, &mut self.pending.held, first).await;
        self.note(result)?;
        self.pending = Pending::NONE;
        Ok(())
    }

    /// Writes `len` bytes of `data`, or zeros, at byte `pos` of the chain at
    /// `first`, and returns the position of the last cluster written.
    async fn fill(&mut self, first: u32, hint: ChainPos, pos: u64, data: Option<&[u8]>, len: usize) -> FsResult<ChainPos, D::Error> {
        if len == 0 {
            return Ok(hint);
        }
        let cluster_size = self.fat.geometry().cluster_size() as u64;
        let mut hint = hint;
        let mut done = 0;
        while done < len {
            let at = pos + done as u64;
            let want = (at / cluster_size) as u32;
            hint = rawio::walk(&mut self.dev, &mut self.block, &self.fat, first, hint, want).await?;
            if hint.index() < want {
                return Err(ErrorKind::Corrupt.into());
            }
            let within = at % cluster_size;
            let offset = self.fat.cluster_at(hint.cluster())? + within;
            let n = ((cluster_size - within) as usize).min(len - done);
            let n = rawio::run(&mut self.dev, &mut self.block, &self.fat, &mut hint, n, len - done).await?;
            let result = match data {
                Some(data) => rawio::write_bytes(&mut self.dev, &mut self.block, offset, &data[done..done + n]).await,
                None => rawio::write_zeros(&mut self.dev, &mut self.block, offset, n).await,
            };
            self.note(result)?;
            done += n;
        }
        Ok(hint)
    }

    /// Creates the entry `text` in `dir`: a directory's zeroed cluster with
    /// `.` and `..` first, then the name. Returns the short entry and its
    /// offset.
    async fn create_entry(&mut self, dir: DirStart, text: &str, is_dir: bool) -> FsResult<(u64, ShortEntry), D::Error> {
        let new = NewName::new(text, self.options.code_page(), self.options.fold())?;
        let plan = self.plan(dir, text, true, &new, None).await?;
        let now = self.now();
        let kind = self.kind();
        let attr = if is_dir { raw::ATTR_DIRECTORY } else { raw::ATTR_ARCHIVE };
        let mut entry = ShortEntry::new(plan.short, attr);
        entry.set_nt_case(plan.nt_case);
        stamp(&mut entry, now, now, now, self.options.utc_offset());
        self.grow(&plan).await?;
        let last = plan.start + plan.slots - 1;
        let mut walk = DirWalk::new(dir);
        let offset = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, last)
            .await?
            .ok_or(ErrorKind::Corrupt)?;
        if is_dir {
            self.pending = Pending::chain(0, Owner::Entry(offset));
            let made = match self.allocate_zeroed(1).await {
                Ok(first) => {
                    entry.set_first_cluster(kind, first);
                    let mut dot = entry;
                    dot.set_name(*b".          ");
                    dot.set_nt_case(0);
                    let mut dot_dot = dot;
                    dot_dot.set_name(*b"..         ");
                    dot_dot.set_first_cluster(kind, self.parent_cluster(dir));
                    let mut raw = [0u8; 2 * ENTRY_SIZE as usize];
                    raw[..32].copy_from_slice(&dot.encode());
                    raw[32..].copy_from_slice(&dot_dot.encode());
                    match self.fat.cluster_at(first) {
                        Ok(at) => self.put(at, &raw).await,
                        Err(kind) => Err(kind.into()),
                    }
                }
                Err(err) => Err(err),
            };
            if let Err(err) = made {
                let _ = self.recover().await;
                return Err(err);
            }
        }
        if let Err(err) = self.insert_entry(dir, &new, &plan, &entry).await {
            let _ = self.recover().await;
            return Err(err);
        }
        self.pending = Pending::NONE;
        Ok((offset, entry))
    }

    /// Finds room for `new` in `dir` and picks its short name. With
    /// `check_exists`, an entry matching `text` fails with
    /// [`ErrorKind::AlreadyExists`]. The short name of the entry at `skip`
    /// does not count as taken.
    async fn plan(&mut self, dir: DirStart, text: &str, check_exists: bool, new: &NewName<'_>, skip: Option<u64>) -> FsResult<Plan, D::Error> {
        let (code_page, fold) = (self.options.code_page(), self.options.fold());
        let needed = new.slots();
        let mut walk = DirWalk::new(dir);
        let mut long = Assembler::new();
        let mut slot = 0;
        let mut end = false;
        let mut taken = 0u8;
        if new.candidates[CANDIDATES - 1][0] == 0 {
            taken |= 1 << (CANDIDATES - 1);
        }
        let (mut run_start, mut run_len) = (0, 0);
        let mut found = None;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            let free = end
                || match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                    Slot::End => {
                        end = true;
                        true
                    }
                    Slot::Free => {
                        long.reset();
                        true
                    }
                    Slot::Long(part) => {
                        long.push(&part);
                        false
                    }
                    Slot::Short(entry) => {
                        let units = long.finish(entry.lfn_checksum()).filter(|units| !units.is_empty());
                        if skip != Some(offset) {
                            if check_exists && entry.is_visible() && matches(text, units, &entry, code_page, fold) {
                                return Err(ErrorKind::AlreadyExists.into());
                            }
                            for (bit, candidate) in new.candidates.iter().enumerate() {
                                if entry.name() == *candidate {
                                    taken |= 1 << bit;
                                }
                            }
                        }
                        false
                    }
                };
            if free {
                if run_len == 0 {
                    run_start = slot;
                }
                run_len += 1;
                if run_len == needed && found.is_none() {
                    found = Some(run_start);
                }
            } else {
                run_len = 0;
            }
            slot += 1;
            if end && found.is_some() {
                break;
            }
        }
        let (start, grow, tail) = match found {
            Some(start) => (start, 0, 0),
            None => {
                let start = if run_len == 0 { slot } else { run_start };
                if matches!(dir, DirStart::Fixed { .. })
                    || start as u64 + needed as u64 > raw::MAX_DIR_ENTRIES as u64
                {
                    return Err(ErrorKind::NoSpace.into());
                }
                let per_cluster = self.fat.geometry().cluster_size() / ENTRY_SIZE as u32;
                (start, (needed - run_len).div_ceil(per_cluster), walk.pos().cluster())
            }
        };
        let short = if new.lossless && taken & 1 == 0 {
            new.candidates[0]
        } else if new.short_only() {
            return Err(ErrorKind::AlreadyExists.into());
        } else if let Some(bit) = (1..new.candidates.len()).find(|bit| taken & (1 << bit) == 0) {
            new.candidates[bit]
        } else {
            self.hashed_short(dir, text, skip).await?
        };
        Ok(Plan {
            start,
            slots: needed,
            grow,
            tail,
            short,
            nt_case: if new.short_only() { new.case_bits.unwrap_or(0) } else { 0 },
        })
    }

    /// A short name with a hashed `HHHH~N` tail that no entry of `dir` has.
    async fn hashed_short(&mut self, dir: DirStart, text: &str, skip: Option<u64>) -> FsResult<[u8; 11], D::Error> {
        let (code_page, fold) = (self.options.code_page(), self.options.fold());
        for suffix in CANDIDATES as u8..=u8::MAX {
            let Some(mut candidate) = crate::names::short_name(text, suffix, code_page, fold) else {
                continue;
            };
            short_name::to_disk(&mut candidate);
            if !self.short_taken(dir, &candidate, skip).await? {
                return Ok(candidate);
            }
        }
        Err(ErrorKind::AlreadyExists.into())
    }

    async fn short_taken(&mut self, dir: DirStart, name: &[u8; 11], skip: Option<u64>) -> FsResult<bool, D::Error> {
        let mut walk = DirWalk::new(dir);
        let mut slot = 0;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.name() == *name && skip != Some(offset) => return Ok(true),
                _ => {}
            }
            slot += 1;
        }
        Ok(false)
    }

    /// Appends the zeroed clusters `plan` needs to its directory.
    async fn grow(&mut self, plan: &Plan) -> FsResult<(), D::Error> {
        if plan.grow == 0 {
            return Ok(());
        }
        self.pending = Pending::chain(0, Owner::Cluster(plan.tail));
        let grown = match self.allocate_zeroed(plan.grow).await {
            Ok(grown) => self.set_fat(plan.tail, grown).await,
            Err(err) => Err(err),
        };
        if let Err(err) = grown {
            let _ = self.recover().await;
            return Err(err);
        }
        self.pending = Pending::NONE;
        Ok(())
    }

    /// Writes `entry` and the long name of `new` into the run `plan` found
    /// and returns the short entry's offset.
    async fn insert_entry(&mut self, dir: DirStart, new: &NewName<'_>, plan: &Plan, entry: &ShortEntry) -> FsResult<u64, D::Error> {
        let last = plan.start + plan.slots - 1;
        let mut walk = DirWalk::new(dir);
        let offset = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, last)
            .await?
            .ok_or(ErrorKind::Corrupt)?;
        self.run = Some(Run { dir, first: plan.start, short: last });
        let checksum = entry.lfn_checksum();
        let mut walk = DirWalk::new(dir);
        let mut written = 0;
        let result = rawio::write_slots(
            &mut self.dev,
            &mut self.block,
            &self.fat,
            &mut walk,
            plan.start,
            plan.slots,
            |index| {
                if index + 1 == plan.slots {
                    entry.encode()
                } else {
                    let (sequence, units) = new.encoded.entry(index as usize);
                    LongEntry::new(sequence, checksum, &units).encode()
                }
            },
            &mut written,
        )
        .await;
        self.note(result)?;
        self.run = None;
        Ok(offset)
    }

    /// Removes an entry and frees its clusters: the short entry first,
    /// then its long name, then the chain.
    async fn remove_located(&mut self, dir: DirStart, found: &Located) -> FsResult<(), D::Error> {
        if self.open_slot(found.offset).is_some() {
            return Err(ErrorKind::Busy.into());
        }
        let first = found.entry.first_cluster(self.kind());
        if first != 0 {
            self.pending = Pending::chain(first, Owner::Entry(found.offset));
        }
        self.run = Some(Run { dir, first: found.first, short: found.slot });
        self.put(found.offset, &[raw::ENTRY_FREE]).await?;
        self.clear(dir, found.first, found.slot).await?;
        self.run = None;
        if first != 0 {
            self.free_chain(first).await?;
        }
        Ok(())
    }

    /// Marks slots `from..to` of `dir` deleted.
    async fn clear(&mut self, dir: DirStart, from: u32, to: u32) -> FsResult<(), D::Error> {
        let result = rawio::clear_slots(&mut self.dev, &mut self.block, &self.fat, dir, from, to).await;
        self.note(result)
    }

    async fn set_dot_dot(&mut self, dir: u32, parent: u32) -> FsResult<(), D::Error> {
        let at = self.fat.cluster_at(dir)? + ENTRY_SIZE;
        let mut entry = self.read_short(at).await?;
        if !entry.is_dot_dot() {
            return Err(ErrorKind::Corrupt.into());
        }
        entry.set_first_cluster(self.kind(), parent);
        self.put(at, &entry.encode()).await
    }

    /// The parent cluster in the `..` entry of the directory at `first`,
    /// or `None` for the root.
    async fn dot_dot(&mut self, first: u32) -> FsResult<Option<u32>, D::Error> {
        let offset = self.fat.cluster_at(first)? + ENTRY_SIZE;
        let Slot::Short(entry) = rawio::read_slot(&mut self.dev, &mut self.block, offset).await? else {
            return Err(ErrorKind::Corrupt.into());
        };
        if !entry.is_dot_dot() {
            return Err(ErrorKind::Corrupt.into());
        }
        let cluster = entry.first_cluster(self.kind());
        if cluster == 0 || self.fat.geometry().root() == RootLocation::Cluster(cluster) {
            return Ok(None);
        }
        Ok(Some(self.fat.check_cluster(cluster)?))
    }

    /// Whether `dir` is the directory starting at `ancestor` or below it.
    async fn is_within(&mut self, dir: DirStart, ancestor: u32) -> FsResult<bool, D::Error> {
        let DirStart::Chain(mut cluster) = dir else {
            return Ok(false);
        };
        for _ in 0..=MAX_DEPTH {
            if cluster == ancestor {
                return Ok(true);
            }
            if self.fat.geometry().root() == RootLocation::Cluster(cluster) {
                return Ok(false);
            }
            match self.dot_dot(cluster).await? {
                Some(up) => cluster = up,
                None => return Ok(false),
            }
        }
        Err(ErrorKind::LimitExceeded.into())
    }

    /// The first entry of `start` that `query` matches.
    async fn find(&mut self, start: DirStart, query: Query<'_>) -> FsResult<Option<Located>, D::Error> {
        let (code_page, fold, kind) = (self.options.code_page(), self.options.fold(), self.kind());
        let mut walk = DirWalk::new(start);
        let mut slot = 0;
        let mut long = Assembler::new();
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            let units = long.finish(found.entry.lfn_checksum());
            let named = units.is_some();
            let units = units.filter(|units| !units.is_empty());
            let (hit, exact) = match query {
                Query::Name(name) => (
                    matches(name, units, &found.entry, code_page, fold),
                    is_exact(name, units, &found.entry, code_page),
                ),
                Query::Any => (true, false),
                Query::Dir(first) => (found.entry.is_dir() && found.entry.first_cluster(kind) == first, false),
            };
            if hit {
                return Ok(Some(Located {
                    first: if named { found.long_start } else { found.slot },
                    slot: found.slot,
                    offset: found.offset,
                    entry: found.entry,
                    exact,
                }));
            }
        }
        Ok(None)
    }

    /// Scans from `slot` to the next visible short entry, feeding long-name
    /// fragments to `long`, and leaves `slot` after it.
    async fn next_visible(&mut self, walk: &mut DirWalk, slot: &mut u32, long: &mut Assembler) -> FsResult<Option<Found>, D::Error> {
        let mut long_start = *slot;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, walk, *slot).await? {
            let at = *slot;
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Free => long.reset(),
                Slot::Long(part) => {
                    if part.sequence() & raw::LFN_LAST_ENTRY != 0 {
                        long_start = at;
                    }
                    long.push(&part)
                }
                Slot::Short(entry) if entry.is_visible() => {
                    *slot = at + 1;
                    return Ok(Some(Found { slot: at, offset, entry, long_start }));
                }
                Slot::Short(_) => long.reset(),
            }
            *slot = at + 1;
        }
        Ok(None)
    }

    async fn read_short(&mut self, offset: u64) -> FsResult<ShortEntry, D::Error> {
        match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
            Slot::Short(entry) => Ok(entry),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    async fn put(&mut self, offset: u64, data: &[u8]) -> FsResult<(), D::Error> {
        let result = rawio::write_bytes(&mut self.dev, &mut self.block, offset, data).await;
        self.note(result)
    }

    async fn set_fat(&mut self, cluster: u32, value: u32) -> FsResult<(), D::Error> {
        let result = rawio::set(&mut self.dev, &mut self.block, &mut self.fat, cluster, value).await;
        self.note(result)
    }

    async fn flush_device(&mut self) -> FsResult<(), D::Error> {
        if self.read_only {
            return Ok(());
        }
        let result = self.dev.flush().await;
        self.note(result)
    }

    /// Marks the volume read-only when the device refused a write.
    fn note<R>(&mut self, result: FsResult<R, D::Error>) -> FsResult<R, D::Error> {
        if let Err(err) = &result
            && err.kind() == ErrorKind::ReadOnly
        {
            self.read_only = true;
        }
        result
    }

    /// Checks that the volume is writable and finishes what an interrupted
    /// operation left.
    async fn prepare(&mut self) -> FsResult<(), D::Error> {
        self.writable()?;
        self.recover().await
    }

    /// Copies FAT entries an interrupted write did not mirror, clears
    /// long-name slots it left without their short entry, and frees the
    /// clusters it held unless it linked them where they stay. What cannot
    /// be finished because the volume is corrupt is dropped.
    async fn recover(&mut self) -> FsResult<(), D::Error> {
        match self.finish_interrupted().await {
            Err(err) if err.kind() == ErrorKind::Corrupt => {
                self.fat.forget_unmirrored();
                self.run = None;
                self.pending = Pending::NONE;
                Ok(())
            }
            other => other,
        }
    }

    async fn finish_interrupted(&mut self) -> FsResult<(), D::Error> {
        if self.fat.unmirrored().is_some() {
            let mirrored = rawio::mirror(&mut self.dev, &mut self.block, &mut self.fat).await;
            self.note(mirrored)?;
        }
        if let Some(run) = self.run {
            let mut walk = DirWalk::new(run.dir);
            let short = match rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, run.short).await? {
                Some(at) => matches!(rawio::read_slot(&mut self.dev, &mut self.block, at).await?, Slot::Short(entry) if entry.is_visible()),
                None => true,
            };
            if !short {
                self.clear(run.dir, run.first, run.short).await?;
            }
            self.run = None;
        }
        if self.pending.is_none() {
            self.pending = Pending::NONE;
            return Ok(());
        }
        let pending = self.pending;
        let (head, extra) = (pending.held.head(), pending.held.extra());
        let linked = head != 0 && self.links(pending.owner, head).await?;
        let keep = match pending.owner {
            Owner::Cluster(_) | Owner::Entry(_) => linked,
            Owner::Tail(tail) => {
                if linked {
                    let end = self.kind().end_of_chain();
                    self.set_fat(tail, end).await?;
                }
                false
            }
            Owner::First(offset) => {
                if linked {
                    let mut entry = self.read_short(offset).await?;
                    entry.set_first_cluster(self.kind(), 0);
                    self.put(offset, &entry.encode()).await?;
                }
                false
            }
            Owner::None => false,
        };
        if !keep {
            self.pending.owner = Owner::None;
            if extra != 0 {
                self.reclaim(extra, head).await?;
            }
            if head != 0 {
                self.reclaim(head, 0).await?;
            }
        }
        self.pending = Pending::NONE;
        Ok(())
    }

    /// Whether `owner` links the chain at `head` on disk.
    async fn links(&mut self, owner: Owner, head: u32) -> FsResult<bool, D::Error> {
        let kind = self.kind();
        Ok(match owner {
            Owner::None => false,
            Owner::Cluster(prev) | Owner::Tail(prev) => {
                self.fat.is_cluster(prev)
                    && rawio::get(&mut self.dev, &mut self.block, &self.fat, prev).await? & kind.mask() == head
            }
            Owner::Entry(offset) | Owner::First(offset) => matches!(
                rawio::read_slot(&mut self.dev, &mut self.block, offset).await?,
                Slot::Short(entry) if entry.is_visible() && entry.first_cluster(kind) == head
            ),
        })
    }

    /// Frees the clusters of a chain from `head` for as long as they are
    /// allocated. `keep` stays pending once the chain ends.
    async fn reclaim(&mut self, head: u32, keep: u32) -> FsResult<(), D::Error> {
        let kind = self.kind();
        let max = self.fat.geometry().max_cluster();
        let mut cluster = head;
        for _ in 0..max {
            if !self.fat.is_cluster(cluster) {
                break;
            }
            let stored = rawio::get(&mut self.dev, &mut self.block, &self.fat, cluster).await? & kind.mask();
            if stored == 0 || kind.is_bad(stored) {
                break;
            }
            let next = kind.next(stored, max).ok().flatten();
            self.pending = Pending {
                held: hadris_fat_raw::io::Held::new(next.unwrap_or(keep), cluster),
                owner: Owner::None,
            };
            self.set_fat(cluster, 0).await?;
            match next {
                Some(next) => cluster = next,
                None => break,
            }
        }
        Ok(())
    }
}

}
