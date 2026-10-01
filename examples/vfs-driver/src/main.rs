//! The shape of a kernel VFS or FUSE driver over Hadris: inode numbers are
//! `NodeId`s, every operation goes through the `FileSystem` trait on the
//! detected filesystem (`AnyFs`), lookups are reference counted and released
//! with `forget`, directories are read in pages that resume from a cursor,
//! and every error becomes an errno.
//!
//! The volume is a FAT32 partition on an MBR disk, mounted read-write and
//! then read-only.
//!
//! Catalog actions: IO-PART-01, IO-DETECT-02, VOL-FORMAT-03, VOL-STAT-01,
//! VOL-CAPS-01, VOL-MOUNT-02, DIR-LOOKUP-01, DIR-LIST-01,
//! DIR-MKDIR-01, DIR-RMDIR-01, FILE-CREATE-01, FILE-READ-01, FILE-WRITE-01,
//! FILE-WRITE-02, FILE-TRUNC-01, FILE-RENAME-01, META-STAT-01,
//! HOST-ERRNO-01.
//!
//! ```text
//! cargo run -p hadris-example-vfs-driver
//! ```

use anyhow::{Context, Result, ensure};
use hadris::Errno;
use hadris::fat::{FatKind, FatOptions};
use hadris::fs::sync::FileSystem;
use hadris::fs::{
    DirCursor, FileType, Metadata, MountOptions, Name, NodeId, OpenMode, RenameMode, SetAttr,
};
use hadris::part::{DiskLayout, MbrType, PartitionSpec, Size};
use hadris::storage::{BlockSize, MemDevice};

/// What a VFS sees: inode numbers, attributes and errno values.
struct Driver<F: FileSystem> {
    fs: F,
}

/// An errno as a Rust error, so the scenario can use `?`.
#[derive(Debug)]
struct Failed(Errno);

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name())
    }
}

impl std::error::Error for Failed {}

type Op<T> = Result<T, Failed>;

fn errno<E>(err: hadris::Error<E>) -> Failed {
    Failed(err.kind().errno())
}

/// The errno an operation failed with, or `None` when it succeeded.
fn code<T>(op: Op<T>) -> Option<Errno> {
    op.err().map(|Failed(errno)| errno)
}

impl<F: FileSystem> Driver<F> {
    fn root(&self) -> NodeId {
        self.fs.root()
    }

    /// `lookup`: one reference the kernel later drops with `forget`.
    fn lookup(&mut self, parent: NodeId, name: &str) -> Op<(NodeId, Metadata)> {
        let node = self.fs.lookup(parent, Name::new(name)).map_err(errno)?;
        let attr = self.fs.stat(node).map_err(errno)?;
        Ok((node, attr))
    }

    fn forget(&mut self, node: NodeId, count: u64) {
        self.fs.forget(node, count);
    }

    fn getattr(&mut self, node: NodeId) -> Op<Metadata> {
        self.fs.stat(node).map_err(errno)
    }

    /// `readdir`: the names of at most `page` entries from `cursor`, and
    /// where to resume.
    fn readdir(
        &mut self,
        dir: NodeId,
        mut cursor: DirCursor,
        page: usize,
    ) -> Op<(Vec<String>, Option<DirCursor>)> {
        let mut out = Vec::new();
        while out.len() < page {
            match self.fs.readdir(dir, cursor).map_err(errno)? {
                Some(entry) => {
                    let name = String::from_utf8_lossy(entry.name().as_bytes()).into_owned();
                    out.push(name);
                    cursor = entry.next_cursor();
                }
                None => return Ok((out, None)),
            }
        }
        Ok((out, Some(cursor)))
    }

    fn mkdir(&mut self, parent: NodeId, name: &str) -> Op<NodeId> {
        self.fs
            .mkdir(parent, Name::new(name), &SetAttr::new())
            .map_err(errno)
    }

    /// `create` with `O_CREAT | O_EXCL`, returning an open file.
    fn create(&mut self, parent: NodeId, name: &str) -> Op<NodeId> {
        let node = self
            .fs
            .create(parent, Name::new(name), &SetAttr::new())
            .map_err(errno)?;
        self.fs.open(node, OpenMode::Write).map_err(errno)?;
        Ok(node)
    }

    fn write(&mut self, node: NodeId, offset: u64, data: &[u8]) -> Op<usize> {
        self.fs.write(node, offset, data).map_err(errno)
    }

    fn read(&mut self, node: NodeId, offset: u64, len: usize) -> Op<Vec<u8>> {
        let mut buf = vec![0; len];
        let n = self.fs.read(node, offset, &mut buf).map_err(errno)?;
        buf.truncate(n);
        Ok(buf)
    }

    fn truncate(&mut self, node: NodeId, len: u64) -> Op<()> {
        self.fs.truncate(node, len).map_err(errno)
    }

    fn release(&mut self, node: NodeId) -> Op<()> {
        self.fs.close(node).map_err(errno)
    }

    fn rename(&mut self, dir: NodeId, from: &str, to: &str, mode: RenameMode) -> Op<()> {
        self.fs
            .rename(dir, Name::new(from), dir, Name::new(to), mode)
            .map_err(errno)
    }

    fn rmdir(&mut self, parent: NodeId, name: &str) -> Op<()> {
        self.fs.rmdir(parent, Name::new(name)).map_err(errno)
    }

    fn unlink(&mut self, parent: NodeId, name: &str) -> Op<()> {
        self.fs.unlink(parent, Name::new(name)).map_err(errno)
    }

    fn statfs(&mut self) -> Op<(u64, u64, u32, usize)> {
        let stats = self.fs.statfs().map_err(errno)?;
        let namemax = self.fs.capabilities().max_name_bytes();
        Ok((
            stats.total_blocks(),
            stats.free_blocks(),
            stats.block_size(),
            namemax,
        ))
    }

    fn fsync_all(&mut self) -> Op<()> {
        self.fs.sync().map_err(errno)
    }
}

fn main() -> Result<()> {
    let mut disk = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
    let layout = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::FAT32_LBA, Size::MiB(48)))
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Remaining));
    let table = hadris::part::sync::create(&mut disk, &layout)?;
    let part = table.partition(0).context("no first partition")?;
    let mut window = hadris::part::sync::open(&mut disk, &part)?;
    hadris::fat::sync::format(&mut window, &FatOptions::new().with_kind(FatKind::Fat32))?;

    let fs = hadris::sync::open(window, MountOptions::new()).map_err(|err| err.into_error())?;
    read_write(&mut Driver { fs })?;

    let window = hadris::part::sync::open(&mut disk, &part)?;
    let fs = hadris::sync::open(window, MountOptions::new().read_only())
        .map_err(|err| err.into_error())?;
    read_only(&mut Driver { fs })?;

    println!("VFS operations behaved as a kernel expects");
    Ok(())
}

fn read_write<F: FileSystem>(vfs: &mut Driver<F>) -> Result<()> {
    let root = vfs.root();
    let (total, free_before, block_size, namemax) = vfs.statfs()?;
    ensure!(total > 0 && free_before <= total && block_size >= 512);
    ensure!(namemax >= 255, "FAT long names hold 255 UTF-16 units");

    let logs = vfs.mkdir(root, "logs")?;
    let log = vfs.create(logs, "boot.log")?;
    ensure!(vfs.write(log, 0, b"kernel: started\n")? == 16);
    ensure!(vfs.write(log, 4096, b"init: ready\n")? == 12);
    ensure!(vfs.getattr(log)?.len() == 4096 + 12);
    ensure!(vfs.read(log, 0, 16)? == b"kernel: started\n");
    ensure!(
        vfs.read(log, 16, 8)? == [0; 8],
        "a gap written past reads as zeros"
    );
    vfs.truncate(log, 16)?;
    ensure!(vfs.read(log, 0, 64)? == b"kernel: started\n");
    vfs.release(log)?;
    vfs.forget(log, 1);

    for name in ["a", "b", "c", "d", "e"] {
        let node = vfs.mkdir(logs, name)?;
        vfs.forget(node, 1);
    }
    let mut names = Vec::new();
    let mut cursor = Some(DirCursor::START);
    while let Some(from) = cursor {
        let (page, next) = vfs.readdir(logs, from, 2)?;
        names.extend(page);
        cursor = next;
    }
    names.sort();
    ensure!(names == ["a", "b", "boot.log", "c", "d", "e"], "{names:?}");

    ensure!(code(vfs.lookup(root, "missing")) == Some(Errno::ENOENT));
    ensure!(code(vfs.mkdir(root, "logs")) == Some(Errno::EEXIST));
    ensure!(code(vfs.rmdir(root, "logs")) == Some(Errno::ENOTEMPTY));
    ensure!(code(vfs.rename(logs, "a", "b", RenameMode::NoReplace)) == Some(Errno::EEXIST));
    ensure!(code(vfs.unlink(logs, "a")) == Some(Errno::EISDIR));
    vfs.rename(logs, "boot.log", "boot.0.log", RenameMode::Replace)?;
    vfs.rmdir(logs, "e")?;

    let (_, free_after, ..) = vfs.statfs()?;
    ensure!(free_after < free_before);
    vfs.fsync_all()?;
    vfs.forget(logs, 1);
    Ok(())
}

fn read_only<F: FileSystem>(vfs: &mut Driver<F>) -> Result<()> {
    let root = vfs.root();
    ensure!(!vfs.fs.capabilities().writable());
    let (logs, attr) = vfs.lookup(root, "logs")?;
    ensure!(attr.file_type() == FileType::Dir);
    let (log, attr) = vfs.lookup(logs, "boot.0.log")?;
    ensure!(attr.len() == 16);
    ensure!(vfs.read(log, 0, 64)? == b"kernel: started\n");
    ensure!(code(vfs.mkdir(root, "new")) == Some(Errno::EROFS));
    ensure!(code(vfs.lookup(logs, "e")) == Some(Errno::ENOENT));
    vfs.forget(log, 1);
    vfs.forget(logs, 1);
    Ok(())
}
