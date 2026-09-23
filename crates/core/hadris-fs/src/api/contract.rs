//! A test kit for drivers: [`check`] runs the format-independent rules of
//! the [`FsDriver`] contract against a writable filesystem.
//!
//! ```rust,ignore
//! let mut fs = FatFs::open_with(dev, MountOptions::new().with_table(HeapTable::new()))?;
//! hadris_fs::sync::contract::check(&mut fs)?;
//! ```

use super::*;
use crate::ContractViolation;

type Outcome<T> = Result<T, ContractViolation>;

const fn name(bytes: &[u8]) -> &Name {
    match Name::from_bytes(bytes) {
        Ok(name) => name,
        Err(_) => panic!("contract names are valid"),
    }
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

io_transform! {

/// Creates an empty file and returns it pinned.
async fn new_file<Fs: FsDriver + ?Sized>(
    fs: &mut Fs,
    case: &'static str,
    dir: NodeId,
    file: &Name,
) -> Outcome<NodeId> {
    ok(case, "create makes a file", fs.create(dir, file, NewNode::File, &SetMetadata::new()).await)
}

/// Looks `file` up, forgets it and returns its id.
async fn id_of<Fs: FsDriver + ?Sized>(
    fs: &mut Fs,
    case: &'static str,
    dir: NodeId,
    file: &Name,
) -> Outcome<NodeId> {
    let node = ok(case, "lookup finds an existing name", fs.lookup(dir, file).await)?;
    fs.forget(node);
    Ok(node)
}

/// Checks the rules of the [`FsDriver`] contract that do not depend on the
/// format, on a writable filesystem with files and directories.
///
/// It works in a directory named `hadris-contract` that it creates under
/// the root and removes again, using short lowercase ASCII names. It checks
/// pins and ids across `lookup`, `create`, listing and `rename`; that a pin
/// never blocks `remove` or a replacing `rename` while an open node does;
/// that a removed pinned node answers [`ErrorKind::NotFound`];
/// [`RemoveKind`], [`RenameFlags::NO_REPLACE`] and the type rules of
/// `rename`; cursor ranges and resumption; and reads, writes and `set_len`.
/// It returns the first rule broken and then leaves the directory behind.
pub async fn check<Fs: FsDriver + ?Sized>(fs: &mut Fs) -> Result<(), ContractViolation> {
    let none = SetMetadata::new();

    let case = "root";
    let root = fs.root();
    holds(case, "NodeId 0 is never a node", root.get() != 0)?;
    let meta = ok(case, "the root has metadata", fs.node_metadata(root).await)?;
    holds(case, "the root is a directory", meta.file_type().is_dir())?;
    holds(case, "the filesystem is writable", fs.capabilities().is_writable())?;

    let case = "create";
    let dir = ok(case, "create makes a directory", fs.create(root, SCRATCH, NewNode::Dir, &none).await)?;
    holds(case, "NodeId 0 is never a node", dir.get() != 0)?;
    let again = id_of(fs, case, root, SCRATCH).await?;
    holds(case, "lookup returns the id create pinned", again == dir)?;
    fails(
        case,
        "creating an existing name fails with AlreadyExists",
        fs.create(root, SCRATCH, NewNode::Dir, &none).await,
        ErrorKind::AlreadyExists,
    )?;
    let a = new_file(fs, case, dir, A).await?;
    fs.forget(a);
    let b = new_file(fs, case, dir, B).await?;
    fs.forget(b);
    let sub = ok(case, "create makes a directory", fs.create(dir, SUB, NewNode::Dir, &none).await)?;
    fs.forget(sub);

    let case = "listing";
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let mut first = None;
    let mut count = 0;
    while let Some(entry) = ok(case, "read_dir_entry succeeds", fs.read_dir_entry(dir, &mut cursor, &mut buf).await)? {
        holds(case, "listings never contain . or ..", buf.as_bytes() != b"." && buf.as_bytes() != b"..")?;
        holds(case, "cursors stay at or below DirCursor::MAX_RAW", cursor.into_raw() <= DirCursor::MAX_RAW)?;
        holds(case, "a directory lists each entry once", count < 3)?;
        first.get_or_insert(cursor);
        count += 1;
        let Some(child) = buf.as_name() else {
            return Err(ContractViolation::new(case, "listed names are valid names", None));
        };
        let looked_up = id_of(fs, case, dir, child).await?;
        holds(case, "a listed id is the id lookup returns", looked_up == entry.node())?;
    }
    holds(case, "a directory lists every entry", count == 3)?;
    let mut resumed = first.unwrap_or(DirCursor::start());
    let mut rest = 0;
    while ok(case, "a stored cursor resumes", fs.read_dir_entry(dir, &mut resumed, &mut buf).await)?.is_some() {
        rest += 1;
        holds(case, "a stored cursor resumes after its entry", rest <= 2)?;
    }
    holds(case, "a stored cursor resumes after its entry", rest == 2)?;

    let case = "remove-kind";
    fails(
        case,
        "removing a directory as a file fails with IsADirectory",
        fs.remove(dir, SUB, RemoveKind::File).await,
        ErrorKind::IsADirectory,
    )?;
    fails(
        case,
        "removing a file as a directory fails with NotADirectory",
        fs.remove(dir, A, RemoveKind::Dir).await,
        ErrorKind::NotADirectory,
    )?;
    fails(
        case,
        "removing a missing name fails with NotFound",
        fs.remove(dir, MISSING, RemoveKind::Any).await,
        ErrorKind::NotFound,
    )?;
    let a = ok(case, "lookup finds an existing name", fs.lookup(dir, A).await)?;
    let inside = fs.lookup(a, B).await;
    fs.forget(a);
    fails(case, "looking up inside a file fails with NotADirectory", inside, ErrorKind::NotADirectory)?;

    let case = "remove-not-empty";
    let sub = ok(case, "lookup finds an existing name", fs.lookup(dir, SUB).await)?;
    let inner = new_file(fs, case, sub, INNER).await?;
    fs.forget(inner);
    fails(
        case,
        "removing a directory with entries fails with DirectoryNotEmpty",
        fs.remove(dir, SUB, RemoveKind::Dir).await,
        ErrorKind::DirectoryNotEmpty,
    )?;
    ok(case, "remove takes a file", fs.remove(sub, INNER, RemoveKind::File).await)?;

    let case = "remove-pinned";
    let a = ok(case, "lookup finds an existing name", fs.lookup(dir, A).await)?;
    ok(case, "a pin does not block remove", fs.remove(dir, A, RemoveKind::File).await)?;
    fails(case, "a removed node answers NotFound", fs.node_metadata(a).await, ErrorKind::NotFound)?;
    fails(case, "a removed node answers NotFound", fs.read_at(a, 0, &mut [0; 1]).await, ErrorKind::NotFound)?;
    fails(case, "a removed name is gone", fs.lookup(dir, A).await, ErrorKind::NotFound)?;
    fs.forget(a);
    let a = new_file(fs, case, dir, A).await?;
    fs.forget(a);

    let case = "remove-open";
    let b = ok(case, "lookup finds an existing name", fs.lookup(dir, B).await)?;
    ok(case, "open_node takes a pinned node", fs.open_node(b).await)?;
    fails(case, "removing an open node fails with Busy", fs.remove(dir, B, RemoveKind::File).await, ErrorKind::Busy)?;
    holds(case, "a refused removal keeps the node", id_of(fs, case, dir, B).await? == b)?;
    fs.close_node(b);
    ok(case, "a closed node can be removed", fs.remove(dir, B, RemoveKind::File).await)?;
    fs.forget(b);

    let case = "rename";
    let c = new_file(fs, case, dir, C).await?;
    ok(case, "rename moves a file", fs.rename(dir, C, dir, D, RenameFlags::empty()).await)?;
    holds(case, "rename keeps the node's id", id_of(fs, case, dir, D).await? == c)?;
    let e = new_file(fs, case, dir, E).await?;
    fs.forget(e);
    fails(
        case,
        "NO_REPLACE refuses an existing target with AlreadyExists",
        fs.rename(dir, D, dir, E, RenameFlags::NO_REPLACE).await,
        ErrorKind::AlreadyExists,
    )?;
    holds(case, "a refused rename changes nothing", id_of(fs, case, dir, D).await? == c)?;
    let e = ok(case, "lookup finds an existing name", fs.lookup(dir, E).await)?;
    ok(case, "a pin does not block a replacing rename", fs.rename(dir, D, dir, E, RenameFlags::empty()).await)?;
    fails(case, "a replaced node answers NotFound", fs.node_metadata(e).await, ErrorKind::NotFound)?;
    fs.forget(e);
    holds(case, "the moved node keeps its id", id_of(fs, case, dir, E).await? == c)?;
    fails(case, "rename removes the old name", fs.lookup(dir, D).await, ErrorKind::NotFound)?;
    let f = new_file(fs, case, dir, F).await?;
    ok(case, "open_node takes a pinned node", fs.open_node(f).await)?;
    let replaced = fs.rename(dir, E, dir, F, RenameFlags::empty()).await;
    fs.close_node(f);
    fs.forget(f);
    fails(case, "replacing an open node fails with Busy", replaced, ErrorKind::Busy)?;
    fails(
        case,
        "a file does not replace a directory",
        fs.rename(dir, E, dir, SUB, RenameFlags::empty()).await,
        ErrorKind::IsADirectory,
    )?;
    fails(
        case,
        "a directory does not replace a file",
        fs.rename(dir, SUB, dir, E, RenameFlags::empty()).await,
        ErrorKind::NotADirectory,
    )?;
    let into_itself = fs.rename(dir, SUB, sub, SUB, RenameFlags::empty()).await;
    fs.forget(sub);
    fails(case, "a directory does not move into itself", into_itself, ErrorKind::InvalidInput)?;

    let case = "data";
    let mut data = [0u8; 1000];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = i as u8;
    }
    let mut done = 0;
    while done < data.len() {
        let n = ok(case, "write_at writes", fs.write_at(c, done as u64, &data[done..]).await)?;
        holds(case, "write_at makes progress", n > 0)?;
        done += n;
    }
    let len = ok(case, "node_metadata succeeds", fs.node_metadata(c).await)?.len();
    holds(case, "node_metadata shows a pending size at once", len == 1000)?;
    let mut back = [0u8; 1000];
    let mut done = 0;
    while done < back.len() {
        let n = ok(case, "read_at reads", fs.read_at(c, done as u64, &mut back[done..]).await)?;
        holds(case, "read_at returns what was written", n > 0)?;
        done += n;
    }
    holds(case, "read_at returns what was written", back == data)?;
    holds(case, "read_at returns 0 at the end", ok(case, "read_at reads", fs.read_at(c, 1000, &mut back).await)? == 0)?;
    ok(case, "set_len shrinks", fs.set_len(c, 10).await)?;
    ok(case, "set_len grows", fs.set_len(c, 20).await)?;
    let mut tail = [0xffu8; 16];
    let n = ok(case, "read_at reads", fs.read_at(c, 10, &mut tail).await)?;
    holds(case, "set_len grows with zeros", n == 10 && tail[..10] == [0; 10])?;
    ok(case, "publish_node succeeds", fs.publish_node(c).await)?;
    ok(case, "sync_node succeeds", fs.sync_node(c).await)?;
    fs.forget(c);

    let case = "cleanup";
    for file in [A, E, F] {
        ok(case, "remove takes a file", fs.remove(dir, file, RemoveKind::File).await)?;
    }
    ok(case, "remove takes an empty directory", fs.remove(dir, SUB, RemoveKind::Dir).await)?;
    ok(case, "a pin does not block remove", fs.remove(root, SCRATCH, RemoveKind::Dir).await)?;
    fs.forget(dir);
    ok(case, "sync succeeds", fs.sync().await)
}

}
