sync_only! {

use super::copy::{CHUNK, link_buffer, target_child, write_all_at};
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

/// Whether Windows gives `name` a meaning other than a plain file in the
/// directory: a device name such as `CON` or `com1.txt`, a character that
/// names a stream or a wildcard, or a trailing dot or space it strips.
#[cfg(any(windows, test))]
fn windows_special(name: &str) -> bool {
    const DEVICES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if name.ends_with(['.', ' '])
        || name.chars().any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return true;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    if DEVICES.iter().any(|device| stem.eq_ignore_ascii_case(device)) {
        return true;
    }
    let mut chars = stem.chars();
    let prefix: alloc::string::String = chars.by_ref().take(3).collect();
    let digit = chars.next();
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(digit, Some('1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'))
        && chars.next().is_none()
}

/// `bytes` as a host file name, if it is exactly one normal component. On
/// Windows, device names and names Windows would alter are refused too.
fn host_name(bytes: &[u8]) -> io::Result<&OsStr> {
    #[cfg(unix)]
    let name = std::os::unix::ffi::OsStrExt::from_bytes(bytes);
    #[cfg(not(unix))]
    let name = OsStr::new(core::str::from_utf8(bytes).map_err(|_| escape_error())?);
    #[cfg(windows)]
    if name.to_str().is_some_and(windows_special) {
        return Err(escape_error());
    }
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

fn symlink_in_the_way() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "refusing to replace a symlink")
}

/// Makes room for a new non-directory at `path`: an existing file is
/// unlinked, so its other hard links keep their contents. Fails on an
/// existing symlink or directory.
fn clear_host_path(path: &Path) -> io::Result<()> {
    match host::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(symlink_in_the_way()),
        Ok(meta) if meta.is_dir() => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Ok(_) => host::remove_file(path),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Creates a new file at `path`. `create_new` never follows a symlink, so
/// one planted after [`clear_host_path`] makes this fail instead.
fn create_host_file(path: &Path) -> io::Result<host::File> {
    clear_host_path(path)?;
    host::OpenOptions::new().write(true).create_new(true).open(path)
}

/// Creates the directory `path`, or accepts an existing directory that is
/// not a symlink.
fn create_host_dir(path: &Path) -> io::Result<()> {
    match host::create_dir(path) {
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let kind = host::symlink_metadata(path)?.file_type();
            if kind.is_dir() {
                Ok(())
            } else if kind.is_symlink() {
                Err(symlink_in_the_way())
            } else {
                Err(err)
            }
        }
        other => other,
    }
}

/// Writes the pinned non-directory `node` to the host path `path`.
fn extract_node<D: FsDriver + ?Sized>(fs: &mut D, node: NodeId, meta: &Metadata, path: &Path) -> io::Result<()> {
    match meta.file_type() {
        FileType::File => {
            let mut file = create_host_file(path)?;
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
            let mut target = link_buffer(meta.len()).map_err(Error::<D::DeviceError>::from)?;
            let n = fs.read_link(node, &mut target)?;
            #[cfg(unix)]
            {
                clear_host_path(path)?;
                std::os::unix::fs::symlink(
                    <OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&target[..n]),
                    path,
                )
            }
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
/// created with its parents when missing. An existing file is replaced by a
/// new one rather than truncated, so other hard links to it keep their
/// contents.
///
/// Every entry name must be one plain host path component: names that are
/// empty, `.`, `..`, absolute, or contain a separator or a drive prefix fail
/// with [`std::io::ErrorKind::InvalidData`], and so do, on Windows, device
/// names such as `CON` or `com1.txt`, names containing `:` or a wildcard, and
/// names ending in a dot or space. An existing symlink in the way fails with
/// [`std::io::ErrorKind::AlreadyExists`] instead of being followed or
/// replaced, and files are created with `create_new`, which never follows a
/// link. The contents of an image therefore cannot direct a write outside
/// `host`; another process that swaps a directory below `host` for a symlink
/// while the extraction runs still can, as with `tar`.
///
/// File modification times and, on Unix, permission bits (without set-id or
/// sticky bits) are applied. Symlinks are created on Unix only, and a target
/// longer than 4096 bytes fails with [`std::io::ErrorKind::Other`] carrying
/// [`ErrorKind::LimitExceeded`]; device nodes, FIFOs and sockets fail with
/// [`std::io::ErrorKind::Unsupported`].
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

/// The entries of the host directory `path`, sorted by name bytes.
fn sorted_entries(path: &Path) -> io::Result<alloc::vec::IntoIter<host::DirEntry>> {
    let mut entries = host::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_cached_key(host::DirEntry::file_name);
    Ok(entries.into_iter())
}

fn import_walk<D: FsDriver + ?Sized>(
    fs: &mut D,
    stack: &mut Vec<(alloc::vec::IntoIter<host::DirEntry>, NodeId)>,
) -> io::Result<()> {
    while let Some((entries, dir)) = stack.last_mut() {
        let dir = *dir;
        let Some(entry) = entries.next() else {
            if let Some((_, done)) = stack.pop() {
                fs.forget(done);
            }
            continue;
        };
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
        match sorted_entries(&path) {
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
/// with its parents when missing. The entries of each directory are copied
/// in order of their name bytes, whatever order the host lists them in, so
/// the same tree always gives the same image. Existing files are overwritten; an existing
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
    let entries = match sorted_entries(host) {
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
    fn windows_special_names() {
        for name in ["boot.cfg", "CONFIG", "console.log", "COM10", "LPT0", "comx", "a.b.c", "NULL.txt"] {
            assert!(!windows_special(name), "{name}");
        }
        for name in [
            "CON", "con", "Con.txt", "nul.tar.gz", "AUX ", "aux .c", "PRN", "COM1", "com9.log", "LPT3",
            "lpt\u{b9}", "CONIN$", "conout$.x", "a:b", "stream::$DATA", "a?", "a*", "a<b", "a|b",
            "a\"b", "tab\t", "dot.", "space ",
        ] {
            assert!(windows_special(name), "{name}");
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
