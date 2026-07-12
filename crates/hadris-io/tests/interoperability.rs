use hadris_io::{Error, ErrorKind, Read};

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

#[test]
fn embedded_adapter_preserves_typed_source() {
    let mut reader = hadris_io::sync::FromEmbedded::new(Device);
    let error = Read::read(&mut reader, &mut [0]).unwrap_err();
    assert_eq!(error.source_ref(), Some(&DeviceError));
    assert_eq!(error.kind(), ErrorKind::Other);
}

#[test]
fn std_reader_is_accepted_without_adapter() {
    let mut reader = std::io::Cursor::new(b"ok".to_vec());
    let mut bytes = [0; 2];
    Read::read_exact(&mut reader, &mut bytes).unwrap();
    assert_eq!(&bytes, b"ok");
}

#[test]
fn std_source_round_trip_is_lossless() {
    let source = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let wrapped: Error<std::io::Error> = source.into();
    assert_eq!(wrapped.kind(), ErrorKind::PermissionDenied);
    let source: std::io::Error = wrapped.into();
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(source.to_string(), "denied");
}
