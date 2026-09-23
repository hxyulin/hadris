use super::*;

io_transform! {

/// How a path becomes a node.
///
/// Chosen by type, never by a cargo feature, so two crates or two volumes in
/// one build can resolve differently. Write your own to add a jail or
/// case-insensitive lookup.
pub trait Resolver {
    /// Resolves `path` from the root of `fs` and pins the result.
    async fn resolve<D: FsDriver + ?Sized>(
        &self,
        fs: &mut D,
        path: &str,
    ) -> FsResult<NodeId, D::DeviceError>;
}

/// The default resolver: `..` removes the previous component of the path
/// text, so `/a/missing/../b` is `/b`.
///
/// Holds one pin at a time, allocates nothing and needs only `lookup`.
/// Symlinks are not followed: a link in the middle of a path gives
/// [`ErrorKind::NotADirectory`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Lexical;

impl Resolver for Lexical {
    async fn resolve<D: FsDriver + ?Sized>(
        &self,
        fs: &mut D,
        path: &str,
    ) -> FsResult<NodeId, D::DeviceError> {
        let mut current: Option<NodeId> = None;
        let mut components = VPath::new(path).components();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                continue;
            };
            if cancelled(components.clone()) {
                continue;
            }
            let dir = current.unwrap_or(fs.root());
            let found = match Name::new(name) {
                Ok(name) => fs.lookup(dir, name).await,
                Err(err) => Err(err.into()),
            };
            if let Some(prev) = current.take() {
                fs.forget(prev);
            }
            current = Some(found?);
        }
        Ok(current.unwrap_or(fs.root()))
    }
}

/// POSIX resolution: every component must exist, `..` goes to the real
/// parent through [`FsDriver::parent`], and symlinks are followed.
///
/// At most 40 links are followed, then [`ErrorKind::Symlink`]. Allocates
/// nothing: link targets are spliced into an `N`-byte buffer on the stack,
/// and a path that outgrows it fails with [`ErrorKind::LimitExceeded`], like
/// `ENAMETOOLONG`. Paths without symlinks never touch the buffer.
#[derive(Debug, Clone, Copy)]
pub struct Posix<const N: usize = 1024>;

impl Posix {
    /// A resolver with a 1 KiB link buffer. `Posix::<256>` picks another size.
    pub const fn new() -> Self {
        Posix
    }
}

impl<const N: usize> Default for Posix<N> {
    fn default() -> Self {
        Posix
    }
}

impl<const N: usize> Resolver for Posix<N> {
    async fn resolve<D: FsDriver + ?Sized>(
        &self,
        fs: &mut D,
        path: &str,
    ) -> FsResult<NodeId, D::DeviceError> {
        let mut buf = [0u8; N];
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
                        fs.forget(current);
                        return Err(err);
                    }
                }
            };
        }
        macro_rules! fail {
            ($err:expr) => {{
                fs.forget(current);
                return Err($err.into());
            }};
        }
        loop {
            let pending = match spliced {
                Some(start) => &buf[start..],
                None => path.as_bytes(),
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
                let meta = check!(fs.node_metadata(current).await);
                if !meta.file_type().is_dir() {
                    fail!(ErrorKind::NotADirectory);
                }
                continue;
            }
            if component == b".." {
                let up = check!(fs.parent(current).await);
                fs.forget(current);
                current = up;
                continue;
            }
            let name = check!(Name::new(component).map_err(Error::from));
            let child = check!(fs.lookup(current, name).await);
            let meta = match fs.node_metadata(child).await {
                Ok(meta) => meta,
                Err(err) => {
                    fs.forget(child);
                    fail!(err)
                }
            };
            if meta.file_type() != FileType::Symlink {
                fs.forget(current);
                current = child;
                continue;
            }
            links += 1;
            if links > MAX_LINKS {
                fs.forget(child);
                fail!(ErrorKind::Symlink)
            }
            let sep = usize::from(rest_len > 0);
            let Some(room) = N.checked_sub(rest_len + sep) else {
                fs.forget(child);
                fail!(ErrorKind::LimitExceeded)
            };
            if spliced.is_none() {
                buf[N - rest_len..].copy_from_slice(&path.as_bytes()[end..]);
            }
            let read = fs.read_link(child, &mut buf[..room]).await;
            fs.forget(child);
            let len = check!(read);
            let start = room - len;
            buf.copy_within(..len, start);
            if sep == 1 {
                buf[room] = b'/';
            }
            if buf.get(start) == Some(&b'/') {
                fs.forget(current);
                current = fs.root();
            }
            spliced = Some(start);
            pos = 0;
        }
        if want_dir {
            let meta = check!(fs.node_metadata(current).await);
            if !meta.file_type().is_dir() {
                fail!(ErrorKind::NotADirectory);
            }
        }
        Ok(current)
    }
}

/// A driver whose paths resolve with `R` instead of [`Lexical`].
///
/// Every other call goes straight to the driver, and it dereferences to the
/// driver for format-specific methods. Put it inside a [`Volume`] to change
/// the policy for every handle on that volume.
#[derive(Debug)]
pub struct WithResolver<D, R> {
    driver: D,
    resolver: R,
}

impl<D, R> WithResolver<D, R> {
    /// Resolves `driver`'s paths with `resolver`.
    pub fn new(driver: D, resolver: R) -> Self {
        Self { driver, resolver }
    }

    /// Returns the driver.
    pub fn into_inner(self) -> D {
        self.driver
    }
}

impl<D, R> core::ops::Deref for WithResolver<D, R> {
    type Target = D;

    fn deref(&self) -> &D {
        &self.driver
    }
}

impl<D, R> core::ops::DerefMut for WithResolver<D, R> {
    fn deref_mut(&mut self) -> &mut D {
        &mut self.driver
    }
}

impl<D: FsDriver, R: Resolver> FsDriver for WithResolver<D, R> {
    type DeviceError = D::DeviceError;
    forward_driver_methods!();
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        self.resolver.resolve(&mut self.driver, path).await
    }
}

}

/// Links [`Posix`] follows before failing with [`ErrorKind::Symlink`].
const MAX_LINKS: u32 = 40;

/// Whether a later `..` removes the component just before `rest`.
fn cancelled(rest: Components<'_>) -> bool {
    let mut depth = 0usize;
    for component in rest {
        match component {
            Component::Normal(_) => depth += 1,
            Component::Parent if depth == 0 => return true,
            Component::Parent => depth -= 1,
            _ => {}
        }
    }
    false
}
