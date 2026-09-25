use super::*;

/// Links `Follow` and `NoFollow` follow before failing with
/// [`ErrorKind::Symlink`].
const MAX_LINKS: u32 = 40;

/// Bytes of path plus link text `Follow` and `NoFollow` hold.
const LINK_BUFFER: usize = 1024;

/// The components of `path`, skipping empty ones.
fn components(path: &[u8]) -> impl Iterator<Item = &[u8]> + Clone {
    path.split(|&b| b == b'/').filter(|c| !c.is_empty())
}

/// Whether a later `..` in `rest` removes the component just before it.
fn cancelled<'a>(rest: impl Iterator<Item = &'a [u8]>) -> bool {
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
    let mut current: Option<NodeId> = None;
    let mut rest = components(path);
    while let Some(component) = rest.next() {
        if matches!(component, b"." | b"..") || cancelled(rest.clone()) {
            continue;
        }
        let dir = current.unwrap_or(fs.root());
        let found = fs.lookup(dir, Name::new(component)).await;
        if let Some(prev) = current.take() {
            fs.forget(prev, 1);
        }
        current = Some(found?);
    }
    Ok(current.unwrap_or(fs.root()))
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
    let mut current = fs.root();
    let mut want_dir = false;
    macro_rules! check {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(err) => {
                    fs.forget(current, 1);
                    return Err(err);
                }
            }
        };
    }
    macro_rules! fail {
        ($err:expr) => {{
            fs.forget(current, 1);
            return Err($err.into());
        }};
    }
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
            let meta = check!(fs.stat(current).await);
            if !meta.file_type().is_dir() {
                fail!(ErrorKind::NotADirectory);
            }
            continue;
        }
        if component == b".." {
            let up = check!(fs.parent(current).await);
            fs.forget(current, 1);
            current = up;
            continue;
        }
        let child = check!(fs.lookup(current, Name::new(component)).await);
        let meta = match fs.stat(child).await {
            Ok(meta) => meta,
            Err(err) => {
                fs.forget(child, 1);
                fail!(err)
            }
        };
        if meta.file_type() != FileType::Symlink || (rest_len == 0 && !follow_last) {
            fs.forget(current, 1);
            current = child;
            continue;
        }
        links += 1;
        if links > MAX_LINKS {
            fs.forget(child, 1);
            fail!(ErrorKind::Symlink)
        }
        let sep = usize::from(rest_len > 0);
        let Some(room) = LINK_BUFFER.checked_sub(rest_len + sep) else {
            fs.forget(child, 1);
            fail!(ErrorKind::LimitExceeded)
        };
        if spliced.is_none() {
            buf[LINK_BUFFER - rest_len..].copy_from_slice(&path[end..]);
        }
        let read = fs.readlink(child, &mut buf[..room]).await.map(|target| target.len());
        fs.forget(child, 1);
        let len = check!(read);
        let start = room - len;
        buf.copy_within(..len, start);
        if sep == 1 {
            buf[room] = b'/';
        }
        if buf.get(start) == Some(&b'/') {
            fs.forget(current, 1);
            current = fs.root();
        }
        spliced = Some(start);
        pos = 0;
    }
    if want_dir {
        let meta = check!(fs.stat(current).await);
        if !meta.file_type().is_dir() {
            fail!(ErrorKind::NotADirectory);
        }
    }
    Ok(current)
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

/// Creates the directory `path` and every missing parent, resolving `..`
/// as `how` does.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub(super) async fn create_dir_all<F: FileSystem + ?Sized>(
    fs: &mut F,
    path: &[u8],
    how: Resolve,
) -> FsResult<(), F::DeviceError> {
    let mut current = fs.root();
    let mut rest = components(path);
    let mut result = Ok(());
    while let Some(component) = rest.next() {
        if component == b"." {
            continue;
        }
        let next = if component == b".." {
            if how == Resolve::Lexical {
                continue;
            }
            fs.parent(current).await
        } else if how == Resolve::Lexical && cancelled(rest.clone()) {
            continue;
        } else {
            let name = Name::new(component);
            match fs.lookup(current, name).await {
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    fs.mkdir(current, name, &SetAttr::new()).await
                }
                other => other,
            }
        };
        fs.forget(current, 1);
        current = next?;
        match fs.stat(current).await {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                result = Err(ErrorKind::NotADirectory.into());
                break;
            }
            Err(err) => {
                result = Err(err);
                break;
            }
        }
    }
    fs.forget(current, 1);
    result
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
