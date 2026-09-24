//! Device errors survive without allocation (S12), std users get
//! `io::Error` (S11), and read-only mounts refuse writes up front (S16).

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use common::MemError;
use common::sync::{MemFs, fixture};
use hadris_fs::sync::{DriverExt, PathExt, Volume};
use hadris_fs::{ErrorKind, OpenOptions, PathError};

#[test]
fn kernels_get_their_device_error_back() {
    let mut fs = fixture();
    fs.fail_next(MemError::Timeout { lba: 7 });
    let err = fs.read_to_vec("/a.txt").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(err.device_error(), Some(&MemError::Timeout { lba: 7 }));
    let errno = match (err.kind(), err.device_error()) {
        (_, Some(MemError::Timeout { .. })) => 110,
        (ErrorKind::NotFound, None) => 2,
        _ => 5,
    };
    assert_eq!(errno, 110);
    let missing = fs.read_to_vec("/nope").unwrap_err();
    assert_eq!(
        (missing.kind(), missing.device_error()),
        (ErrorKind::NotFound, None)
    );
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn std_users_get_io_errors() {
    fn read(vol: &Volume<MemFs, hadris_fs::sync::StdMutex>) -> std::io::Result<Vec<u8>> {
        Ok(vol.read_to_vec("/nope")?)
    }
    let vol = Volume::new(fixture());
    assert_eq!(read(&vol).unwrap_err().kind(), std::io::ErrorKind::NotFound);
    vol.lock().fail_next(MemError::Timeout { lba: 1 });
    let err: std::io::Error = vol.read_to_vec("/a.txt").unwrap_err().into();
    let inner = err.into_inner().unwrap().downcast::<MemError>().unwrap();
    assert_eq!(*inner, MemError::Timeout { lba: 1 });
}

#[test]
fn read_only_mounts_refuse_writes_before_touching_anything() {
    let mut fs = fixture().read_only();
    let err = fs
        .open("/a.txt", OpenOptions::write().truncate())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ReadOnly);
    assert_eq!(fs.read_to_vec("/a.txt").unwrap(), b"root a");
    assert_eq!(
        fs.write_file("/new", b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fs.create_dir_all("/d").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    let io: std::io::Error = fs.write_file("/a.txt", b"x").unwrap_err().into();
    assert_eq!(io.kind(), std::io::ErrorKind::ReadOnlyFilesystem);
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn code_mixing_devices_uses_any_error() {
    fn copy<S: DriverExt, T: DriverExt>(
        src: &mut S,
        dst: &mut T,
        path: &str,
    ) -> Result<(), PathError> {
        let data = src.read_to_vec(path)?;
        dst.write_file(path, &data)?;
        Ok(())
    }
    let mut src = fixture();
    let mut dst = MemFs::new();
    copy(&mut src, &mut dst, "/a.txt").unwrap();
    assert_eq!(dst.read_to_vec("/a.txt").unwrap(), b"root a");
    src.fail_next(MemError::Timeout { lba: 3 });
    let err = copy(&mut src, &mut dst, "/a.txt").unwrap_err();
    assert_eq!(
        err.downcast_device::<MemError>(),
        Some(&MemError::Timeout { lba: 3 })
    );
}

#[test]
fn truncating_open_shrinks_the_file() {
    let vol = Volume::local(fixture());
    vol.write_file("/a.txt", b"xy").unwrap();
    assert_eq!(vol.read_to_vec("/a.txt").unwrap(), b"xy");
}
