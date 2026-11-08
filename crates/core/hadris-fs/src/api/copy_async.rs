include!("copy_shared.rs");
use super::volume::{Held, Volume};

/// A directory being copied: its children in the tree, the next one to
/// copy, the target directory (pinned unless it is the top) and the times
/// to set once its children are written.
struct Frame<'t, 'v> {
    held: Option<Held<'v>>,
    children: Vec<(&'t Name, TreeEntry<'t>)>,
    next: usize,
    dst: NodeId,
    path: Vec<u8>,
    times: Option<SetAttr>,
}

/// The tree path of the first nonempty file whose lazy content reads
/// through `volume`, which a copy holding the volume would wait on forever.
fn reads_through<F>(tree: &Tree, volume: &Volume<F>) -> Option<Vec<u8>> {
    let origin = volume.origin();
    let join = |path: &[u8], name: &Name| {
        let mut path = path.to_vec();
        path.push(b'/');
        path.extend_from_slice(name.as_bytes());
        path
    };
    let mut stack = alloc::vec![(tree.root(), Vec::new())];
    while let Some((entry, path)) = stack.pop() {
        for (name, child) in entry.children() {
            if child.node().file_type() == FileType::Dir {
                stack.push((child, join(&path, name)));
            } else if child
                .node()
                .content()
                .is_some_and(|content| !content.is_empty() && content.origin() == Some(origin))
            {
                return Some(join(&path, name));
            }
        }
    }
    None
}

io_transform! {

/// Creates the file `name` in `dir` from `node`: its data, then its times.
async fn copy_file<F: FileSystem>(
    fs: &mut F,
    volume: &Volume<F>,
    dir: NodeId,
    name: &Name,
    node: &Node,
    dropped: &mut Dropped,
    buf: &mut [u8],
) -> Result<(), PathError> {
    let (create, times, partial) = split_attrs(node.attrs(), &fs.capabilities(), dropped);
    let to = fs.create(dir, name, &create).await?;
    let mut held = volume.hold(to);
    let result = match fs.open(to, OpenMode::Write).await {
        Ok(()) => {
            held.mark_open();
            let done = copy_file_contents(fs, node, to, &times, partial, dropped, buf).await;
            let closed = fs.close(to).await;
            held.closed();
            done.and(closed.map_err(PathError::from))
        }
        Err(err) => Err(err.into()),
    };
    held.forget(fs);
    result
}

async fn walk<'t, 'v, F: FileSystem>(
    fs: &mut F,
    volume: &'v Volume<F>,
    stack: &mut Vec<Frame<'t, 'v>>,
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
            if let Some(held) = frame.held { held.forget(fs); }
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
                let held = volume.hold(to);
                if let Err(err) = try_partial(fs, to, partial, dropped).await {
                    held.forget(fs);
                    return Err(PathError::from(err).with_path(&path));
                }
                stack.push(Frame { held: Some(held), children: entry.children().collect(), next: 0, dst: to, path, times: Some(times) });
            }
            FileType::File if entry.links() > 1 && copied.contains(&entry.id()) => {
                report.push_warning(Warning::new(WarningKind::Skipped, "hard links cannot be created through the filesystem trait").with_path(&path));
            }
            FileType::File => {
                copy_file(fs, volume, dir, name, node, dropped, buf).await.map_err(|err| err.with_path(&path))?;
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

/// Copies the contents of `tree` into the directory `dir` of `volume`,
/// a mounted filesystem of any format.
///
/// Cancellation queues acquired pins and opens for cleanup on the next
/// volume operation. Partial copied contents may remain. Requires pointer atomics.
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
///
/// # Errors
///
/// Returns [`ErrorKind::AlreadyExists`] for a conflicting name,
/// [`ErrorKind::Unsupported`] for unreadable content in this mode, or
/// filesystem and content-source errors with the tree path. Earlier
/// creations and writes remain on failure and are not flushed. A tree
/// whose nonempty content `read_tree` reads lazily from `volume` itself
/// returns [`ErrorKind::Unsupported`] with the first such path before
/// anything is written.
pub async fn copy_tree<F: FileSystem>(tree: &Tree, volume: &Volume<F>, dir: NodeId) -> Result<Report, PathError> {
    if let Some(path) = reads_through(tree, volume) {
        return Err(PathError::from(ErrorKind::Unsupported).with_path(&path));
    }
    watched!(volume, {
    let mut fs = volume.enter().await?;
    let mut report = Report::new();
    let mut dropped = Dropped::default();
    let mut copied = Vec::new();
    let mut buf = alloc::vec![0u8; CHUNK];
    let root = tree.root();
    let mut stack = alloc::vec![Frame { held: None, children: root.children().collect(), next: 0, dst: dir, path: Vec::new(), times: None }];
    let result = walk(&mut *fs, volume, &mut stack, &mut report, &mut dropped, &mut copied, &mut buf).await;
    for frame in stack.into_iter().skip(1) {
        if let Some(held) = frame.held { held.forget(&mut *fs); }
    }
    result?;
    dropped.report(&mut report);
    Ok(report)
    })
}

}
