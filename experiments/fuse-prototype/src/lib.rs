//! A prototype FUSE adapter over any sync `hadris_fs` `FileSystem`.
//!
//! [`Adapter`] holds the per-operation logic and returns `Result<_, Errno>`,
//! so it can be driven without a kernel (see `tests/ops.rs`). [`Fuse`] is the
//! thin `fuser::Filesystem` glue around it.
//!
//! Workarounds for gaps in the `hadris-fs` traits are marked `GAP:` and
//! discussed in `docs/v3-trait-review.md`.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::Display;
use std::os::unix::ffi::OsStrExt;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    Errno, FileAttr, FileHandle, FileType as FuseType, Filesystem, FopenFlags, Generation, INodeNo,
    KernelConfig, LockOwner, OpenFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow, WriteFlags,
};
use hadris_fs::sync::FileSystem;
use hadris_fs::{
    Attributes, DateTime, DirCursor, ErrorKind, FileTimes, FileType, Metadata, Name, NameBuf,
    NewNode, NodeId, RenameFlags, SetMetadata,
};

const TTL: Duration = Duration::from_secs(1);
const RENAME_NOREPLACE: u32 = 1;
/// FUSE readdir offsets 1 and 2 are the synthesised `.` and `..`; driver
/// cursors are shifted past them.
const DOT_ENTRIES: u64 = 2;

/// Maps an `ErrorKind` to an errno.
pub fn errno(kind: ErrorKind) -> Errno {
    match kind {
        ErrorKind::Io => Errno::EIO,
        ErrorKind::NotFound => Errno::ENOENT,
        ErrorKind::AlreadyExists => Errno::EEXIST,
        ErrorKind::NotADirectory => Errno::ENOTDIR,
        ErrorKind::IsADirectory => Errno::EISDIR,
        ErrorKind::DirectoryNotEmpty => Errno::ENOTEMPTY,
        ErrorKind::NoSpace => Errno::ENOSPC,
        ErrorKind::ReadOnly => Errno::EROFS,
        ErrorKind::InvalidInput => Errno::EINVAL,
        ErrorKind::Corrupt => Errno::EIO,
        ErrorKind::Unsupported => Errno::EOPNOTSUPP,
        // GAP: one kind for a name that is too long, a file past the format's
        // size limit and a full node table. EFBIG is the most common cause
        // through FUSE (the kernel already rejects names over 255 bytes).
        ErrorKind::LimitExceeded => Errno::EFBIG,
        ErrorKind::Symlink => Errno::ELOOP,
        ErrorKind::InvalidHandle => Errno::ESTALE,
        ErrorKind::Busy => Errno::EBUSY,
        _ => Errno::EIO,
    }
}

fn fail<E: Display>(err: hadris_fs::Error<E>) -> Errno {
    if err.kind() == ErrorKind::Io {
        eprintln!("hadris-fuse: device error: {err}");
    }
    errno(err.kind())
}

fn name(bytes: &OsStr) -> Result<&Name, Errno> {
    Name::from_bytes(bytes.as_bytes()).map_err(|err| match err.kind() {
        ErrorKind::LimitExceeded => Errno::ENAMETOOLONG,
        kind => errno(kind),
    })
}

fn system_time(time: Option<DateTime>) -> SystemTime {
    let Some(time) = time else {
        return UNIX_EPOCH;
    };
    let secs = time.unix_seconds();
    let nanos = Duration::from_nanos(time.nanoseconds() as u64);
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64) + nanos
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs()) + nanos
    }
}

fn date_time(time: SystemTime) -> Option<DateTime> {
    let (secs, nanos) = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
        Err(e) => (-(e.duration().as_secs() as i64), 0),
    };
    DateTime::new(secs, nanos).ok()
}

fn time_or_now(time: TimeOrNow) -> Option<DateTime> {
    date_time(match time {
        TimeOrNow::SpecificTime(time) => time,
        TimeOrNow::Now => SystemTime::now(),
    })
}

fn fuse_type(kind: FileType) -> FuseType {
    match kind {
        FileType::Dir => FuseType::Directory,
        FileType::Symlink => FuseType::Symlink,
        FileType::CharDevice => FuseType::CharDevice,
        FileType::BlockDevice => FuseType::BlockDevice,
        FileType::Fifo => FuseType::NamedPipe,
        FileType::Socket => FuseType::Socket,
        _ => FuseType::RegularFile,
    }
}

/// What the adapter knows about an inode the kernel holds.
#[derive(Debug, Default)]
struct Inode {
    /// Lookups the kernel has not forgotten.
    lookups: u64,
    /// Of those, lookups that hold no driver pin because the adapter
    /// released them to remove the node.
    dead: u64,
    opens: u32,
}

#[derive(Debug)]
struct Handle {
    ino: u64,
    append: bool,
    dirty: bool,
}

#[derive(Debug, Default)]
struct State {
    inodes: HashMap<u64, Inode>,
    handles: HashMap<u64, Handle>,
    next_fh: u64,
}

impl State {
    fn drop_if_idle(&mut self, ino: u64) {
        if self
            .inodes
            .get(&ino)
            .is_some_and(|i| i.lookups == 0 && i.opens == 0)
        {
            self.inodes.remove(&ino);
        }
    }
}

/// Filesystem statistics in FUSE units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Statfs {
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub bsize: u32,
    pub namelen: u32,
}

/// The FUSE operations over a `FileSystem`, one method per request.
pub struct Adapter<F: FileSystem> {
    fs: F,
    root: NodeId,
    uid: u32,
    gid: u32,
    /// Serialises namespace operations (lookup, forget, create, remove,
    /// rename, open, release) so the pin bookkeeping stays consistent. Reads,
    /// writes, getattr and readdir do not take it.
    state: Mutex<State>,
}

impl<F: FileSystem> Adapter<F>
where
    F::DeviceError: Display,
{
    pub fn new(fs: F, uid: u32, gid: u32) -> Self {
        let root = fs.root();
        Self {
            fs,
            root,
            uid,
            gid,
            state: Mutex::new(State {
                next_fh: 1,
                ..State::default()
            }),
        }
    }

    pub fn fs(&self) -> &F {
        &self.fs
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// GAP: the contract does not reserve 0 or say what the root's id is,
    /// while FUSE reserves 0 and fixes the root at 1. Swap the root with
    /// whatever node the driver numbers 1.
    pub fn ino(&self, id: NodeId) -> Result<u64, Errno> {
        match id.get() {
            _ if id == self.root => Ok(INodeNo::ROOT.0),
            0 => Err(Errno::EIO),
            1 => Ok(self.root.get()),
            raw => Ok(raw),
        }
    }

    pub fn node(&self, ino: u64) -> NodeId {
        if ino == INodeNo::ROOT.0 {
            self.root
        } else if ino == self.root.get() {
            NodeId::new(1)
        } else {
            NodeId::new(ino)
        }
    }

    fn attr(&self, ino: u64, meta: &Metadata) -> FileAttr {
        let times = meta.times();
        let kind = fuse_type(meta.file_type());
        let mut perm = if kind == FuseType::Directory {
            0o755
        } else {
            0o644
        };
        if meta.attributes().contains(Attributes::READ_ONLY)
            || !self.fs.capabilities().is_writable()
        {
            perm &= !0o222;
        }
        let modified = times.modified().or(times.created());
        FileAttr {
            ino: INodeNo(ino),
            size: meta.len(),
            // GAP: Metadata has no allocated size; this assumes no holes and
            // ignores cluster rounding, so `du` under-reports.
            blocks: meta.len().div_ceil(512),
            atime: system_time(times.accessed().or(modified)),
            mtime: system_time(modified),
            ctime: system_time(times.changed().or(modified)),
            crtime: system_time(times.created()),
            kind,
            perm: meta.permissions().map_or(perm, |m| m.bits() as u16),
            nlink: meta.nlink().try_into().unwrap_or(u32::MAX),
            uid: meta.owner().map_or(self.uid, |o| o.0),
            gid: meta.owner().map_or(self.gid, |o| o.1),
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    pub fn getattr(&self, ino: u64) -> Result<FileAttr, Errno> {
        let meta = self.fs.node_metadata(self.node(ino)).map_err(fail)?;
        Ok(self.attr(ino, &meta))
    }

    /// Records one kernel lookup of a node the driver has just pinned.
    fn record_lookup(&self, state: &mut State, id: NodeId) -> Result<FileAttr, Errno> {
        let ino = match self.ino(id) {
            Ok(ino) => ino,
            Err(err) => {
                self.fs.forget(id);
                return Err(err);
            }
        };
        match self.fs.node_metadata(id) {
            Ok(meta) => {
                state.inodes.entry(ino).or_default().lookups += 1;
                Ok(self.attr(ino, &meta))
            }
            Err(err) => {
                self.fs.forget(id);
                Err(fail(err))
            }
        }
    }

    pub fn lookup(&self, parent: u64, child: &OsStr) -> Result<FileAttr, Errno> {
        let child = name(child)?;
        let mut state = self.state();
        let id = self.fs.lookup(self.node(parent), child).map_err(fail)?;
        self.record_lookup(&mut state, id)
    }

    /// GAP: `forget` drops one pin, so a FUSE forget of `n` lookups is `n`
    /// driver calls (and `n` lock round trips on a `Volume`).
    pub fn forget(&self, ino: u64, nlookup: u64) {
        if ino == INodeNo::ROOT.0 {
            return;
        }
        let mut state = self.state();
        let Some(inode) = state.inodes.get_mut(&ino) else {
            return;
        };
        let n = nlookup.min(inode.lookups);
        let dead = n.min(inode.dead);
        inode.dead -= dead;
        inode.lookups -= n;
        let id = self.node(ino);
        for _ in dead..n {
            self.fs.forget(id);
        }
        state.drop_if_idle(ino);
    }

    pub fn setattr(
        &self,
        ino: u64,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
    ) -> Result<FileAttr, Errno> {
        let id = self.node(ino);
        if let Some(size) = size {
            self.fs.set_len(id, size).map_err(fail)?;
        }
        let mut times = FileTimes::new();
        if let Some(atime) = atime {
            times = times.with_accessed(time_or_now(atime));
        }
        if let Some(mtime) = mtime {
            times = times.with_modified(time_or_now(mtime));
        }
        if !times.is_empty() {
            self.fs
                .set_metadata(id, &SetMetadata::new().with_times(times))
                .map_err(fail)?;
        }
        // Mode, uid and gid are ignored, as the `set_metadata` contract says
        // for formats that cannot store them; `cp -p` and `tar` then succeed.
        self.getattr(ino)
    }

    /// Lists `ino` from FUSE `offset`, calling `add(ino, next_offset, kind,
    /// name)` until it reports a full buffer.
    pub fn readdir(
        &self,
        ino: u64,
        offset: u64,
        mut add: impl FnMut(u64, u64, FuseType, &[u8]) -> bool,
    ) -> Result<(), Errno> {
        let dir = self.node(ino);
        if offset < 1 && add(ino, 1, FuseType::Directory, b".") {
            return Ok(());
        }
        if offset < 2 {
            // GAP: the parent's id needs a `parent` call (a directory scan on
            // FAT) and a pin that is dropped at once, so the number reported
            // may differ from the one a lookup of `..` would pin.
            let up = if dir == self.root {
                INodeNo::ROOT.0
            } else {
                match self.fs.parent(dir) {
                    Ok(up) => {
                        let ino = self.ino(up).unwrap_or(INodeNo::ROOT.0);
                        self.fs.forget(up);
                        ino
                    }
                    Err(_) => INodeNo::ROOT.0,
                }
            };
            if add(up, 2, FuseType::Directory, b"..") {
                return Ok(());
            }
        }
        // GAP: DirCursor's raw value may use all 64 bits, while FUSE offsets
        // are off_t and 1 and 2 are taken by the dot entries.
        let mut cursor = DirCursor::from_raw(offset.saturating_sub(DOT_ENTRIES));
        let mut buf = NameBuf::new();
        while let Some(entry) = self
            .fs
            .read_dir_entry(dir, &mut cursor, &mut buf)
            .map_err(fail)?
        {
            let next = cursor
                .into_raw()
                .checked_add(DOT_ENTRIES)
                .filter(|&n| n <= i64::MAX as u64)
                .ok_or(Errno::EOVERFLOW)?;
            // GAP: the entry's id is not pinned and, for FAT, can differ from
            // the id a lookup of the same name pins (moved and fallback ids).
            let child = self.ino(entry.node())?;
            if add(child, next, fuse_type(entry.file_type()), buf.as_bytes()) {
                break;
            }
        }
        Ok(())
    }

    fn new_handle(&self, state: &mut State, ino: u64, append: bool) -> u64 {
        let fh = state.next_fh;
        state.next_fh += 1;
        state.handles.insert(
            fh,
            Handle {
                ino,
                append,
                dirty: false,
            },
        );
        state.inodes.entry(ino).or_default().opens += 1;
        fh
    }

    pub fn open(&self, ino: u64, flags: i32) -> Result<u64, Errno> {
        let write = flags & libc::O_ACCMODE != libc::O_RDONLY;
        if write && !self.fs.capabilities().is_writable() {
            return Err(Errno::EROFS);
        }
        let meta = self.fs.node_metadata(self.node(ino)).map_err(fail)?;
        if meta.file_type().is_dir() && write {
            return Err(Errno::EISDIR);
        }
        let mut state = self.state();
        Ok(self.new_handle(&mut state, ino, flags & libc::O_APPEND != 0))
    }

    pub fn read(&self, fh: u64, offset: u64, size: u32) -> Result<Vec<u8>, Errno> {
        let ino = self.handle_ino(fh)?;
        let mut out = vec![0u8; size as usize];
        let mut done = 0;
        while done < out.len() {
            let n = self
                .fs
                .read_at(self.node(ino), offset + done as u64, &mut out[done..])
                .map_err(fail)?;
            if n == 0 {
                break;
            }
            done += n;
        }
        out.truncate(done);
        Ok(out)
    }

    fn handle_ino(&self, fh: u64) -> Result<u64, Errno> {
        self.state()
            .handles
            .get(&fh)
            .map(|h| h.ino)
            .ok_or(Errno::EBADF)
    }

    pub fn write(&self, fh: u64, offset: u64, data: &[u8]) -> Result<u32, Errno> {
        let (ino, append) = {
            let mut state = self.state();
            let handle = state.handles.get_mut(&fh).ok_or(Errno::EBADF)?;
            handle.dirty = true;
            (handle.ino, handle.append)
        };
        let id = self.node(ino);
        let mut at = if append {
            self.fs.node_metadata(id).map_err(fail)?.len()
        } else {
            offset
        };
        let mut rest = data;
        while !rest.is_empty() {
            let n = self.fs.write_at(id, at, rest).map_err(fail)?;
            if n == 0 {
                return Err(Errno::EFBIG);
            }
            at += n as u64;
            rest = &rest[n..];
        }
        u32::try_from(data.len()).map_err(|_| Errno::EINVAL)
    }

    /// GAP: `sync_node` is the only way to publish a pending size, and on
    /// `FatFs` it also flushes the device, so every `close(2)` of a written
    /// file is a device flush.
    pub fn flush(&self, fh: u64) -> Result<(), Errno> {
        let ino = {
            let mut state = self.state();
            let handle = state.handles.get_mut(&fh).ok_or(Errno::EBADF)?;
            if !handle.dirty {
                return Ok(());
            }
            handle.dirty = false;
            handle.ino
        };
        self.fs.sync_node(self.node(ino)).map_err(fail)
    }

    pub fn release(&self, fh: u64) -> Result<(), Errno> {
        let result = self.flush(fh);
        let mut state = self.state();
        if let Some(handle) = state.handles.remove(&fh) {
            if let Some(inode) = state.inodes.get_mut(&handle.ino) {
                inode.opens = inode.opens.saturating_sub(1);
            }
            state.drop_if_idle(handle.ino);
        }
        result
    }

    pub fn fsync(&self, ino: u64) -> Result<(), Errno> {
        self.fs.sync_node(self.node(ino)).map_err(fail)
    }

    pub fn fsyncdir(&self) -> Result<(), Errno> {
        self.fs.sync().map_err(fail)
    }

    pub fn create(&self, parent: u64, child: &OsStr, flags: i32) -> Result<(FileAttr, u64), Errno> {
        let child = name(child)?;
        let mut state = self.state();
        let dir = self.node(parent);
        let id = match self
            .fs
            .create(dir, child, NewNode::File, &SetMetadata::new())
        {
            Ok(id) => id,
            Err(err) if err.kind() == ErrorKind::AlreadyExists && flags & libc::O_EXCL == 0 => {
                self.fs.lookup(dir, child).map_err(fail)?
            }
            Err(err) => return Err(fail(err)),
        };
        let attr = self.record_lookup(&mut state, id)?;
        if attr.kind == FuseType::Directory {
            drop(state);
            self.forget(attr.ino.0, 1);
            return Err(Errno::EISDIR);
        }
        let fh = self.new_handle(&mut state, attr.ino.0, flags & libc::O_APPEND != 0);
        Ok((attr, fh))
    }

    pub fn mkdir(&self, parent: u64, child: &OsStr) -> Result<FileAttr, Errno> {
        self.make(parent, child, NewNode::Dir)
    }

    /// GAP: FAT has no symlinks or devices, and the trait has no way to add
    /// a hard link at all.
    pub fn make(&self, parent: u64, child: &OsStr, kind: NewNode<'_>) -> Result<FileAttr, Errno> {
        let child = name(child)?;
        let mut state = self.state();
        let id = self
            .fs
            .create(self.node(parent), child, kind, &SetMetadata::new())
            .map_err(|err| match err.kind() {
                ErrorKind::Unsupported => Errno::EPERM,
                _ => fail(err),
            })?;
        self.record_lookup(&mut state, id)
    }

    /// Pins `child` for the adapter and returns it with its metadata.
    fn target(&self, dir: NodeId, child: &Name) -> Result<(NodeId, Metadata), Errno> {
        let id = self.fs.lookup(dir, child).map_err(fail)?;
        match self.fs.node_metadata(id) {
            Ok(meta) => Ok((id, meta)),
            Err(err) => {
                self.fs.forget(id);
                Err(fail(err))
            }
        }
    }

    fn is_empty_dir(&self, id: NodeId) -> Result<bool, Errno> {
        let mut cursor = DirCursor::start();
        let mut buf = NameBuf::new();
        Ok(self
            .fs
            .read_dir_entry(id, &mut cursor, &mut buf)
            .map_err(fail)?
            .is_none())
    }

    /// GAP: `remove` and a replacing `rename` fail with `Busy` while the
    /// node is pinned, but the kernel holds a lookup on every name it has
    /// resolved, so every `rm` would fail. The adapter releases the kernel's
    /// pins on `id` and its own from [`target`](Self::target), runs `op`,
    /// and on failure pins the node again through `dir` and `child`. An open
    /// file still gives `EBUSY`.
    fn unpinned<T>(
        &self,
        state: &mut State,
        (dir, child, id): (NodeId, &Name, NodeId),
        op: impl FnOnce() -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let ino = self.ino(id)?;
        let inode = state.inodes.entry(ino).or_default();
        if inode.opens > 0 {
            self.fs.forget(id);
            return Err(Errno::EBUSY);
        }
        let live = inode.lookups - inode.dead;
        for _ in 0..=live {
            self.fs.forget(id);
        }
        inode.dead = inode.lookups;
        let result = op();
        if result.is_err() {
            if let Some(inode) = state.inodes.get_mut(&ino) {
                inode.dead -= live;
            }
            self.repin(dir, child, id, live);
        }
        result
    }

    /// Takes `count` pins on `child` again after a failed removal.
    /// GAP: if the driver hands out a different id, the kernel's inode is
    /// stale and nothing in the trait can bring the old id back.
    fn repin(&self, dir: NodeId, child: &Name, id: NodeId, count: u64) {
        for _ in 0..count {
            match self.fs.lookup(dir, child) {
                Ok(again) if again == id => {}
                Ok(again) => {
                    self.fs.forget(again);
                    eprintln!(
                        "hadris-fuse: {id:?} came back as {again:?}; the kernel inode is stale"
                    );
                    return;
                }
                Err(_) => return,
            }
        }
    }

    fn remove(&self, parent: u64, child: &OsStr, want_dir: bool) -> Result<(), Errno> {
        let child = name(child)?;
        let dir = self.node(parent);
        let mut state = self.state();
        // GAP: `remove` takes no expected type, so unlink and rmdir need a
        // lookup and a metadata call first.
        let (id, meta) = self.target(dir, child)?;
        let is_dir = meta.file_type().is_dir();
        let check = if is_dir != want_dir {
            Err(if want_dir {
                Errno::ENOTDIR
            } else {
                Errno::EISDIR
            })
        } else if is_dir && !self.is_empty_dir(id)? {
            Err(Errno::ENOTEMPTY)
        } else {
            Ok(())
        };
        if let Err(err) = check {
            self.fs.forget(id);
            return Err(err);
        }
        self.unpinned(&mut state, (dir, child, id), || {
            self.fs.remove(dir, child).map_err(fail)
        })
    }

    pub fn unlink(&self, parent: u64, child: &OsStr) -> Result<(), Errno> {
        self.remove(parent, child, false)
    }

    pub fn rmdir(&self, parent: u64, child: &OsStr) -> Result<(), Errno> {
        self.remove(parent, child, true)
    }

    pub fn rename(
        &self,
        parent: u64,
        from: &OsStr,
        new_parent: u64,
        to: &OsStr,
        flags: u32,
    ) -> Result<(), Errno> {
        if flags & !RENAME_NOREPLACE != 0 {
            // RENAME_EXCHANGE and RENAME_WHITEOUT have no RenameFlags yet.
            return Err(Errno::EINVAL);
        }
        let mut rflags = RenameFlags::empty();
        if flags & RENAME_NOREPLACE != 0 {
            rflags |= RenameFlags::NO_REPLACE;
        }
        let (from, to) = (name(from)?, name(to)?);
        let (from_dir, to_dir) = (self.node(parent), self.node(new_parent));
        let mut state = self.state();
        let (src, src_meta) = self.target(from_dir, from)?;
        self.fs.forget(src);
        let target = match self.target(to_dir, to) {
            Ok(target) => Some(target),
            Err(err) if err == Errno::ENOENT => None,
            Err(err) => return Err(err),
        };
        let rename = || {
            self.fs
                .rename(from_dir, from, to_dir, to, rflags)
                .map_err(fail)
        };
        let Some((dst, dst_meta)) = target else {
            return rename();
        };
        if dst == src || rflags.contains(RenameFlags::NO_REPLACE) {
            self.fs.forget(dst);
            return if dst == src {
                rename()
            } else {
                Err(Errno::EEXIST)
            };
        }
        let check = match (src_meta.file_type().is_dir(), dst_meta.file_type().is_dir()) {
            (true, false) => Err(Errno::ENOTDIR),
            (false, true) => Err(Errno::EISDIR),
            (true, true) if !self.is_empty_dir(dst)? => Err(Errno::ENOTEMPTY),
            _ => Ok(()),
        };
        if let Err(err) = check {
            self.fs.forget(dst);
            return Err(err);
        }
        self.unpinned(&mut state, (to_dir, to, dst), rename)
    }

    pub fn statfs(&self) -> Result<Statfs, Errno> {
        let stats = self.fs.stats().map_err(fail)?;
        let caps = self.fs.capabilities();
        Ok(Statfs {
            blocks: stats.total_blocks(),
            bfree: stats.free_blocks(),
            // GAP: FsStats has no space reserved for privileged users.
            bavail: stats.free_blocks(),
            // GAP: no inode counts; FAT has no inode limit but a FAT12/16
            // root directory does have an entry limit.
            files: stats.file_count().unwrap_or(0),
            ffree: 0,
            bsize: stats.block_size(),
            namelen: caps.max_name_len().min(255) as u32,
        })
    }

    pub fn read_link(&self, ino: u64) -> Result<Vec<u8>, Errno> {
        let mut buf = vec![0u8; 4096];
        let n = self
            .fs
            .read_link(self.node(ino), &mut buf)
            .map_err(|err| match err.kind() {
                ErrorKind::Unsupported => Errno::EINVAL,
                _ => fail(err),
            })?;
        buf.truncate(n);
        Ok(buf)
    }

    pub fn sync(&self) -> Result<(), Errno> {
        self.fs.sync().map_err(fail)
    }

    /// Number of inodes the adapter tracks, for tests.
    pub fn tracked_inodes(&self) -> usize {
        self.state().inodes.len()
    }
}

/// The `fuser` glue.
pub struct Fuse<F: FileSystem>(pub Adapter<F>);

impl<F> Filesystem for Fuse<F>
where
    F: FileSystem + Send + Sync + 'static,
    F::DeviceError: Display,
{
    fn init(&mut self, _req: &Request, _config: &mut KernelConfig) -> std::io::Result<()> {
        Ok(())
    }

    fn destroy(&mut self) {
        if let Err(err) = self.0.sync() {
            eprintln!("hadris-fuse: sync at unmount failed: {err:?}");
        }
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.0.lookup(parent.0, name) {
            // GAP: no generation number, so a reused inode number is
            // indistinguishable from the old node.
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(err) => reply.error(err),
        }
    }

    fn forget(&self, _req: &Request, ino: INodeNo, nlookup: u64) {
        self.0.forget(ino.0, nlookup);
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.0.getattr(ino.0) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(err) => reply.error(err),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        match self.0.setattr(ino.0, size, atime, mtime) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(err) => reply.error(err),
        }
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        match self.0.read_link(ino.0) {
            Ok(data) => reply.data(&data),
            Err(err) => reply.error(err),
        }
    }

    fn mknod(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        let number = hadris_fs::DeviceNumber::new(
            (rdev >> 8) & 0xfff,
            (rdev & 0xff) | ((rdev >> 12) & 0xfff00),
        );
        let kind = match mode & libc::S_IFMT as u32 {
            m if m == libc::S_IFREG as u32 => NewNode::File,
            m if m == libc::S_IFCHR as u32 => NewNode::Device(hadris_fs::DeviceKind::Char, number),
            m if m == libc::S_IFBLK as u32 => NewNode::Device(hadris_fs::DeviceKind::Block, number),
            // GAP: FileType has Fifo and Socket, but NewNode cannot create them.
            _ => return reply.error(Errno::EPERM),
        };
        match self.0.make(parent.0, name, kind) {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(err) => reply.error(err),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        match self.0.mkdir(parent.0, name) {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(err) => reply.error(err),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        match self.0.unlink(parent.0, name) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        match self.0.rmdir(parent.0, name) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn symlink(
        &self,
        _req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &std::path::Path,
        reply: ReplyEntry,
    ) {
        let target = target.as_os_str().as_bytes();
        match self.0.make(parent.0, link_name, NewNode::Symlink(target)) {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(err) => reply.error(err),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: fuser::RenameFlags,
        reply: ReplyEmpty,
    ) {
        match self
            .0
            .rename(parent.0, name, newparent.0, newname, flags.bits())
        {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        match self.0.open(ino.0, flags.0) {
            Ok(fh) => reply.opened(FileHandle(fh), FopenFlags::empty()),
            Err(err) => reply.error(err),
        }
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        match self.0.read(fh.0, offset, size) {
            Ok(data) => reply.data(&data),
            Err(err) => reply.error(err),
        }
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        match self.0.write(fh.0, offset, data) {
            Ok(n) => reply.written(n),
            Err(err) => reply.error(err),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        match self.0.flush(fh.0) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        match self.0.release(fh.0) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn fsync(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        match self.0.fsync(ino.0) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn opendir(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        reply.opened(FileHandle(0), FopenFlags::empty());
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let result = self.0.readdir(ino.0, offset, |ino, next, kind, name| {
            reply.add(INodeNo(ino), next, kind, OsStr::from_bytes(name))
        });
        match result {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn fsyncdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        match self.0.fsyncdir() {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(err),
        }
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        match self.0.statfs() {
            Ok(s) => reply.statfs(
                s.blocks, s.bfree, s.bavail, s.files, s.ffree, s.bsize, s.namelen, s.bsize,
            ),
            Err(err) => reply.error(err),
        }
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        match self.0.create(parent.0, name, flags) {
            Ok((attr, fh)) => reply.created(
                &TTL,
                &attr,
                Generation(0),
                FileHandle(fh),
                FopenFlags::empty(),
            ),
            Err(err) => reply.error(err),
        }
    }
}
