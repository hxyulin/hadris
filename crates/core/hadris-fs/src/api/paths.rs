use super::*;

/// Links `Follow` and `NoFollow` follow before failing with
/// [`ErrorKind::Symlink`].
const MAX_LINKS: u32 = 40;

/// Bytes of path plus link text `Follow` and `NoFollow` hold.
const LINK_BUFFER: usize = 1024;

/// The components of `path`, skipping empty ones.
pub(super) fn components(path: &[u8]) -> impl Iterator<Item = &[u8]> + Clone {
    path.split(|&b| b == b'/').filter(|c| !c.is_empty())
}

/// Whether a later `..` in `rest` removes the component just before it.
pub(super) fn cancelled<'a>(rest: impl Iterator<Item = &'a [u8]>) -> bool {
    let mut depth = 0usize;
    for component in rest {
        match component {
            b"." => {}
            b".." if depth == 0 => return true,
            b".." => depth -= 1,
            _ => depth += 1,
        }
    }
    false
}

/// Splits `path` into its parent path and last name. The last name must be
/// a plain name: a path that is empty, the root, or ends in `.` or `..`
/// fails with [`ErrorKind::InvalidInput`].
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub(super) fn split_parent(path: &[u8]) -> Result<(&[u8], &Name), ErrorKind> {
    let mut end = path.len();
    while end > 0 && path[end - 1] == b'/' {
        end -= 1;
    }
    let start = path[..end]
        .iter()
        .rposition(|&b| b == b'/')
        .map_or(0, |i| i + 1);
    let name = &path[start..end];
    if matches!(name, b"" | b"." | b"..") {
        return Err(ErrorKind::InvalidInput);
    }
    Ok((&path[..start], Name::new(name)))
}

/// The pins a resolution holds across `.await`s: the directory it has
/// reached and the child it is looking at. It owns the borrow of the
/// filesystem, so when it is dropped before [`keep`](Self::keep), as when
/// its future is dropped, it forgets them itself; `forget` needs no
/// `.await`.
struct Pins<'f, F: FileSystem + ?Sized> {
    fs: &'f mut F,
    current: NodeId,
    child: Option<NodeId>,
    kept: bool,
}

impl<'f, F: FileSystem + ?Sized> Pins<'f, F> {
    fn new(fs: &'f mut F) -> Self {
        let current = fs.root();
        Self {
            fs,
            current,
            child: None,
            kept: false,
        }
    }

    /// Moves to the pinned `node`, forgetting the directory left.
    fn step(&mut self, node: NodeId) {
        let left = core::mem::replace(&mut self.current, node);
        self.fs.forget(left, 1);
    }

    /// Hands the pin of the node reached to the caller.
    fn keep(mut self) -> NodeId {
        self.kept = true;
        self.current
    }
}

impl<F: FileSystem + ?Sized> Drop for Pins<'_, F> {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            self.fs.forget(child, 1);
        }
        if !self.kept {
            self.fs.forget(self.current, 1);
        }
    }
}

io_transform! {

/// The default of [`FileSystem::resolve`].
pub(super) async fn resolve<F: FileSystem + ?Sized>(
    fs: &mut F,
    path: &[u8],
    how: Resolve,
) -> FsResult<NodeId, F::DeviceError> {
    match how {
        Resolve::Lexical => lexical(fs, path).await,
        Resolve::Follow => posix(fs, path, true).await,
        Resolve::NoFollow => posix(fs, path, false).await,
    }
}

async fn lexical<F: FileSystem + ?Sized>(fs: &mut F, path: &[u8]) -> FsResult<NodeId, F::DeviceError> {
    let mut pins = Pins::new(fs);
    let mut rest = components(path);
    while let Some(component) = rest.next() {
        if matches!(component, b"." | b"..") || cancelled(rest.clone()) {
            continue;
        }
        let found = pins.fs.lookup(pins.current, Name::new(component)).await?;
        pins.step(found);
    }
    Ok(pins.keep())
}

/// POSIX resolution without an allocator: link targets are spliced into a
/// fixed buffer, and a path that outgrows it fails with
/// [`ErrorKind::LimitExceeded`]. `follow_last` says whether a symlink in the
/// last component is followed.
async fn posix<F: FileSystem + ?Sized>(
    fs: &mut F,
    path: &[u8],
    follow_last: bool,
) -> FsResult<NodeId, F::DeviceError> {
    let mut buf = [0u8; LINK_BUFFER];
    let mut spliced: Option<usize> = None;
    let mut pos = 0;
    let mut links = 0;
    let mut pins = Pins::new(fs);
    let mut want_dir = false;
    loop {
        let pending = match spliced {
            Some(start) => &buf[start..],
            None => path,
        };
        while pending.get(pos) == Some(&b'/') {
            pos += 1;
        }
        if pos == pending.len() {
            break;
        }
        let end = pending[pos..]
            .iter()
            .position(|&c| c == b'/')
            .map_or(pending.len(), |i| pos + i);
        let (from, rest_len) = (pos, pending.len() - end);
        want_dir = rest_len > 0;
        pos = end;
        let component = &pending[from..end];
        if component == b"." {
            if !pins.fs.stat(pins.current).await?.file_type().is_dir() {
                return Err(ErrorKind::NotADirectory.into());
            }
            continue;
        }
        if component == b".." {
            let up = pins.fs.parent(pins.current).await?;
            pins.step(up);
            continue;
        }
        let child = pins.fs.lookup(pins.current, Name::new(component)).await?;
        pins.child = Some(child);
        let meta = pins.fs.stat(child).await?;
        if meta.file_type() != FileType::Symlink || (rest_len == 0 && !follow_last) {
            pins.child = None;
            pins.step(child);
            continue;
        }
        links += 1;
        if links > MAX_LINKS {
            return Err(ErrorKind::Symlink.into());
        }
        let sep = usize::from(rest_len > 0);
        let room = LINK_BUFFER.checked_sub(rest_len + sep).ok_or(ErrorKind::LimitExceeded)?;
        if spliced.is_none() {
            buf[LINK_BUFFER - rest_len..].copy_from_slice(&path[end..]);
        }
        let len = pins.fs.readlink(child, &mut buf[..room]).await?.len();
        pins.child = None;
        pins.fs.forget(child, 1);
        let start = room - len;
        buf.copy_within(..len, start);
        if sep == 1 {
            buf[room] = b'/';
        }
        if buf.get(start) == Some(&b'/') {
            let root = pins.fs.root();
            pins.step(root);
        }
        spliced = Some(start);
        pos = 0;
    }
    if want_dir && !pins.fs.stat(pins.current).await?.file_type().is_dir() {
        return Err(ErrorKind::NotADirectory.into());
    }
    Ok(pins.keep())
}

/// Resolves the parent of `path` with `how` and returns it pinned, with the
/// last name.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub(super) async fn resolve_parent<'p, F: FileSystem + ?Sized>(
    fs: &mut F,
    path: &'p [u8],
    how: Resolve,
) -> FsResult<(NodeId, &'p Name), F::DeviceError> {
    let (parent, name) = split_parent(path)?;
    Ok((fs.resolve(parent, how).await?, name))
}

/// Writes all of `buf` to `node` at `offset`.
#[cfg(feature = "alloc")]
pub(super) async fn write_all_at<F: FileSystem + ?Sized>(
    fs: &mut F,
    node: NodeId,
    mut offset: u64,
    mut buf: &[u8],
) -> FsResult<(), F::DeviceError> {
    while !buf.is_empty() {
        let n = fs.write(node, offset, buf).await?;
        if n == 0 {
            return Err(ErrorKind::NoSpace.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "alloc")]
    #[test]
    fn split_parent_takes_the_last_name() {
        assert_eq!(
            split_parent(b"/a/b.txt").map(|(p, n)| (p, n.as_bytes())),
            Ok((&b"/a/"[..], &b"b.txt"[..]))
        );
        assert_eq!(
            split_parent(b"a//").map(|(p, n)| (p, n.as_bytes())),
            Ok((&b""[..], &b"a"[..]))
        );
        for bad in [&b""[..], b"/", b"//", b"/a/..", b"a/."] {
            assert_eq!(split_parent(bad).err(), Some(ErrorKind::InvalidInput));
        }
    }

    #[test]
    fn lexical_cancellation() {
        assert!(cancelled(components(b"..")));
        assert!(!cancelled(components(b"x/..")));
        assert!(cancelled(components(b"x/../..")));
        assert!(!cancelled(components(b"./x")));
    }
}
