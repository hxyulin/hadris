use super::paths::write_all_at;
use super::tree::ContentReader;
use super::*;
use crate::{Field, Node, PathError, Report, Stored, Tree, TreeEntry, Warning, WarningKind};
use alloc::vec::Vec;

/// Bytes moved per read.
const CHUNK: usize = 64 * 1024;

/// How many nodes lost each field.
#[derive(Default)]
struct Dropped([u64; 5]);

const DROPPABLE: [Field; 5] = [
    Field::Created,
    Field::Modified,
    Field::Accessed,
    Field::Permissions,
    Field::Owner,
];

impl Dropped {
    fn add(&mut self, field: Field) {
        if let Some(index) = DROPPABLE.iter().position(|&f| f == field) {
            self.0[index] += 1;
        }
    }

    fn report(&self, report: &mut Report) {
        for (field, &count) in DROPPABLE.iter().zip(&self.0) {
            if count > 0 {
                report.push_warning(
                    Warning::new(WarningKind::Dropped(*field), "not stored by the target")
                        .with_count(count),
                );
            }
        }
    }
}

/// The attributes a filesystem with `caps` keeps from `attrs`, split into
/// what creation takes and the times and attribute bits, which are set once
/// the data is written, since writing sets bits such as FAT's archive bit. Permissions the format stores only in part come back separately,
/// to be tried on their own. Fields the target drops are counted.
fn split_attrs(
    attrs: &SetAttr,
    caps: &Capabilities,
    dropped: &mut Dropped,
) -> (SetAttr, SetAttr, Option<SetAttr>) {
    let mut create = SetAttr::new();
    let mut times = SetAttr::new();
    let mut partial = None;
    let stored = |field| caps.stores(field) != Stored::No;
    for (field, time) in [
        (Field::Created, attrs.created()),
        (Field::Modified, attrs.modified()),
        (Field::Accessed, attrs.accessed()),
    ] {
        let Some(time) = time else { continue };
        if !stored(field) {
            dropped.add(field);
            continue;
        }
        times = match field {
            Field::Created => times.with_created(time),
            Field::Modified => times.with_modified(time),
            _ => times.with_accessed(time),
        };
    }
    if let Some(permissions) = attrs.permissions() {
        match caps.stores(Field::Permissions) {
            Stored::Yes => create = create.with_permissions(permissions),
            Stored::No => dropped.add(Field::Permissions),
            _ => partial = Some(SetAttr::new().with_permissions(permissions)),
        }
    }
    if let Some(owner) = attrs.owner() {
        match caps.stores(Field::Owner) {
            Stored::Yes => create = create.with_owner(owner),
            _ => dropped.add(Field::Owner),
        }
    }
    if let Some(attributes) = attrs.attributes()
        && stored(Field::Attributes)
    {
        times = times.with_attributes(attributes);
    }
    (create, times, partial)
}

/// A directory being copied: its children in the tree, the next one to
/// copy, the target directory (pinned unless it is the top) and the times
/// to set once its children are written.
struct Frame<'t> {
    children: Vec<(&'t Name, TreeEntry<'t>)>,
    next: usize,
    dst: NodeId,
    path: Vec<u8>,
    times: Option<SetAttr>,
}

io_transform! {

/// Sets `partial`, permissions the target stores only in part; a value it
/// cannot hold is counted as dropped.
async fn try_partial<F: FileSystem + ?Sized>(
    fs: &mut F,
    node: NodeId,
    partial: Option<SetAttr>,
    dropped: &mut Dropped,
) -> FsResult<(), F::DeviceError> {
    let Some(partial) = partial else { return Ok(()) };
    match fs.setattr(node, &partial).await {
        Err(err) if err.kind() == ErrorKind::Unsupported => {
            dropped.add(Field::Permissions);
            Ok(())
        }
        other => other,
    }
}

/// Writes the content of `node` into the open file `to`.
async fn copy_data<F: FileSystem + ?Sized>(fs: &mut F, node: &Node, to: NodeId, buf: &mut [u8]) -> Result<(), PathError> {
    let Some(content) = node.content() else { return Ok(()) };
    let mut reader = ContentReader::open(content).await?;
    let mut offset = 0;
    while offset < reader.len() {
        let n = reader.read_at(offset, buf).await?;
        if n == 0 {
            return Err(PathError::new(ErrorKind::Corrupt, "file content ended early"));
        }
        write_all_at(fs, to, offset, &buf[..n]).await?;
        offset += n as u64;
    }
    Ok(())
}

/// Creates the file `name` in `dir` from `node`: its data, then its times.
async fn copy_file<F: FileSystem + ?Sized>(
    fs: &mut F,
    dir: NodeId,
    name: &Name,
    node: &Node,
    dropped: &mut Dropped,
    buf: &mut [u8],
) -> Result<(), PathError> {
    let (create, times, partial) = split_attrs(node.attrs(), &fs.capabilities(), dropped);
    let to = fs.create(dir, name, &create).await?;
    let result = match fs.open(to, OpenMode::Write).await {
        Ok(()) => {
            let mut done = copy_data(fs, node, to, buf).await;
            if done.is_ok() && !times.is_empty() {
                done = fs.setattr(to, &times).await.map_err(PathError::from);
            }
            if done.is_ok() {
                done = try_partial(fs, to, partial, dropped).await.map_err(PathError::from);
            }
            let closed = fs.close(to).await;
            done.and(closed.map_err(PathError::from))
        }
        Err(err) => Err(err.into()),
    };
    fs.forget(to, 1);
    result
}

async fn walk<'t, F: FileSystem + ?Sized>(
    fs: &mut F,
    stack: &mut Vec<Frame<'t>>,
    report: &mut Report,
    dropped: &mut Dropped,
    copied: &mut Vec<usize>,
    buf: &mut [u8],
) -> Result<(), PathError> {
    while let Some(top) = stack.last_mut() {
        let Some(&(name, entry)) = top.children.get(top.next) else {
            let Some(frame) = stack.pop() else { break };
            let applied = match frame.times {
                Some(times) if !times.is_empty() => fs.setattr(frame.dst, &times).await,
                _ => Ok(()),
            };
            if !stack.is_empty() {
                fs.forget(frame.dst, 1);
            }
            applied.map_err(|err| PathError::from(err).with_path(&frame.path))?;
            continue;
        };
        top.next += 1;
        let dir = top.dst;
        let mut path = top.path.clone();
        path.push(b'/');
        path.extend_from_slice(name.as_bytes());
        let node = entry.node();
        match node.file_type() {
            FileType::Dir => {
                let (create, times, partial) = split_attrs(node.attrs(), &fs.capabilities(), dropped);
                let to = fs.mkdir(dir, name, &create).await.map_err(|err| PathError::from(err).with_path(&path))?;
                if let Err(err) = try_partial(fs, to, partial, dropped).await {
                    fs.forget(to, 1);
                    return Err(PathError::from(err).with_path(&path));
                }
                stack.push(Frame { children: entry.children().collect(), next: 0, dst: to, path, times: Some(times) });
            }
            FileType::File if entry.links() > 1 && copied.contains(&entry.id()) => {
                report.push_warning(Warning::new(WarningKind::Skipped, "hard links cannot be created through the filesystem trait").with_path(&path));
            }
            FileType::File => {
                copy_file(fs, dir, name, node, dropped, buf).await.map_err(|err| err.with_path(&path))?;
                if entry.links() > 1 {
                    copied.push(entry.id());
                }
            }
            _ => {
                report.push_warning(Warning::new(WarningKind::Skipped, "symlinks and special files cannot be created through the filesystem trait").with_path(&path));
            }
        }
    }
    Ok(())
}

/// Copies the contents of `tree` into the directory `dir` of `fs`, a
/// mounted filesystem of any format.
///
/// Directories and files are created with the attributes their nodes set
/// where `fs` stores them; each directory's times are set after its
/// children are written, so they survive. Fields `fs` does not store are
/// dropped and reported once per field with the number of nodes. Nodes
/// the filesystem trait cannot create (symlinks, special files and the
/// extra names of a hard-linked file) are skipped, each reported with its
/// path. The attributes of the tree's root are not applied to `dir`.
///
/// An existing name fails with [`ErrorKind::AlreadyExists`]; errors carry
/// the tree path of the node that failed. File content is read in this
/// mode, so the other mode's lazy content fails with
/// [`ErrorKind::Unsupported`]. Files are closed, not flushed; call `sync`
/// to make the copy durable. The report's size is 0 and it lists no
/// extents.
pub async fn copy_tree<F: FileSystem + ?Sized>(tree: &Tree, fs: &mut F, dir: NodeId) -> Result<Report, PathError> {
    let mut report = Report::new();
    let mut dropped = Dropped::default();
    let mut copied = Vec::new();
    let mut buf = alloc::vec![0u8; CHUNK];
    let root = tree.root();
    let mut stack = alloc::vec![Frame { children: root.children().collect(), next: 0, dst: dir, path: Vec::new(), times: None }];
    let result = walk(fs, &mut stack, &mut report, &mut dropped, &mut copied, &mut buf).await;
    for frame in stack.into_iter().skip(1) {
        fs.forget(frame.dst, 1);
    }
    result?;
    dropped.report(&mut report);
    Ok(report)
}

}
