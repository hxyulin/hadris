#![cfg(all(feature = "std", feature = "sync"))]

use hadris_io::{ExactError, Read, Seek, SeekFrom};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceError;

impl core::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("device failed")
    }
}

impl core::error::Error for DeviceError {}

impl embedded_io::Error for DeviceError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

struct Device;

impl embedded_io::ErrorType for Device {
    type Error = DeviceError;
}

impl embedded_io::Read for Device {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
        Err(DeviceError)
    }
}

impl embedded_io::Seek for Device {
    fn seek(&mut self, _pos: embedded_io::SeekFrom) -> Result<u64, Self::Error> {
        Err(DeviceError)
    }
}

#[test]
fn from_embedded_returns_the_device_error() {
    let mut reader = hadris_io::FromEmbedded::new(Device);
    assert_eq!(Read::read(&mut reader, &mut [0]), Err(DeviceError));
    assert_eq!(
        Read::read_exact(&mut reader, &mut [0]),
        Err(ExactError::Io(DeviceError))
    );
    let mut seeker = hadris_io::FromEmbedded::new(Device);
    assert_eq!(
        Seek::seek(&mut seeker, SeekFrom::Start(0)),
        Err(DeviceError)
    );
}

#[test]
fn std_reader_is_accepted_through_std_io() {
    let mut reader = hadris_io::StdIo::new(std::io::Cursor::new(b"ok".to_vec()));
    let mut bytes = [0; 2];
    Read::read_exact(&mut reader, &mut bytes).unwrap();
    assert_eq!(&bytes, b"ok");
}

#[test]
fn std_errors_survive_std_io_unchanged() {
    let mut reader = hadris_io::StdIo::new(std::io::Cursor::new(b"x".to_vec()));
    let err: std::io::Error = Read::read_exact(&mut reader, &mut [0; 4])
        .unwrap_err()
        .into();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn device_errors_become_io_error_sources() {
    let mut reader = hadris_io::FromEmbedded::new(Device);
    let err: std::io::Error = Read::read_exact(&mut reader, &mut [0]).unwrap_err().into();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(
        err.into_inner()
            .unwrap()
            .downcast::<DeviceError>()
            .ok()
            .as_deref(),
        Some(&DeviceError)
    );
}

#[test]
fn to_std_reads_typed_readers() {
    use std::io::Read as _;
    let mut out = String::new();
    hadris_io::ToStd::new(hadris_io::Cursor::new(b"abc"))
        .read_to_string(&mut out)
        .unwrap();
    assert_eq!(out, "abc");
}
