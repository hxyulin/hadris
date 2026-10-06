//! Device errors survive without allocation (S12), std users get
//! `io::Error` (S11), and read-only mounts refuse writes up front (S16).

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use common::MemError;
use common::files::{read, write};
use common::sync::{MemFs, fixture};
use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{ErrorKind, Name, OpenOptions, PathError, Resolve};

#[test]
fn kernels_get_their_device_error_back() {
    let mut fs = fixture();
    fs.fail_next(MemError::Timeout { lba: 7 });
    let err = fs.lookup(fs.root(), Name::new("a.txt")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(err.device_error(), Some(&MemError::Timeout { lba: 7 }));
    let missing = fs.resolve(b"/nope", Resolve::Lexical).unwrap_err();
    assert_eq!(
        (missing.kind(), missing.device_error()),
        (ErrorKind::NotFound, None)
    );
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn std_users_get_io_errors() {
    fn read_io(vol: &Volume<MemFs>, path: &str) -> std::io::Result<Vec<u8>> {
        Ok(read(vol, path)?)
    }
    let vol = Volume::new(fixture());
    assert_eq!(
        read_io(&vol, "/nope").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    vol.lock().fail_next(MemError::Timeout { lba: 1 });
    let err = read_io(&vol, "/a.txt").unwrap_err();
    let inner = err.into_inner().unwrap().downcast::<MemError>().unwrap();
    assert_eq!(*inner, MemError::Timeout { lba: 1 });
}

#[test]
fn read_only_mounts_refuse_writes_before_touching_anything() {
    let vol = Volume::new(fixture().read_only());
    let err = vol
        .open("/a.txt", OpenOptions::new().write().truncate())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ReadOnly);
    assert_eq!(read(&vol, "/a.txt").unwrap(), b"root a");
    assert_eq!(
        write(&vol, "/new", b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        vol.create_dir_all("/d").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    let io: std::io::Error = write(&vol, "/a.txt", b"x").unwrap_err().into();
    assert_eq!(io.kind(), std::io::ErrorKind::ReadOnlyFilesystem);
    assert_eq!(vol.into_inner().unwrap().open_nodes(), 1);
}

#[test]
fn code_mixing_devices_uses_path_error() {
    fn copy<S: FileSystem, T: FileSystem>(
        src: &Volume<S>,
        dst: &Volume<T>,
        path: &str,
    ) -> Result<(), PathError> {
        let data = read(src, path)?;
        write(dst, path, &data)?;
        Ok(())
    }
    let src = Volume::new(fixture());
    let dst = Volume::new(MemFs::new());
    copy(&src, &dst, "/a.txt").unwrap();
    assert_eq!(read(&dst, "/a.txt").unwrap(), b"root a");
    src.lock().fail_next(MemError::Timeout { lba: 3 });
    let err = copy(&src, &dst, "/a.txt").unwrap_err();
    assert_eq!(
        err.downcast_device::<MemError>(),
        Some(&MemError::Timeout { lba: 3 })
    );
}
