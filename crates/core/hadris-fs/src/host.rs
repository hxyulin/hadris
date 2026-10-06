//! The host tier (`std`, sync only): trees from and to host directories,
//! host files as tree content, and the host's defaults for mounting and
//! reproducible builds.
//!
//! Errors are [`PathError`]s that carry the tree path and the host path.
//!
//! ```rust,no_run
//! use hadris_fs::host::{self, TreeOptions};
//!
//! let options = match host::source_date_epoch()? {
//!     Some(epoch) => TreeOptions::new().with_clamp(epoch),
//!     None => TreeOptions::new(),
//! };
//! let (tree, skipped) = host::read_tree("rootfs", &options)?;
//! assert!(skipped.is_empty());
//! let report = host::write_tree("copy-of-rootfs", &tree)?;
//! for warning in report.warnings() {
//!     eprintln!("{warning}");
//! }
//! # Ok::<(), hadris_fs::PathError>(())
//! ```

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::fmt;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use crate::DeviceNumber;
use crate::sync::ContentReader;
use crate::{
    Attributes, Content, DateTime, Error, ErrorKind, Field, FileType, MountOptions, Node, Owner,
    PathError, Report, SetAttr, SystemClock, Tree, TreeEntry, Warning, WarningKind,
};

/// What [`read_tree`] makes of a host symlink.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Symlinks {
    /// A symlink node with the target as stored.
    #[default]
    Keep,
    /// The node the link points at. A dangling link or a loop is an entry
    /// error.
    Follow,
    /// Left out, with no error.
    Skip,
    /// An entry error naming the link.
    Fail,
}

/// What [`read_tree`] does with an entry it cannot read: permission
/// denied, vanished, a name the host cannot give as bytes, a loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OnError {
    /// Stop with the error.
    #[default]
    Fail,
    /// Leave the entry out and return its error with the tree.
    Skip,
}

type Exclude = Box<dyn Fn(&Path) -> bool + Send + Sync>;

/// Options for [`read_tree`].
#[derive(Default)]
pub struct TreeOptions {
    symlinks: Symlinks,
    on_error: OnError,
    exclude: Option<Exclude>,
    owner: Option<Owner>,
    clamp: Option<DateTime>,
}

impl TreeOptions {
    /// Keep symlinks, fail on the first unreadable entry, exclude nothing,
    /// keep owners and times.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets what a host symlink becomes.
    #[must_use]
    pub fn with_symlinks(self, symlinks: Symlinks) -> Self {
        Self { symlinks, ..self }
    }

    /// Sets what an unreadable entry does.
    #[must_use]
    pub fn with_on_error(self, on_error: OnError) -> Self {
        Self { on_error, ..self }
    }

    /// Leaves out every host path for which `filter` is true, with the whole
    /// subtree of a directory. The filter sees the path as walked: the root
    /// joined with the relative path.
    #[must_use]
    pub fn with_exclude(self, filter: impl Fn(&Path) -> bool + Send + Sync + 'static) -> Self {
        Self {
            exclude: Some(Box::new(filter)),
            ..self
        }
    }

    /// Gives every node `owner` instead of the host's, as `cpio -R` or
    /// `mkisofs -uid` do.
    #[must_use]
    pub fn with_owner(self, owner: Owner) -> Self {
        Self {
            owner: Some(owner),
            ..self
        }
    }

    /// Sets every time later than `limit` to `limit`, the clamping
    /// `SOURCE_DATE_EPOCH` asks for.
    #[must_use]
    pub fn with_clamp(self, limit: DateTime) -> Self {
        Self {
            clamp: Some(limit),
            ..self
        }
    }

    /// What a host symlink becomes.
    pub fn symlinks(&self) -> Symlinks {
        self.symlinks
    }

    /// What an unreadable entry does.
    pub fn on_error(&self) -> OnError {
        self.on_error
    }

    /// The owner every node gets, if set.
    pub fn owner(&self) -> Option<Owner> {
        self.owner
    }

    /// The latest time a node keeps, if set.
    pub fn clamp(&self) -> Option<DateTime> {
        self.clamp
    }
}

impl fmt::Debug for TreeOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeOptions")
            .field("symlinks", &self.symlinks)
            .field("on_error", &self.on_error)
            .field("exclude", &self.exclude.is_some())
            .field("owner", &self.owner)
            .field("clamp", &self.clamp)
            .finish()
    }
}

fn host_error(err: io::Error, message: &'static str, host: &Path) -> PathError {
    let kind = match err.kind() {
        io::ErrorKind::NotFound => Some(ErrorKind::NotFound),
        io::ErrorKind::AlreadyExists => Some(ErrorKind::AlreadyExists),
        io::ErrorKind::NotADirectory => Some(ErrorKind::NotADirectory),
        io::ErrorKind::IsADirectory => Some(ErrorKind::IsADirectory),
        io::ErrorKind::DirectoryNotEmpty => Some(ErrorKind::DirectoryNotEmpty),
        _ => None,
    };
    let err = PathError::from(Error::device(err, message)).with_host_path(host);
    match kind {
        Some(kind) => err.with_kind(kind),
        None => err,
    }
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

/// `name` as bytes for a tree path.
fn name_bytes(name: &OsStr) -> Option<&[u8]> {
    #[cfg(unix)]
    return Some(std::os::unix::ffi::OsStrExt::as_bytes(name));
    #[cfg(not(unix))]
    name.to_str().map(str::as_bytes)
}

/// The host file `path` as tree content, read when a sync writer needs it.
/// Its length is taken now, so planning does no I/O; a file whose length
/// changed by then fails the write with both paths. A disk device such as
/// `/dev/sdb` or `\\.\PhysicalDrive1` gives its whole contents.
pub fn file(path: impl AsRef<Path>) -> Result<Content, PathError> {
    let path = path.as_ref();
    let file =
        fs::File::open(path).map_err(|err| host_error(err, "cannot open host file", path))?;
    if file.metadata().is_ok_and(|meta| meta.is_dir()) {
        return Err(
            PathError::new(ErrorKind::IsADirectory, "tree content must be a file")
                .with_host_path(path),
        );
    }
    let len = hadris_storage::host::file_len(&file)
        .map_err(|err| host_error(err, "cannot measure host file", path))?;
    Ok(Content::host(path.to_path_buf(), len))
}

/// `SOURCE_DATE_EPOCH` from the environment, the time reproducible builds
/// pass to `with_time` and [`TreeOptions::with_clamp`]. `None` when it is
/// not set; [`ErrorKind::InvalidInput`] when it is not a decimal number of
/// seconds.
pub fn source_date_epoch() -> Result<Option<DateTime>, PathError> {
    let Some(value) = std::env::var_os("SOURCE_DATE_EPOCH") else {
        return Ok(None);
    };
    let bad = || {
        PathError::new(
            ErrorKind::InvalidInput,
            "SOURCE_DATE_EPOCH is not a decimal number of seconds",
        )
    };
    let text = value.to_str().ok_or_else(bad)?;
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad());
    }
    let seconds: i64 = text.parse().map_err(|_| bad())?;
    DateTime::from_unix_seconds(seconds)
        .map(Some)
        .map_err(|_| bad())
}

/// The host's defaults for mounting: [`MountOptions::new`] with
/// [`SystemClock`] and the host's current UTC offset for FAT times, as
/// Windows and Linux vfat assume. Each stays overridable with the `with_`
/// methods.
pub fn mount_options() -> MountOptions {
    static CLOCK: SystemClock = SystemClock;
    let options = MountOptions::new().with_clock(&CLOCK);
    options
        .with_utc_offset(local_utc_offset())
        .unwrap_or(options)
}

/// The host's current offset from UTC in minutes, east positive. 0 when the
/// host cannot tell.
pub fn local_utc_offset() -> i16 {
    zone::offset_minutes().clamp(-24 * 60, 24 * 60)
}

#[cfg(all(
    unix,
    any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    )
))]
mod zone {
    pub(super) fn offset_minutes() -> i16 {
        let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
            return 0;
        };
        let Ok(now) = libc::time_t::try_from(now.as_secs()) else {
            return 0;
        };
        let mut tm = core::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: `localtime_r` reads `now` and fills `tm`, both valid for
        // the call; it returns null without writing when it fails.
        let filled = unsafe { libc::localtime_r(&now, tm.as_mut_ptr()) };
        if filled.is_null() {
            return 0;
        }
        // SAFETY: `localtime_r` succeeded, so it initialized `tm`.
        let tm = unsafe { tm.assume_init() };
        i16::try_from(tm.tm_gmtoff / 60).unwrap_or(0)
    }
}

#[cfg(windows)]
mod zone {
    #[repr(C)]
    struct SystemTime([u16; 8]);

    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTimeZoneInformation(info: *mut TimeZoneInformation) -> u32;
    }

    pub(super) fn offset_minutes() -> i16 {
        let mut info = TimeZoneInformation {
            bias: 0,
            standard_name: [0; 32],
            standard_date: SystemTime([0; 8]),
            standard_bias: 0,
            daylight_name: [0; 32],
            daylight_date: SystemTime([0; 8]),
            daylight_bias: 0,
        };
        // SAFETY: `info` is a valid, writable TIME_ZONE_INFORMATION.
        let id = unsafe { GetTimeZoneInformation(&mut info) };
        let bias = match id {
            1 => info.bias + info.standard_bias,
            2 => info.bias + info.daylight_bias,
            u32::MAX => return 0,
            _ => info.bias,
        };
        i16::try_from(-bias).unwrap_or(0)
    }
}

#[cfg(not(any(
    windows,
    all(
        unix,
        any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly",
        )
    )
)))]
mod zone {
    pub(super) fn offset_minutes() -> i16 {
        0
    }
}

// ---------------------------------------------------------------------------
// Host directory to tree.

/// Directories [`read_tree`] and [`write_tree`] descend before
/// [`ErrorKind::LimitExceeded`].
const MAX_DEPTH: usize = 1024;

struct Reader<'o> {
    tree: Tree,
    options: &'o TreeOptions,
    skipped: Vec<PathError>,
    #[cfg(unix)]
    inodes: BTreeMap<(u64, u64), Vec<u8>>,
    #[cfg(unix)]
    dirs: BTreeMap<(u64, u64), ()>,
}

enum Kind {
    Dir,
    File(u64),
    Symlink(Vec<u8>),
    #[cfg(unix)]
    Special(FileType, Option<DeviceNumber>),
}

#[cfg(unix)]
fn device_number(rdev: u64) -> DeviceNumber {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let (major, minor) = (
        ((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff),
        (rdev & 0xff) | ((rdev >> 12) & !0xff),
    );
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let (major, minor) = ((rdev >> 24) & 0xff, rdev & 0xff_ffff);
    DeviceNumber::new(major as u32, minor as u32)
}

impl Reader<'_> {
    fn attrs(&self, meta: &fs::Metadata) -> SetAttr {
        let clamp = |time: Option<DateTime>| match (time, self.options.clamp) {
            (Some(time), Some(limit))
                if (time.unix_seconds(), time.nanoseconds())
                    > (limit.unix_seconds(), limit.nanoseconds()) =>
            {
                Some(limit)
            }
            (time, _) => time,
        };
        let mut attrs = SetAttr::new();
        if let Some(time) = clamp(meta.modified().ok().and_then(from_system_time)) {
            attrs = attrs.with_modified(time);
        }
        if let Some(time) = clamp(meta.accessed().ok().and_then(from_system_time)) {
            attrs = attrs.with_accessed(time);
        }
        if let Some(time) = clamp(meta.created().ok().and_then(from_system_time)) {
            attrs = attrs.with_created(time);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            attrs = attrs
                .with_permissions(crate::Permissions::new(meta.mode()))
                .with_owner(Owner::new(meta.uid(), meta.gid()));
        }
        #[cfg(not(unix))]
        if meta.permissions().readonly() {
            attrs = attrs.with_attributes(Attributes::READ_ONLY);
        }
        match self.options.owner {
            Some(owner) => attrs.with_owner(owner),
            None => attrs,
        }
    }

    fn kind(&self, host: &Path, meta: &fs::Metadata) -> Result<Kind, PathError> {
        let file_type = meta.file_type();
        if file_type.is_dir() {
            return Ok(Kind::Dir);
        }
        if file_type.is_file() {
            return Ok(Kind::File(meta.len()));
        }
        if file_type.is_symlink() {
            let target =
                fs::read_link(host).map_err(|err| host_error(err, "cannot read symlink", host))?;
            #[cfg(unix)]
            let target = std::os::unix::ffi::OsStrExt::as_bytes(target.as_os_str()).to_vec();
            #[cfg(not(unix))]
            let target = target
                .to_str()
                .ok_or_else(|| {
                    PathError::new(ErrorKind::InvalidInput, "symlink target is not UTF-8")
                        .with_host_path(host)
                })?
                .replace('\\', "/")
                .into_bytes();
            return Ok(Kind::Symlink(target));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt};
            if file_type.is_char_device() {
                return Ok(Kind::Special(
                    FileType::CharDevice,
                    Some(device_number(meta.rdev())),
                ));
            }
            if file_type.is_block_device() {
                return Ok(Kind::Special(
                    FileType::BlockDevice,
                    Some(device_number(meta.rdev())),
                ));
            }
            if file_type.is_fifo() {
                return Ok(Kind::Special(FileType::Fifo, None));
            }
            if file_type.is_socket() {
                return Ok(Kind::Special(FileType::Socket, None));
            }
        }
        Err(PathError::new(ErrorKind::Unsupported, "unknown host file type").with_host_path(host))
    }

    /// Records an entry error: returns it under [`OnError::Fail`], keeps it
    /// otherwise.
    fn fail(&mut self, err: PathError) -> Result<(), PathError> {
        match self.options.on_error {
            OnError::Skip => {
                self.skipped.push(err);
                Ok(())
            }
            _ => Err(err),
        }
    }

    /// Adds the host entry `host` at `path`. Returns whether it is a
    /// directory to walk.
    fn add(&mut self, host: &Path, path: &[u8]) -> Result<bool, PathError> {
        let mut meta = fs::symlink_metadata(host)
            .map_err(|err| host_error(err, "cannot read host metadata", host))?;
        if meta.file_type().is_symlink() {
            match self.options.symlinks {
                Symlinks::Skip => return Ok(false),
                Symlinks::Fail => {
                    return Err(PathError::new(ErrorKind::Symlink, "symlinks are refused")
                        .with_host_path(host));
                }
                Symlinks::Follow => {
                    meta = fs::metadata(host)
                        .map_err(|err| host_error(err, "cannot follow symlink", host))?;
                }
                _ => {}
            }
        }
        let kind = self.kind(host, &meta)?;
        let attrs = self.attrs(&meta);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let key = (meta.dev(), meta.ino());
            match kind {
                Kind::Dir if self.dirs.insert(key, ()).is_some() => {
                    return Err(PathError::new(
                        ErrorKind::Symlink,
                        "symlinks lead back to a directory already read",
                    )
                    .with_host_path(host));
                }
                Kind::File(_) if meta.nlink() > 1 => {
                    if let Some(first) = self.inodes.get(&key) {
                        let first = first.clone();
                        self.tree.link(first, path)?;
                        return Ok(false);
                    }
                    self.inodes.insert(key, path.to_vec());
                }
                _ => {}
            }
        }
        let node = match kind {
            Kind::Dir => Node::dir(),
            Kind::File(len) => Node::file(Content::host(host.to_path_buf(), len)),
            Kind::Symlink(target) => Node::symlink(target),
            #[cfg(unix)]
            Kind::Special(file_type, device) => Node::special(file_type, device),
        };
        let dir = node.file_type() == FileType::Dir;
        self.tree.insert(path, node.with_attrs(attrs))?;
        Ok(dir)
    }

    fn walk(&mut self, root: &Path) -> Result<(), PathError> {
        let mut pending: Vec<(PathBuf, Vec<u8>, usize)> =
            alloc::vec![(root.to_path_buf(), Vec::new(), 0)];
        while let Some((dir, prefix, depth)) = pending.pop() {
            if depth >= MAX_DEPTH {
                let err = PathError::new(ErrorKind::LimitExceeded, "the tree is too deep")
                    .with_path(&prefix)
                    .with_host_path(&dir);
                self.fail(err)?;
                continue;
            }
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    self.fail(
                        host_error(err, "cannot list host directory", &dir).with_path(&prefix),
                    )?;
                    continue;
                }
            };
            let mut names = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => names.push(entry.file_name()),
                    Err(err) => self.fail(
                        host_error(err, "cannot list host directory", &dir).with_path(&prefix),
                    )?,
                }
            }
            names.sort_by(|a, b| name_bytes(a).cmp(&name_bytes(b)));
            let mut dirs = Vec::new();
            for name in names {
                let host = dir.join(&name);
                if self
                    .options
                    .exclude
                    .as_ref()
                    .is_some_and(|exclude| exclude(&host))
                {
                    continue;
                }
                let Some(bytes) = name_bytes(&name) else {
                    let err = PathError::new(ErrorKind::InvalidInput, "host name is not UTF-8")
                        .with_host_path(&host);
                    self.fail(err.with_path(&prefix))?;
                    continue;
                };
                let mut path = prefix.clone();
                path.push(b'/');
                path.extend_from_slice(bytes);
                match self.add(&host, &path) {
                    Ok(true) => dirs.push((host, path, depth + 1)),
                    Ok(false) => {}
                    Err(err) => self.fail(err.with_path(&path))?,
                }
            }
            pending.extend(dirs.into_iter().rev());
        }
        Ok(())
    }
}

/// A tree of the host directory `root`, as `mkisofs`, `mkfs.fat -d` or
/// `cpio -o` read one.
///
/// Names are the host's bytes (UTF-8 required outside Unix), host hard
/// links become tree hard links, and device nodes, FIFOs and sockets
/// become special nodes. Each node gets the host's modification, access
/// and creation times and, on Unix, its mode and owner, adjusted by
/// `options`. Directories are read in order of their name bytes, so the
/// same directory always gives the same tree. File content is only
/// measured here and read when a writer needs it; a file whose length
/// changed by then fails the write with both paths. `root`'s own
/// attributes go to the tree root.
///
/// With [`OnError::Skip`] the errors of the entries left out come back
/// with the tree; otherwise the first one fails the call. A directory more
/// than 1024 levels deep fails with [`ErrorKind::LimitExceeded`], and with
/// [`Symlinks::Follow`] a link back to a directory already read fails with
/// [`ErrorKind::Symlink`].
pub fn read_tree(
    root: impl AsRef<Path>,
    options: &TreeOptions,
) -> Result<(Tree, Vec<PathError>), PathError> {
    let root = root.as_ref();
    let meta =
        fs::metadata(root).map_err(|err| host_error(err, "cannot read host metadata", root))?;
    if !meta.is_dir() {
        return Err(PathError::new(
            ErrorKind::NotADirectory,
            "the root of a tree must be a directory",
        )
        .with_host_path(root));
    }
    let mut reader = Reader {
        tree: Tree::new(),
        options,
        skipped: Vec::new(),
        #[cfg(unix)]
        inodes: BTreeMap::new(),
        #[cfg(unix)]
        dirs: BTreeMap::new(),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        reader.dirs.insert((meta.dev(), meta.ino()), ());
    }
    let attrs = reader.attrs(&meta);
    reader.tree.replace("/", Node::dir().with_attrs(attrs))?;
    reader.walk(root)?;
    Ok((reader.tree, reader.skipped))
}

// ---------------------------------------------------------------------------
// Tree to host directory.

/// Whether Windows gives `name` a meaning other than a plain file in the
/// directory: a device name such as `CON` or `com1.txt`, a character that
/// names a stream or a wildcard, or a trailing dot or space it strips.
#[cfg(any(windows, test))]
fn windows_special(name: &str) -> bool {
    const DEVICES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return true;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    if DEVICES
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return true;
    }
    let mut chars = stem.chars();
    let prefix: alloc::string::String = chars.by_ref().take(3).collect();
    let digit = chars.next();
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(digit, Some('1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'))
        && chars.next().is_none()
}

/// `bytes` as a host file name, if it is exactly one plain component with
/// no `\`, NUL or drive prefix. On Windows, device names and names Windows
/// would alter are refused too.
fn host_name(bytes: &[u8]) -> Option<&OsStr> {
    if bytes.contains(&b'\\') || bytes.contains(&0) {
        return None;
    }
    #[cfg(unix)]
    let name = <OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(bytes);
    #[cfg(not(unix))]
    let name = OsStr::new(core::str::from_utf8(bytes).ok()?);
    #[cfg(windows)]
    if name.to_str().is_some_and(windows_special) {
        return None;
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) if part == name => Some(name),
        _ => None,
    }
}

/// Creates the directory `path`, or accepts an existing directory that is
/// not a symlink.
fn create_host_dir(path: &Path) -> io::Result<()> {
    match fs::create_dir(path) {
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(path)?.file_type().is_dir() {
                Ok(())
            } else {
                Err(err)
            }
        }
        other => other,
    }
}

/// The times and permissions to give a written node, and what the host
/// cannot keep.
struct Finish {
    modified: Option<SystemTime>,
    accessed: Option<SystemTime>,
    #[cfg(unix)]
    mode: Option<u32>,
}

impl Finish {
    fn of(attrs: &SetAttr, dropped: &mut [u64; 3]) -> Self {
        if attrs.created().is_some() {
            dropped[0] += 1;
        }
        if attrs.owner().is_some() {
            dropped[1] += 1;
        }
        if attrs
            .attributes()
            .is_some_and(|attributes| attributes != Attributes::NONE)
        {
            dropped[2] += 1;
        }
        Self {
            modified: attrs.modified().and_then(to_system_time),
            accessed: attrs.accessed().and_then(to_system_time),
            #[cfg(unix)]
            mode: attrs
                .permissions()
                .map(|permissions| permissions.bits() & 0o7777),
        }
    }

    fn apply(&self, file: &fs::File) -> io::Result<()> {
        let mut times = fs::FileTimes::new();
        if let Some(time) = self.modified {
            times = times.set_modified(time);
        }
        if let Some(time) = self.accessed {
            times = times.set_accessed(time);
        }
        if self.modified.is_some() || self.accessed.is_some() {
            file.set_times(times)?;
        }
        #[cfg(unix)]
        if let Some(mode) = self.mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        Ok(())
    }
}

struct Writer {
    report: Report,
    dropped: [u64; 3],
    links: BTreeMap<usize, PathBuf>,
    symlinks: Vec<(PathBuf, Vec<u8>, Vec<u8>)>,
    buf: Vec<u8>,
}

impl Writer {
    fn file(&mut self, entry: TreeEntry<'_>, host: &Path) -> Result<(), PathError> {
        let node = entry.node();
        if let Some(first) = self.links.get(&entry.id()) {
            return fs::hard_link(first, host)
                .map_err(|err| host_error(err, "cannot create hard link", host));
        }
        let finish = Finish::of(node.attrs(), &mut self.dropped);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(host)
            .map_err(|err| host_error(err, "cannot create host file", host))?;
        if let Some(content) = node.content() {
            let mut reader = ContentReader::open(content)?;
            let mut offset = 0;
            while offset < reader.len() {
                let n = reader.read_at(offset, &mut self.buf)?;
                if n == 0 {
                    return Err(PathError::new(
                        ErrorKind::Corrupt,
                        "file content ended early",
                    ));
                }
                file.write_all(&self.buf[..n])
                    .map_err(|err| host_error(err, "cannot write host file", host))?;
                offset += n as u64;
            }
        }
        finish
            .apply(&file)
            .map_err(|err| host_error(err, "cannot set host file times", host))?;
        if entry.links() > 1 {
            self.links.insert(entry.id(), host.to_path_buf());
        }
        Ok(())
    }

    /// Sets a directory's times and permissions once its children are in.
    fn finish_dir(&mut self, attrs: &SetAttr, host: &Path) -> Result<(), PathError> {
        let finish = Finish::of(attrs, &mut self.dropped);
        #[cfg(unix)]
        {
            let dir = fs::File::open(host)
                .map_err(|err| host_error(err, "cannot open host directory", host))?;
            finish
                .apply(&dir)
                .map_err(|err| host_error(err, "cannot set host directory times", host))?;
        }
        #[cfg(not(unix))]
        let _ = (finish, host);
        Ok(())
    }

    fn symlinks(&mut self) -> Result<(), PathError> {
        for (host, target, path) in core::mem::take(&mut self.symlinks) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(
                <OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&target),
                &host,
            )
            .map_err(|err| host_error(err, "cannot create symlink", &host).with_path(&path))?;
            #[cfg(not(unix))]
            {
                let _ = (host, target);
                self.report.push_warning(
                    Warning::new(WarningKind::Skipped, "symlinks are created on Unix only")
                        .with_path(&path),
                );
            }
        }
        Ok(())
    }
}

/// Writes `tree` into the host directory `dir`, which is created if
/// missing: the extraction half of [`read_tree`].
///
/// Take the tree from a mounted volume with `read_tree` in
/// [`crate::sync`] (content read lazily) or from a cpio stream; extracting
/// one file is a `read_tree` of that path.
///
/// It never writes outside `dir`: tree paths hold no `..`, a name with a
/// `\`, a NUL or a drive prefix fails with [`ErrorKind::InvalidInput`], as
/// do, on Windows, device names such as `CON` and names Windows would alter.
/// No existing symlink under `dir` is followed: an existing symlink or file
/// in the way fails with [`ErrorKind::AlreadyExists`], files are created
/// with `create_new`, and existing directories are merged. Symlinks from
/// the tree are created last, on Unix only.
///
/// Permissions (Unix) and modification and access times are kept,
/// directories' after their children, and hard links become host hard
/// links. Owners, creation times and DOS attributes are not restored, and
/// device nodes, FIFOs and sockets are skipped; the report lists both.
/// Errors carry the tree path and the host path. Another process that
/// swaps a directory below `dir` for a symlink while it runs can still
/// redirect a write, as with `tar`.
pub fn write_tree(dir: impl AsRef<Path>, tree: &Tree) -> Result<Report, PathError> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir).map_err(|err| host_error(err, "cannot create host directory", dir))?;
    let mut writer = Writer {
        report: Report::new(),
        dropped: [0; 3],
        links: BTreeMap::new(),
        symlinks: Vec::new(),
        buf: alloc::vec![0u8; 64 * 1024],
    };
    let root = tree.root();
    type Frame<'t> = (
        Vec<(&'t crate::Name, TreeEntry<'t>)>,
        usize,
        PathBuf,
        Vec<u8>,
        Option<SetAttr>,
    );
    let mut stack: Vec<Frame<'_>> = alloc::vec![(
        root.children().collect(),
        0,
        dir.to_path_buf(),
        Vec::new(),
        None
    )];
    while let Some((children, next, host_dir, prefix, _)) = stack.last_mut() {
        let Some(&(name, entry)) = children.get(*next) else {
            if let Some((_, _, host, path, Some(attrs))) = stack.pop() {
                writer
                    .finish_dir(&attrs, &host)
                    .map_err(|err| err.with_path(&path))?;
            }
            continue;
        };
        *next += 1;
        let mut path = prefix.clone();
        path.push(b'/');
        path.extend_from_slice(name.as_bytes());
        let host_dir = host_dir.clone();
        let Some(host_part) = host_name(name.as_bytes()) else {
            return Err(
                PathError::new(ErrorKind::InvalidInput, "name cannot be a host file name")
                    .with_path(&path),
            );
        };
        let host = host_dir.join(host_part);
        let node = entry.node();
        match node.file_type() {
            FileType::Dir => {
                if stack.len() > MAX_DEPTH {
                    return Err(
                        PathError::new(ErrorKind::LimitExceeded, "the tree is too deep")
                            .with_path(&path),
                    );
                }
                create_host_dir(&host).map_err(|err| {
                    host_error(err, "cannot create host directory", &host).with_path(&path)
                })?;
                stack.push((
                    entry.children().collect(),
                    0,
                    host,
                    path,
                    Some(*node.attrs()),
                ));
            }
            FileType::File => writer
                .file(entry, &host)
                .map_err(|err| err.with_path(&path))?,
            FileType::Symlink => {
                if fs::symlink_metadata(&host).is_ok() {
                    return Err(PathError::new(
                        ErrorKind::AlreadyExists,
                        "a host file is in the way",
                    )
                    .with_path(&path)
                    .with_host_path(&host));
                }
                let target = node.target().unwrap_or_default().to_vec();
                writer.symlinks.push((host, target, path));
            }
            _ => writer.report.push_warning(
                Warning::new(
                    WarningKind::Skipped,
                    "device nodes, FIFOs and sockets are not created on the host",
                )
                .with_path(&path),
            ),
        }
    }
    writer.symlinks()?;
    for (field, count) in [Field::Created, Field::Owner, Field::Attributes]
        .into_iter()
        .zip(writer.dropped)
    {
        if count > 0 {
            writer.report.push_warning(
                Warning::new(WarningKind::Dropped(field), "not restored on the host")
                    .with_count(count),
            );
        }
    }
    Ok(writer.report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_are_single_components() {
        assert!(host_name(b"boot.cfg").is_some());
        for bad in [
            &b""[..],
            b".",
            b"..",
            b"a/b",
            b"/etc",
            b"a/",
            b"a\\b",
            b"a\0",
        ] {
            assert!(host_name(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn windows_special_names() {
        for name in [
            "boot.cfg",
            "CONFIG",
            "console.log",
            "COM10",
            "LPT0",
            "comx",
            "a.b.c",
            "NULL.txt",
        ] {
            assert!(!windows_special(name), "{name}");
        }
        for name in [
            "CON",
            "con",
            "Con.txt",
            "nul.tar.gz",
            "AUX ",
            "aux .c",
            "PRN",
            "COM1",
            "com9.log",
            "LPT3",
            "lpt\u{b9}",
            "CONIN$",
            "conout$.x",
            "a:b",
            "stream::$DATA",
            "a?",
            "a*",
            "a<b",
            "a|b",
            "a\"b",
            "tab\t",
            "dot.",
            "space ",
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

    #[test]
    fn the_local_offset_is_within_a_day() {
        let offset = local_utc_offset();
        assert!((-24 * 60..=24 * 60).contains(&offset));
        assert_eq!(mount_options().utc_offset(), Some(offset));
    }
}
