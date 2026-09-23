sync_only! {

use super::copy::{CHUNK, target_child, write_all_at};
use super::*;
use crate::{DateTime, FileTimes, Mode};
use alloc::vec::Vec;
use std::ffi::OsStr;
use std::fs as host;
use std::io::{self, Read as _, Write as _};
use std::path::{Component as HostComponent, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Permission bits carried between images and the host. Set-id and sticky
/// bits from an image are never applied.
const PERMISSION_BITS: u32 = 0o777;

fn escape_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "entry name would leave the target directory")
}

/// `bytes` as a host file name, if it is exactly one normal component.
fn host_name(bytes: &[u8]) -> io::Result<&OsStr> {
    #[cfg(unix)]
    let name = std::os::unix::ffi::OsStrExt::from_bytes(bytes);
    #[cfg(not(unix))]
    let name = OsStr::new(core::str::from_utf8(bytes).map_err(|_| escape_error())?);
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(HostComponent::Normal(part)), None) if part == name => Ok(name),
        _ => Err(escape_error()),
    }
}

/// `name` as bytes for a [`Name`] or a symlink target.
fn os_bytes(name: &OsStr) -> io::Result<&[u8]> {
    #[cfg(unix)]
    return Ok(std::os::unix::ffi::OsStrExt::as_bytes(name));
    #[cfg(not(unix))]
    name.to_str().map(str::as_bytes).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "host name is not valid UTF-8")
    })
}

fn to_system_time(time: DateTime) -> Option<SystemTime> {
    let seconds = Duration::from_secs(time.unix_seconds().unsigned_abs());
    let base = if time.unix_seconds() >= 0 {
        UNIX_EPOCH.checked_add(seconds)
    } else {
        UNIX_EPOCH.checked_sub(seconds)
    };
    base?.checked_add(Duration::from_nanos(time.nanoseconds().into()))
}

fn from_system_time(time: SystemTime) -> Option<DateTime> {
    let (seconds, nanoseconds) = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => (i64::try_from(after.as_secs()).ok()?, after.subsec_nanos()),
        Err(before) => {
            let before = before.duration();
            let seconds = -i64::try_from(before.as_secs()).ok()?;
            match before.subsec_nanos() {
                0 => (seconds, 0),
                nanos => (seconds - 1, 1_000_000_000 - nanos),
            }
        }
    };
    DateTime::new(seconds, nanoseconds).ok()
}

/// Fails when `path` is a symlink, so a write never follows one out of the
/// target directory.
fn refuse_symlink(path: &Path) -> io::Result<()> {
    match host::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "refusing to write through a symlink",
        )),
        _ => Ok(()),
    }
}

/// Creates the directory `path`, or accepts an existing directory that is
/// not a symlink.
fn create_host_dir(path: &Path) -> io::Result<()> {
    match host::create_dir(path) {
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if host::symlink_metadata(path)?.file_type().is_dir() {
                Ok(())
            } else {
                Err(err)
            }
        }
        other => other,
    }
}

/// Writes the pinned non-directory `node` to the host path `path`.
fn extract_node<D: FsDriver + ?Sized>(fs: &mut D, node: NodeId, meta: &Metadata, path: &Path) -> io::Result<()> {
    refuse_symlink(path)?;
    match meta.file_type() {
        FileType::File => {
            let mut file = host::File::create(path)?;
            let mut buf = [0u8; CHUNK];
            let mut offset = 0;
            loop {
                let n = fs.read_at(node, offset, &mut buf)?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])?;
                offset += n as u64;
            }
            if let Some(time) = meta.times().modified().and_then(to_system_time) {
                file.set_modified(time)?;
            }
            #[cfg(unix)]
            if let Some(mode) = meta.permissions() {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(host::Permissions::from_mode(mode.bits() & PERMISSION_BITS))?;
            }
            Ok(())
        }
        FileType::Symlink => {
            let len = usize::try_from(meta.len()).map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
            let mut target = alloc::vec![0u8; if len == 0 { CHUNK } else { len }];
            let n = fs.read_link(node, &mut target)?;
            #[cfg(unix)]
            return std::os::unix::fs::symlink(
                <OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&target[..n]),
                path,
            );
            #[cfg(not(unix))]
            {
                let _ = n;
                Err(io::Error::new(io::ErrorKind::Unsupported, "symlinks are extracted on Unix only"))
            }
        }
        _ => Err(io::Error::new(io::ErrorKind::Unsupported, "cannot extract device nodes, FIFOs or sockets")),
    }
}

fn extract_walk<D: FsDriver + ?Sized>(fs: &mut D, stack: &mut Vec<(NodeId, DirCursor, PathBuf)>) -> io::Result<()> {
    let mut name = NameBuf::new();
    while let Some((dir, cursor, path)) = stack.last_mut() {
        let dir = *dir;
        if fs.read_dir_entry(dir, cursor, &mut name)?.is_none() {
            if let Some((done, ..)) = stack.pop() {
                fs.forget(done);
            }
            continue;
        }
        let target = path.join(host_name(name.as_bytes())?);
        let child_name = name.as_name().ok_or(ErrorKind::Corrupt).map_err(Error::<D::DeviceError>::from)?;
        let child = fs.lookup(dir, child_name)?;
        let result = match fs.node_metadata(child) {
            Ok(meta) if meta.file_type().is_dir() => match create_host_dir(&target) {
                Ok(()) => {
                    stack.push((child, DirCursor::start(), target));
                    continue;
                }
                Err(err) => Err(err),
            },
            Ok(meta) => extract_node(fs, child, &meta, &target),
            Err(err) => Err(err.into()),
        };
        fs.forget(child);
        result?;
    }
    Ok(())
}

/// Copies the file, symlink or directory tree at `from` on `src` to the host
/// path `host`, as `tar -x` or `7z x` would.
///
/// `src` is any [`Access`]. A directory is merged into `host`, which is
/// created with its parents when missing; existing files are overwritten.
/// Every entry name must be one plain host path component: names that are
/// absolute, contain a separator or a drive prefix, or are `..` fail with
/// [`std::io::ErrorKind::InvalidData`], and an existing symlink in the way
/// fails with [`std::io::ErrorKind::AlreadyExists`] instead of being followed,
/// so an image cannot write outside `host`. File modification times and, on
/// Unix, permission bits (without set-id or sticky bits) are applied.
/// Symlinks are created on Unix only; device nodes, FIFOs and sockets fail
/// with [`std::io::ErrorKind::Unsupported`].
///
/// Only in the sync API: the host side is blocking `std::fs`.
pub fn extract_to_host<S: Access>(src: S, from: &str, host: impl AsRef<Path>) -> io::Result<()> {
    let host = host.as_ref();
    let mut fs = src.into_driver();
    let node = fs.resolve(from)?;
    let meta = match fs.node_metadata(node) {
        Ok(meta) => meta,
        Err(err) => {
            fs.forget(node);
            return Err(err.into());
        }
    };
    if !meta.file_type().is_dir() {
        let result = extract_node(&mut fs, node, &meta, host);
        fs.forget(node);
        return result;
    }
    if let Err(err) = host::create_dir_all(host) {
        fs.forget(node);
        return Err(err);
    }
    let mut stack = Vec::new();
    stack.push((node, DirCursor::start(), host.to_path_buf()));
    let result = extract_walk(&mut fs, &mut stack);
    for (dir, ..) in stack {
        fs.forget(dir);
    }
    result
}

/// Copies the host file at `path` into the pinned `node`.
fn import_file<D: FsDriver + ?Sized>(fs: &mut D, node: NodeId, path: &Path, meta: &host::Metadata) -> io::Result<()> {
    let mut file = host::File::open(path)?;
    let mut buf = [0u8; CHUNK];
    let mut offset = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        write_all_at(fs, node, offset, &buf[..n])?;
        offset += n as u64;
    }
    let times = FileTimes::new().with_modified(meta.modified().ok().and_then(from_system_time));
    #[cfg(unix)]
    let mode = Some(Mode::new(std::os::unix::fs::PermissionsExt::mode(&meta.permissions())));
    #[cfg(not(unix))]
    let mode: Option<Mode> = None;
    fs.set_metadata(node, &SetMetadata::new().with_times(times).with_mode(mode))?;
    fs.sync_node(node)?;
    Ok(())
}

/// Copies the host non-directory at `path` to `name` in `dir`.
fn import_node<D: FsDriver + ?Sized>(
    fs: &mut D,
    dir: NodeId,
    name: &Name,
    path: &Path,
    meta: &host::Metadata,
) -> io::Result<()> {
    if meta.file_type().is_symlink() {
        let target = host::read_link(path)?;
        let node = target_child(fs, dir, name, NewNode::Symlink(os_bytes(target.as_os_str())?))?;
        fs.forget(node);
        return Ok(());
    }
    if !meta.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "cannot import device nodes, FIFOs or sockets"));
    }
    let node = target_child(fs, dir, name, NewNode::File)?;
    let result = import_file(fs, node, path, meta);
    fs.forget(node);
    result
}

fn import_walk<D: FsDriver + ?Sized>(fs: &mut D, stack: &mut Vec<(host::ReadDir, NodeId)>) -> io::Result<()> {
    while let Some((entries, dir)) = stack.last_mut() {
        let dir = *dir;
        let Some(entry) = entries.next() else {
            if let Some((_, done)) = stack.pop() {
                fs.forget(done);
            }
            continue;
        };
        let entry = entry?;
        let file_name = entry.file_name();
        let name = Name::new(os_bytes(&file_name)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let path = entry.path();
        let meta = host::symlink_metadata(&path)?;
        if !meta.file_type().is_dir() {
            import_node(fs, dir, name, &path, &meta)?;
            continue;
        }
        let node = target_child(fs, dir, name, NewNode::Dir)?;
        match host::read_dir(&path) {
            Ok(entries) => stack.push((entries, node)),
            Err(err) => {
                fs.forget(node);
                return Err(err);
            }
        }
    }
    Ok(())
}

/// Copies the host file, symlink or directory tree at `host` to `to` on
/// `dst`, as `mkisofs` or `mcopy -s` would.
///
/// `dst` is any [`Access`]. A directory is merged into `to`, which is created
/// with its parents when missing. Existing files are overwritten; an existing
/// node of another type, or an existing symlink, fails with
/// [`std::io::ErrorKind::AlreadyExists`]. File modification times and, on
/// Unix, permissions are copied where `dst` can store them. Symlinks are
/// copied as links, never followed; device nodes, FIFOs and sockets fail
/// with [`std::io::ErrorKind::Unsupported`]. Host names that are not valid
/// [`Name`]s, or not UTF-8 outside Unix, fail with
/// [`std::io::ErrorKind::InvalidData`].
///
/// Only in the sync API: the host side is blocking `std::fs`.
pub fn import_from_host<T: Access>(host: impl AsRef<Path>, dst: T, to: &str) -> io::Result<()> {
    let host = host.as_ref();
    let mut fs = dst.into_driver();
    let meta = host::symlink_metadata(host)?;
    if !meta.file_type().is_dir() {
        let (dir, name) = resolve_parent(&mut fs, to)?;
        let result = import_node(&mut fs, dir, name, host, &meta);
        fs.forget(dir);
        return result;
    }
    create_dir_all(&mut fs, to)?;
    let top = fs.resolve(to)?;
    let entries = match host::read_dir(host) {
        Ok(entries) => entries,
        Err(err) => {
            fs.forget(top);
            return Err(err);
        }
    };
    let mut stack = Vec::new();
    stack.push((entries, top));
    let result = import_walk(&mut fs, &mut stack);
    for (_, dir) in stack {
        fs.forget(dir);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_are_single_components() {
        assert!(host_name(b"boot.cfg").is_ok());
        for bad in [&b""[..], b".", b"..", b"a/b", b"/etc", b"a/"] {
            assert_eq!(host_name(bad).unwrap_err().kind(), io::ErrorKind::InvalidData, "{bad:?}");
        }
        #[cfg(windows)]
        for bad in [&b"a\\b"[..], b"C:", b"C:x", b"\\\\server"] {
            assert_eq!(host_name(bad).unwrap_err().kind(), io::ErrorKind::InvalidData, "{bad:?}");
        }
    }

    #[test]
    fn times_round_trip() {
        for (seconds, nanos) in [(0, 0), (1_709_164_800, 5), (-1, 999_999_999), (-86_400, 0)] {
            let time = DateTime::new(seconds, nanos).unwrap();
            assert_eq!(from_system_time(to_system_time(time).unwrap()), Some(time));
        }
    }
}

}
