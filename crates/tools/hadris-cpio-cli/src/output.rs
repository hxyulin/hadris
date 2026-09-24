//! Output files that appear only once they are complete.

use std::fs::{self, File, Permissions};
use std::io;
use std::path::{Path, PathBuf};

use tempfile::{Builder, TempPath};

/// An output being written. A regular file is written to a temporary file in
/// the same directory and renamed into place by [`Output::commit`]; dropping
/// the output uncommitted deletes the temporary file and leaves the path
/// untouched. A device or other non-regular file that already exists at the
/// path is written in place.
pub struct Output {
    path: PathBuf,
    temp: Option<TempPath>,
    replace: bool,
}

impl Output {
    /// Starts writing `path`, replacing any existing file when committed.
    #[allow(dead_code)]
    pub fn create(path: &Path) -> io::Result<(File, Self)> {
        Self::start(path, true)
    }

    /// Starts writing `path`, which must not exist now or when committed.
    #[allow(dead_code)]
    pub fn create_new(path: &Path) -> io::Result<(File, Self)> {
        Self::start(path, false)
    }

    fn start(path: &Path, replace: bool) -> io::Result<(File, Self)> {
        let (target, permissions) = match fs::metadata(path) {
            Ok(_) if !replace => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "output already exists",
                ));
            }
            Ok(meta) if meta.is_file() => (fs::canonicalize(path)?, kept_permissions(&meta)),
            Ok(_) => {
                let file = File::options().read(true).write(true).open(path)?;
                let output = Self {
                    path: path.to_path_buf(),
                    temp: None,
                    replace,
                };
                return Ok((file, output));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                (path.to_path_buf(), default_permissions())
            }
            Err(err) => return Err(err),
        };
        let dir = match target.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        let mut prefix = std::ffi::OsString::from(".");
        prefix.push(target.file_name().unwrap_or_default());
        prefix.push(".");
        let mut builder = Builder::new();
        builder.prefix(&prefix).suffix(".tmp");
        if let Some(permissions) = permissions {
            builder.permissions(permissions);
        }
        let (file, temp) = builder.tempfile_in(dir)?.into_parts();
        let output = Self {
            path: target,
            temp: Some(temp),
            replace,
        };
        Ok((file, output))
    }

    /// Whether the output is a regular file rather than a device.
    #[allow(dead_code)]
    pub fn is_regular(&self) -> bool {
        self.temp.is_some()
    }

    /// Flushes `file` to disk and moves it into place.
    pub fn commit(self, file: File) -> io::Result<()> {
        let Some(temp) = self.temp else {
            return Ok(());
        };
        file.sync_all()?;
        drop(file);
        let result = if self.replace {
            temp.persist(&self.path)
        } else {
            temp.persist_noclobber(&self.path)
        };
        result.map_err(|err| err.error)
    }
}

#[cfg(unix)]
fn default_permissions() -> Option<Permissions> {
    use std::os::unix::fs::PermissionsExt;
    Some(Permissions::from_mode(0o666))
}

#[cfg(not(unix))]
fn default_permissions() -> Option<Permissions> {
    None
}

#[cfg(unix)]
fn kept_permissions(meta: &fs::Metadata) -> Option<Permissions> {
    Some(meta.permissions())
}

#[cfg(not(unix))]
fn kept_permissions(_: &fs::Metadata) -> Option<Permissions> {
    None
}
