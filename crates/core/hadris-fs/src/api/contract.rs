//! A test kit for drivers: [`check`] runs the format-independent rules of
//! the [`FileSystem`] contract against a writable filesystem, and
//! [`check_read_only`] the rules that hold for a read-only one.
//!
//! ```rust,ignore
//! hadris_fs::sync::contract::check(&mut fat)?;
//! hadris_fs::sync::contract::check_read_only(&mut iso)?;
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
const INVALID: [&Name; 4] = [name(b""), name(b"."), name(b".."), name(b"a/b")];

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

/// Checks the rules of the [`FileSystem`] contract that do not depend on
/// the format, on a writable filesystem with files and directories.
///
/// It works in a directory named `hadris-contract` that it creates under
/// the root and removes again, using short lowercase ASCII names. It checks
/// pins and ids across `lookup`, `create`, `mkdir`, listing and `rename`;
/// that invalid names fail with [`ErrorKind::InvalidInput`]; that a pin
/// never blocks `unlink` or a replacing `rename` while an open node does;
/// that a removed pinned node answers [`ErrorKind::NotFound`]; the type
/// rules of `open`, `unlink`, `rmdir` and `rename`;
/// [`RenameMode::NoReplace`]; cursor ranges and resumption; and reads,
/// writes and `truncate`. It returns the first rule broken and then leaves
/// the directory behind.
pub async fn check<Fs: FileSystem + ?Sized>(fs: &mut Fs) -> Result<(), ContractViolation> {
    let none = SetAttr::new();

    let case = "root";
    let root = fs.root();
    let meta = ok(case, "the root has metadata", fs.stat(root).await)?;
    holds(case, "the root is a directory", meta.file_type().is_dir())?;
    holds(case, "the filesystem is writable", fs.capabilities().writable())?;

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
    ok(case, "truncate shrinks", fs.truncate(c, 10).await)?;
    ok(case, "truncate grows", fs.truncate(c, 20).await)?;
    let mut tail = [0xffu8; 16];
    let n = ok(case, "read reads", fs.read(c, 10, &mut tail).await)?;
    holds(case, "truncate grows with zeros", n == 10 && tail[..10] == [0; 10])?;
    ok(case, "fsync succeeds", fs.fsync(c).await)?;
    ok(case, "close succeeds", fs.close(c).await)?;
    fs.forget(c, 1);

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
/// subdirectory is the directory, and that every file reads back exactly
/// its metadata length. It checks that a missing name, an invalid name and
/// a name inside a file fail with [`ErrorKind::NotFound`],
/// [`ErrorKind::InvalidInput`] and [`ErrorKind::NotADirectory`], that
/// opening a directory fails with [`ErrorKind::IsADirectory`], that the
/// capabilities say read-only, and that every write method fails with
/// [`ErrorKind::ReadOnly`] and changes nothing.
pub async fn check_read_only<Fs: FileSystem + ?Sized>(fs: &mut Fs) -> Result<(), ContractViolation> {
    let none = SetAttr::new();

    let case = "root";
    let root = fs.root();
    let meta = ok(case, "the root has metadata", fs.stat(root).await)?;
    holds(case, "the root is a directory", meta.file_type().is_dir())?;
    holds(case, "the filesystem is read-only", !fs.capabilities().writable())?;
    fails(case, "open of a directory fails with IsADirectory", fs.open(root, OpenMode::Read).await, ErrorKind::IsADirectory)?;

    let case = "read-only";
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
    let mut data = [0u8; 4096];
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
                let closed = fs.close(node).await;
                ok(case, "read reads", read)?;
                ok(case, "close succeeds", closed)?;
                holds(case, "a file reads back its metadata length", offset == meta.len())?;
                if !checked_file {
                    checked_file = true;
                    let case = "read-only";
                    fails(case, "open for writing fails with ReadOnly", fs.open(node, OpenMode::Write).await, ErrorKind::ReadOnly)?;
                    fails(case, "write fails with ReadOnly", fs.write(node, 0, b"x").await, ErrorKind::ReadOnly)?;
                    fails(case, "truncate fails with ReadOnly", fs.truncate(node, 0).await, ErrorKind::ReadOnly)?;
                    fails(case, "looking up inside a file fails with NotADirectory", fs.lookup(node, MISSING).await, ErrorKind::NotADirectory)?;
                    fails(case, "unlink fails with ReadOnly", fs.unlink(dir, child).await, ErrorKind::ReadOnly)?;
                    fails(case, "rename fails with ReadOnly", fs.rename(dir, child, dir, MISSING, RenameMode::Replace).await, ErrorKind::ReadOnly)?;
                    let after = ok(case, "a refused write changes nothing", fs.stat(node).await)?;
                    holds(case, "a refused write changes nothing", after.len() == meta.len())?;
                }
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
    Ok(())
}

}
