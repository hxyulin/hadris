//! A test kit for drivers: [`check`] runs the format-independent rules of
//! the [`FileSystem`] contract against a writable filesystem, and
//! [`check_read_only`] the rules that hold for a read-only one.
//!
//! ```rust,no_run
//! # #[cfg(feature = "sync")]
//! # fn example<F: hadris_fs::sync::FileSystem, G: hadris_fs::sync::FileSystem>(mut fat: F, mut iso: G) -> Result<(), hadris_fs::ContractViolation> {
//! hadris_fs::sync::contract::check(&mut fat)?;
//! hadris_fs::sync::contract::check_read_only(&mut iso)?;
//! # Ok(())
//! # }
//! ```

use super::*;
use crate::ContractViolation;

type Outcome<T> = Result<T, ContractViolation>;

const fn name(bytes: &[u8]) -> &Name {
    Name::from_bytes(bytes)
}

const SCRATCH: &Name = name(b"hadris-contract");
const A: &Name = name(b"a.txt");
const B: &Name = name(b"b.txt");
const C: &Name = name(b"c.txt");
const D: &Name = name(b"d.txt");
const E: &Name = name(b"e.txt");
const F: &Name = name(b"f.txt");
const SUB: &Name = name(b"sub");
const INNER: &Name = name(b"inner.txt");
const MISSING: &Name = name(b"missing.txt");
const INVALID: [&Name; 5] = [
    name(b""),
    name(b"."),
    name(b".."),
    name(b"a/b"),
    name(b"a\0b"),
];

fn ok<T, E>(case: &'static str, rule: &'static str, result: FsResult<T, E>) -> Outcome<T> {
    result.map_err(|err| ContractViolation::new(case, rule, Some(err.kind())))
}

fn fails<T, E>(
    case: &'static str,
    rule: &'static str,
    result: FsResult<T, E>,
    want: ErrorKind,
) -> Outcome<()> {
    match result {
        Err(err) if err.kind() == want => Ok(()),
        Err(err) => Err(ContractViolation::new(case, rule, Some(err.kind()))),
        Ok(_) => Err(ContractViolation::new(case, rule, None)),
    }
}

fn holds(case: &'static str, rule: &'static str, condition: bool) -> Outcome<()> {
    if condition {
        Ok(())
    } else {
        Err(ContractViolation::new(case, rule, None))
    }
}

/// Directories [`check_read_only`] walks at most.
const MAX_DIRS: usize = 256;

io_transform! {

/// Creates an empty file and returns it pinned.
async fn new_file<Fs: FileSystem + ?Sized>(
    fs: &mut Fs,
    case: &'static str,
    dir: NodeId,
    file: &Name,
) -> Outcome<NodeId> {
    ok(case, "create makes a file", fs.create(dir, file, &SetAttr::new()).await)
}

/// Looks `file` up, forgets it and returns its id.
async fn id_of<Fs: FileSystem + ?Sized>(
    fs: &mut Fs,
    case: &'static str,
    dir: NodeId,
    file: &Name,
) -> Outcome<NodeId> {
    let node = ok(case, "lookup finds an existing name", fs.lookup(dir, file).await)?;
    fs.forget(node, 1);
    Ok(node)
}

/// Checks that `parent` of the root is the root and that the root is a
/// directory of length 0.
async fn root_rules<Fs: FileSystem + ?Sized>(fs: &mut Fs) -> Outcome<NodeId> {
    let case = "root";
    let root = fs.root();
    let meta = ok(case, "the root has metadata", fs.stat(root).await)?;
    holds(case, "the root is a directory", meta.file_type().is_dir())?;
    holds(case, "a directory reports length 0", meta.len() == 0)?;
    let up = ok(case, "parent succeeds", fs.parent(root).await)?;
    fs.forget(up, 1);
    holds(case, "the root is its own parent", up == root)?;
    Ok(root)
}

async_only! {

const CANCEL_PATH: &[u8] = b"/hadris-contract/sub/../e.txt";

/// Forwards to a filesystem, waiting once before its call number `at`, and
/// counts the pins its calls hand out and forget, the root's aside. It
/// keeps the default `resolve`, so a dropped resolution shows in `pins`.
struct Stall<'f, Fs: FileSystem + ?Sized> {
    fs: &'f mut Fs,
    at: u32,
    calls: u32,
    pins: i64,
}

impl<Fs: FileSystem + ?Sized> Stall<'_, Fs> {
    async fn wait(&mut self) {
        let now = self.calls == self.at;
        self.calls += 1;
        let mut waited = !now;
        core::future::poll_fn(|cx| {
            if waited {
                core::task::Poll::Ready(())
            } else {
                waited = true;
                cx.waker().wake_by_ref();
                core::task::Poll::Pending
            }
        })
        .await
    }

    fn count(&mut self, found: &FsResult<NodeId, Fs::DeviceError>) {
        if let Ok(node) = found
            && *node != self.fs.root()
        {
            self.pins += 1;
        }
    }
}

impl<Fs: FileSystem + ?Sized> FileSystem for Stall<'_, Fs> {
    type DeviceError = Fs::DeviceError;

    fn capabilities(&self) -> Capabilities {
        self.fs.capabilities()
    }

    fn root(&self) -> NodeId {
        self.fs.root()
    }

    async fn statfs(&mut self) -> FsResult<FsStats, Self::DeviceError> {
        self.wait().await;
        self.fs.statfs().await
    }

    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, Self::DeviceError> {
        self.wait().await;
        self.fs.label(buf).await
    }

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
        self.wait().await;
        let found = self.fs.lookup(dir, name).await;
        self.count(&found);
        found
    }

    fn forget(&mut self, node: NodeId, count: u64) {
        if node != self.fs.root() {
            self.pins -= count as i64;
        }
        self.fs.forget(node, count);
    }

    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        self.wait().await;
        let found = self.fs.parent(dir).await;
        self.count(&found);
        found
    }

    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
        self.wait().await;
        self.fs.stat(node).await
    }

    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, Self::DeviceError> {
        self.wait().await;
        self.fs.readdir(dir, from).await
    }

    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], Self::DeviceError> {
        self.wait().await;
        self.fs.readlink(node, buf).await
    }

    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), Self::DeviceError> {
        self.wait().await;
        self.fs.open(node, mode).await
    }

    async fn close(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        self.wait().await;
        self.fs.close(node).await
    }

    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        self.wait().await;
        self.fs.read(node, offset, buf).await
    }
}

/// Drops the default `resolve` of `path` at each of its waits, with every
/// policy, and checks that none leaves a pin.
async fn cancelled_resolves<Fs: FileSystem + ?Sized>(fs: &mut Fs, path: &[u8]) -> Outcome<()> {
    let case = "cancel";
    for how in [Resolve::Lexical, Resolve::Follow, Resolve::NoFollow] {
        for at in 0..64 {
            let mut stall = Stall { fs: &mut *fs, at, calls: 0, pins: 0 };
            let (waited, found) = {
                let mut future = core::pin::pin!(stall.resolve(path, how));
                let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
                match core::future::Future::poll(future.as_mut(), &mut cx) {
                    core::task::Poll::Pending => (true, None),
                    core::task::Poll::Ready(found) => (false, found.ok()),
                }
            };
            if let Some(node) = found {
                stall.forget(node, 1);
            }
            holds(case, "a dropped resolve leaves no newly acquired pin", stall.pins == 0)?;
            if !waited {
                break;
            }
        }
    }
    Ok(())
}

/// Writes to `buf` the path of an entry two levels down, `/dir/../dir/entry`,
/// or of a shallower one when the tree has none, and returns its length.
async fn sample_path<Fs: FileSystem + ?Sized>(fs: &mut Fs, buf: &mut [u8]) -> Outcome<usize> {
    let case = "cancel";
    let root = fs.root();
    let mut cursor = DirCursor::START;
    let mut first = None;
    while let Some(entry) = ok(case, "readdir succeeds", fs.readdir(root, cursor).await)? {
        cursor = entry.next_cursor();
        let dir = entry.file_type().is_dir();
        if first.is_none() || dir {
            first = Some((entry, dir));
        }
        if dir {
            break;
        }
    }
    let mut len = 0;
    let mut push = |part: &[u8]| {
        let end = len + 1 + part.len();
        if let Some(out) = buf.get_mut(len..end) {
            out[0] = b'/';
            out[1..].copy_from_slice(part);
            len = end;
        }
    };
    let Some((entry, dir)) = first else {
        push(MISSING.as_bytes());
        return Ok(len);
    };
    push(entry.name().as_bytes());
    if dir {
        push(b"..");
        push(entry.name().as_bytes());
        let node = ok(case, "lookup finds a listed name", fs.lookup(root, entry.name()).await)?;
        let child = fs.readdir(node, DirCursor::START).await;
        fs.forget(node, 1);
        if let Some(child) = ok(case, "readdir succeeds", child)? {
            push(child.name().as_bytes());
        }
    }
    Ok(len)
}

}

/// Checks the rules of the [`FileSystem`] contract that do not depend on
/// the format, on a writable filesystem with files and directories.
///
/// It works in a directory named `hadris-contract` that it creates under
/// the root and removes again, using short lowercase ASCII names. It checks
/// pins and ids across `lookup`, `create`, `mkdir`, listing and `rename`;
/// that invalid names, NUL included, fail with [`ErrorKind::InvalidInput`];
/// that the root is its own parent and directories report length 0; that a
/// pin never blocks `unlink` or a replacing `rename` while an open node
/// does; that a removed pinned node answers [`ErrorKind::NotFound`]; the
/// type rules of `open`, `readlink`, `unlink`, `rmdir` and `rename`;
/// [`RenameMode::NoReplace`]; cursor ranges and resumption; reads, reads
/// past the end, writes and `truncate`; and, in the async mode, that the
/// default `resolve` dropped at any of its waits leaves no pin. It returns
/// the first rule broken and then leaves the directory behind.
pub async fn check<Fs: FileSystem + ?Sized>(fs: &mut Fs) -> Result<(), ContractViolation> {
    let none = SetAttr::new();

    let case = "root";
    holds(case, "the filesystem is writable", fs.capabilities().writable())?;
    let root = root_rules(fs).await?;

    let case = "create";
    let dir = ok(case, "mkdir makes a directory", fs.mkdir(root, SCRATCH, &none).await)?;
    let again = id_of(fs, case, root, SCRATCH).await?;
    holds(case, "lookup returns the id mkdir pinned", again == dir)?;
    fails(
        case,
        "creating an existing name fails with AlreadyExists",
        fs.mkdir(root, SCRATCH, &none).await,
        ErrorKind::AlreadyExists,
    )?;
    let a = new_file(fs, case, dir, A).await?;
    fs.forget(a, 1);
    let b = new_file(fs, case, dir, B).await?;
    fs.forget(b, 1);
    let sub = ok(case, "mkdir makes a directory", fs.mkdir(dir, SUB, &none).await)?;
    fs.forget(sub, 1);

    let case = "names";
    for bad in INVALID {
        fails(case, "lookup of an invalid name fails with InvalidInput", fs.lookup(dir, bad).await, ErrorKind::InvalidInput)?;
        fails(case, "create of an invalid name fails with InvalidInput", fs.create(dir, bad, &none).await, ErrorKind::InvalidInput)?;
        fails(case, "mkdir of an invalid name fails with InvalidInput", fs.mkdir(dir, bad, &none).await, ErrorKind::InvalidInput)?;
    }

    let case = "listing";
    let mut cursor = DirCursor::START;
    let mut first = None;
    let mut count = 0;
    while let Some(entry) = ok(case, "readdir succeeds", fs.readdir(dir, cursor).await)? {
        let listed = entry.name().as_bytes();
        holds(case, "listings never contain . or ..", listed != b"." && listed != b"..")?;
        cursor = entry.next_cursor();
        holds(case, "cursors stay at or below DirCursor::MAX_RAW", cursor.into_raw() <= DirCursor::MAX_RAW)?;
        holds(case, "a directory lists each entry once", count < 3)?;
        first.get_or_insert(cursor);
        count += 1;
        let looked_up = id_of(fs, case, dir, entry.name()).await?;
        holds(case, "a listed id is the id lookup returns", looked_up == entry.node())?;
    }
    holds(case, "a directory lists every entry", count == 3)?;
    let len = ok(case, "stat succeeds", fs.stat(dir).await)?.len();
    holds(case, "a directory reports length 0", len == 0)?;
    let mut resumed = first.unwrap_or(DirCursor::START);
    let mut rest = 0;
    while let Some(entry) = ok(case, "a stored cursor resumes", fs.readdir(dir, resumed).await)? {
        resumed = entry.next_cursor();
        rest += 1;
        holds(case, "a stored cursor resumes after its entry", rest <= 2)?;
    }
    holds(case, "a stored cursor resumes after its entry", rest == 2)?;

    let case = "remove-kind";
    fails(case, "unlink of a directory fails with IsADirectory", fs.unlink(dir, SUB).await, ErrorKind::IsADirectory)?;
    fails(case, "rmdir of a file fails with NotADirectory", fs.rmdir(dir, A).await, ErrorKind::NotADirectory)?;
    fails(case, "unlink of a missing name fails with NotFound", fs.unlink(dir, MISSING).await, ErrorKind::NotFound)?;
    let a = ok(case, "lookup finds an existing name", fs.lookup(dir, A).await)?;
    let inside = fs.lookup(a, B).await;
    fs.forget(a, 1);
    fails(case, "looking up inside a file fails with NotADirectory", inside, ErrorKind::NotADirectory)?;

    let case = "open";
    fails(case, "open of a directory fails with IsADirectory", fs.open(dir, OpenMode::Read).await, ErrorKind::IsADirectory)?;

    let case = "remove-not-empty";
    let sub = ok(case, "lookup finds an existing name", fs.lookup(dir, SUB).await)?;
    let inner = new_file(fs, case, sub, INNER).await?;
    fs.forget(inner, 1);
    fails(
        case,
        "rmdir of a directory with entries fails with DirectoryNotEmpty",
        fs.rmdir(dir, SUB).await,
        ErrorKind::DirectoryNotEmpty,
    )?;
    ok(case, "unlink takes a file", fs.unlink(sub, INNER).await)?;

    let case = "remove-pinned";
    let a = ok(case, "lookup finds an existing name", fs.lookup(dir, A).await)?;
    ok(case, "a pin does not block unlink", fs.unlink(dir, A).await)?;
    fails(case, "a removed node answers NotFound", fs.stat(a).await, ErrorKind::NotFound)?;
    fails(case, "a removed node answers NotFound", fs.open(a, OpenMode::Read).await, ErrorKind::NotFound)?;
    fails(case, "a removed name is gone", fs.lookup(dir, A).await, ErrorKind::NotFound)?;
    fs.forget(a, 1);
    let a = new_file(fs, case, dir, A).await?;
    fs.forget(a, 1);

    let case = "remove-open";
    let b = ok(case, "lookup finds an existing name", fs.lookup(dir, B).await)?;
    ok(case, "open takes a pinned file", fs.open(b, OpenMode::Read).await)?;
    fails(case, "unlink of an open node fails with Busy", fs.unlink(dir, B).await, ErrorKind::Busy)?;
    holds(case, "a refused removal keeps the node", id_of(fs, case, dir, B).await? == b)?;
    ok(case, "close ends an open", fs.close(b).await)?;
    ok(case, "a closed node can be removed", fs.unlink(dir, B).await)?;
    fs.forget(b, 1);

    let case = "rename";
    let c = new_file(fs, case, dir, C).await?;
    ok(case, "rename moves a file", fs.rename(dir, C, dir, D, RenameMode::Replace).await)?;
    holds(case, "rename keeps the node's id", id_of(fs, case, dir, D).await? == c)?;
    let e = new_file(fs, case, dir, E).await?;
    fs.forget(e, 1);
    fails(
        case,
        "NoReplace refuses an existing target with AlreadyExists",
        fs.rename(dir, D, dir, E, RenameMode::NoReplace).await,
        ErrorKind::AlreadyExists,
    )?;
    holds(case, "a refused rename changes nothing", id_of(fs, case, dir, D).await? == c)?;
    let e = ok(case, "lookup finds an existing name", fs.lookup(dir, E).await)?;
    ok(case, "a pin does not block a replacing rename", fs.rename(dir, D, dir, E, RenameMode::Replace).await)?;
    fails(case, "a replaced node answers NotFound", fs.stat(e).await, ErrorKind::NotFound)?;
    fs.forget(e, 1);
    holds(case, "the moved node keeps its id", id_of(fs, case, dir, E).await? == c)?;
    fails(case, "rename removes the old name", fs.lookup(dir, D).await, ErrorKind::NotFound)?;
    let f = new_file(fs, case, dir, F).await?;
    ok(case, "open takes a pinned file", fs.open(f, OpenMode::Read).await)?;
    let replaced = fs.rename(dir, E, dir, F, RenameMode::Replace).await;
    let closed = fs.close(f).await;
    fs.forget(f, 1);
    fails(case, "replacing an open node fails with Busy", replaced, ErrorKind::Busy)?;
    ok(case, "close ends an open", closed)?;
    fails(
        case,
        "a file does not replace a directory",
        fs.rename(dir, E, dir, SUB, RenameMode::Replace).await,
        ErrorKind::IsADirectory,
    )?;
    fails(
        case,
        "a directory does not replace a file",
        fs.rename(dir, SUB, dir, E, RenameMode::Replace).await,
        ErrorKind::NotADirectory,
    )?;
    let into_itself = fs.rename(dir, SUB, sub, SUB, RenameMode::Replace).await;
    fs.forget(sub, 1);
    fails(case, "a directory does not move into itself", into_itself, ErrorKind::InvalidInput)?;

    let case = "data";
    ok(case, "open takes a pinned file", fs.open(c, OpenMode::Write).await)?;
    let mut data = [0u8; 1000];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = i as u8;
    }
    let mut done = 0;
    while done < data.len() {
        let n = ok(case, "write writes", fs.write(c, done as u64, &data[done..]).await)?;
        holds(case, "write makes progress", n > 0)?;
        done += n;
    }
    let len = ok(case, "stat succeeds", fs.stat(c).await)?.len();
    holds(case, "stat shows a pending size at once", len == 1000)?;
    let mut back = [0u8; 1000];
    let mut done = 0;
    while done < back.len() {
        let n = ok(case, "read reads", fs.read(c, done as u64, &mut back[done..]).await)?;
        holds(case, "read returns what was written", n > 0)?;
        done += n;
    }
    holds(case, "read returns what was written", back == data)?;
    holds(case, "read returns 0 at the end", ok(case, "read reads", fs.read(c, 1000, &mut back).await)? == 0)?;
    holds(case, "read returns 0 past the end", ok(case, "read reads", fs.read(c, 5000, &mut back).await)? == 0)?;
    let mut link = [0u8; 64];
    fails(case, "readlink of a file fails with InvalidInput", fs.readlink(c, &mut link).await, ErrorKind::InvalidInput)?;
    fails(case, "readlink of a directory fails with InvalidInput", fs.readlink(dir, &mut link).await, ErrorKind::InvalidInput)?;
    ok(case, "truncate shrinks", fs.truncate(c, 10).await)?;
    ok(case, "truncate grows", fs.truncate(c, 20).await)?;
    let mut tail = [0xffu8; 16];
    let n = ok(case, "read reads", fs.read(c, 10, &mut tail).await)?;
    holds(case, "truncate grows with zeros", n == 10 && tail[..10] == [0; 10])?;
    ok(case, "fsync succeeds", fs.fsync(c).await)?;
    ok(case, "close succeeds", fs.close(c).await)?;
    fs.forget(c, 1);

    async_only! {
        cancelled_resolves(fs, CANCEL_PATH).await?;
    }

    let case = "cleanup";
    for file in [A, E, F] {
        ok(case, "unlink takes a file", fs.unlink(dir, file).await)?;
    }
    ok(case, "rmdir takes an empty directory", fs.rmdir(dir, SUB).await)?;
    ok(case, "a pin does not block rmdir", fs.rmdir(root, SCRATCH).await)?;
    fs.forget(dir, 1);
    ok(case, "sync succeeds", fs.sync().await)
}

/// Checks the rules of the [`FileSystem`] contract that hold for a
/// read-only filesystem, on whatever tree it holds.
///
/// It walks up to 256 directories from the root. In each it checks that
/// listings never contain `.` or `..`, that cursors stay at or below
/// [`DirCursor::MAX_RAW`] and resume after their entry, that a listed id is
/// the id `lookup` returns and has the listed type, that `parent` of a
/// subdirectory is the directory, that directories report length 0, that
/// every file reads back exactly its metadata length and 0 bytes past it,
/// and that a symlink reads its target and fails `open` with
/// [`ErrorKind::Symlink`]. It checks that the root is its own parent, that a
/// missing name, an invalid name (NUL included) and a name inside a file
/// fail with [`ErrorKind::NotFound`], [`ErrorKind::InvalidInput`] and
/// [`ErrorKind::NotADirectory`], that opening a directory fails with
/// [`ErrorKind::IsADirectory`] and `readlink` of anything but a symlink with
/// [`ErrorKind::InvalidInput`], that the capabilities say read-only, that
/// every write method fails with [`ErrorKind::ReadOnly`] and changes
/// nothing, that `sync` and `fsync` succeed with nothing to write, and, in
/// the async mode, that the default `resolve` dropped at any of its waits
/// leaves no pin.
pub async fn check_read_only<Fs: FileSystem + ?Sized>(fs: &mut Fs) -> Result<(), ContractViolation> {
    let none = SetAttr::new();

    let case = "root";
    holds(case, "the filesystem is read-only", !fs.capabilities().writable())?;
    let root = root_rules(fs).await?;
    fails(case, "open of a directory fails with IsADirectory", fs.open(root, OpenMode::Read).await, ErrorKind::IsADirectory)?;
    let mut data = [0u8; 4096];
    fails(
        case,
        "readlink of a directory fails with InvalidInput",
        fs.readlink(root, &mut data).await,
        ErrorKind::InvalidInput,
    )?;

    let case = "read-only";
    ok(case, "sync with nothing to write succeeds", fs.sync().await)?;
    ok(case, "fsync with nothing to write succeeds", fs.fsync(root).await)?;
    fails(case, "create fails with ReadOnly", fs.create(root, SCRATCH, &none).await, ErrorKind::ReadOnly)?;
    fails(case, "mkdir fails with ReadOnly", fs.mkdir(root, SCRATCH, &none).await, ErrorKind::ReadOnly)?;
    fails(case, "setattr fails with ReadOnly", fs.setattr(root, &none).await, ErrorKind::ReadOnly)?;
    fails(case, "lookup of a missing name fails with NotFound", fs.lookup(root, MISSING).await, ErrorKind::NotFound)?;
    for bad in INVALID {
        fails("names", "lookup of an invalid name fails with InvalidInput", fs.lookup(root, bad).await, ErrorKind::InvalidInput)?;
    }

    let mut pending = [root; MAX_DIRS];
    let mut queued = 1;
    let mut walked = 0;
    let mut checked_file = false;
    while walked < queued {
        let dir = pending[walked];
        walked += 1;
        let case = "listing";
        let mut cursor = DirCursor::START;
        let mut count = 0usize;
        let mut first = None;
        while let Some(entry) = ok(case, "readdir succeeds", fs.readdir(dir, cursor).await)? {
            let child = entry.name();
            holds(case, "listings never contain . or ..", child.as_bytes() != b"." && child.as_bytes() != b"..")?;
            cursor = entry.next_cursor();
            holds(case, "cursors stay at or below DirCursor::MAX_RAW", cursor.into_raw() <= DirCursor::MAX_RAW)?;
            first.get_or_insert(cursor);
            count += 1;
            let node = ok(case, "lookup finds a listed name", fs.lookup(dir, child).await)?;
            holds(case, "a listed id is the id lookup returns", node == entry.node())?;
            let meta = ok(case, "stat succeeds for a listed entry", fs.stat(node).await)?;
            holds(case, "metadata has the listed type", meta.file_type() == entry.file_type())?;
            if meta.file_type().is_dir() {
                holds(case, "a directory reports length 0", meta.len() == 0 && entry.metadata().len() == 0)?;
                let parent = ok("parent", "parent succeeds", fs.parent(node).await)?;
                fs.forget(parent, 1);
                holds("parent", "parent of a subdirectory is the directory", parent == dir)?;
                if queued < MAX_DIRS {
                    pending[queued] = node;
                    queued += 1;
                    continue;
                }
            } else if meta.file_type().is_file() {
                let case = "data";
                ok(case, "open takes a file", fs.open(node, OpenMode::Read).await)?;
                let mut offset = 0u64;
                let read = loop {
                    let n = match fs.read(node, offset, &mut data).await {
                        Ok(0) => break Ok(()),
                        Ok(n) => n,
                        Err(err) => break Err(err),
                    };
                    offset += n as u64;
                    if offset > meta.len() {
                        break Ok(());
                    }
                };
                let past = fs.read(node, meta.len().saturating_add(1), &mut data).await;
                let closed = fs.close(node).await;
                ok(case, "read reads", read)?;
                ok(case, "close succeeds", closed)?;
                holds(case, "a file reads back its metadata length", offset == meta.len())?;
                holds(case, "read returns 0 past the end", ok(case, "read reads", past)? == 0)?;
                if !checked_file {
                    checked_file = true;
                    fails(
                        case,
                        "readlink of a file fails with InvalidInput",
                        fs.readlink(node, &mut data).await,
                        ErrorKind::InvalidInput,
                    )?;
                    let case = "read-only";
                    fails(case, "open for writing fails with ReadOnly", fs.open(node, OpenMode::Write).await, ErrorKind::ReadOnly)?;
                    fails(case, "write fails with ReadOnly", fs.write(node, 0, b"x").await, ErrorKind::ReadOnly)?;
                    fails(case, "truncate fails with ReadOnly", fs.truncate(node, 0).await, ErrorKind::ReadOnly)?;
                    ok(case, "fsync with nothing to write succeeds", fs.fsync(node).await)?;
                    fails(case, "looking up inside a file fails with NotADirectory", fs.lookup(node, MISSING).await, ErrorKind::NotADirectory)?;
                    fails(case, "unlink fails with ReadOnly", fs.unlink(dir, child).await, ErrorKind::ReadOnly)?;
                    fails(case, "rename fails with ReadOnly", fs.rename(dir, child, dir, MISSING, RenameMode::Replace).await, ErrorKind::ReadOnly)?;
                    let after = ok(case, "a refused write changes nothing", fs.stat(node).await)?;
                    holds(case, "a refused write changes nothing", after.len() == meta.len())?;
                }
            } else if meta.file_type().is_symlink() {
                let case = "symlink";
                fails(case, "open of a symlink fails with Symlink", fs.open(node, OpenMode::Read).await, ErrorKind::Symlink)?;
                ok(case, "readlink reads a symlink", fs.readlink(node, &mut data).await)?;
            }
            fs.forget(node, 1);
        }
        let mut resumed = first.unwrap_or(DirCursor::START);
        let mut rest = 0usize;
        while let Some(entry) = ok(case, "a stored cursor resumes", fs.readdir(dir, resumed).await)? {
            resumed = entry.next_cursor();
            rest += 1;
            holds(case, "a stored cursor resumes after its entry", rest < count)?;
        }
        holds(case, "a stored cursor resumes after its entry", count == 0 || rest + 1 == count)?;
        if dir != root {
            fs.forget(dir, 1);
        }
    }
    async_only! {
        let mut path = [0u8; 2048];
        let len = sample_path(fs, &mut path).await?;
        cancelled_resolves(fs, &path[..len]).await?;
    }
    Ok(())
}

}
