//! The V2 traits, which report one erased [`Error`].
//!
//! Format crates use these until each one moves to the typed traits at the
//! crate root. This module is removed before 3.0; new code uses
//! [`ErrorType`](crate::ErrorType) and the typed [`Read`](crate::sync::Read),
//! [`Write`](crate::sync::Write) and [`Seek`](crate::sync::Seek).

mod error;
pub use error::{Error, ErrorKind, Result};

#[cfg(feature = "std")]
pub use crate::StdIo;
pub use crate::try_io_result_option;
pub use crate::{Cursor, FromEmbedded, SeekFrom};

/// Synchronous V2 traits.
#[cfg(feature = "sync")]
pub mod sync;

#[cfg(feature = "sync")]
pub use sync::*;

/// Asynchronous V2 traits.
#[cfg(feature = "async")]
pub mod r#async;

#[cfg(feature = "sync")]
impl sync::Read for Cursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_slice(buf))
    }
}

#[cfg(feature = "sync")]
impl sync::Seek for Cursor<'_> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.seek_to(pos)
            .map_err(|_| Error::from_kind(ErrorKind::InvalidInput))
    }
}

#[cfg(feature = "async")]
impl r#async::Read for Cursor<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_slice(buf))
    }
}

#[cfg(feature = "async")]
impl r#async::Seek for Cursor<'_> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.seek_to(pos)
            .map_err(|_| Error::from_kind(ErrorKind::InvalidInput))
    }
}

#[cfg(all(feature = "std", feature = "sync"))]
impl<T: std::io::Read + ?Sized> sync::Read for StdIo<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(std::io::Read::read(self.get_mut(), buf)?)
    }
}

#[cfg(all(feature = "std", feature = "sync"))]
impl<T: std::io::Write + ?Sized> sync::Write for StdIo<T> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(std::io::Write::write(self.get_mut(), buf)?)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(std::io::Write::flush(self.get_mut())?)
    }
}

#[cfg(all(feature = "std", feature = "sync"))]
impl<T: std::io::Seek + ?Sized> sync::Seek for StdIo<T> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        Ok(std::io::Seek::seek(self.get_mut(), pos.into())?)
    }
}

#[cfg(all(test, feature = "sync"))]
mod tests {
    use super::*;

    #[test]
    fn cursor_reads_and_seeks() {
        let data = [1u8, 2, 3, 4, 5];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 2];
        cursor.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [1, 2]);
        assert_eq!(cursor.seek(SeekFrom::Start(0)).unwrap(), 0);
        assert_eq!(
            cursor.seek(SeekFrom::End(-9)).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn read_struct_through_legacy_ext() {
        let bytes = 0x1234u16.to_ne_bytes();
        let mut cursor = Cursor::new(&bytes);
        let value: u16 = cursor.read_struct().unwrap();
        assert_eq!(value, 0x1234);
    }

    #[test]
    fn try_io_result_option_short_circuits() {
        fn next(ok: bool) -> Option<Result<u32>> {
            let value: Result<u32> = if ok {
                Ok(21)
            } else {
                Err(Error::new(ErrorKind::NotFound, "missing"))
            };
            Some(Ok(try_io_result_option!(value) * 2))
        }
        assert!(matches!(next(true), Some(Ok(42))));
        assert!(matches!(next(false), Some(Err(_))));
    }

    #[test]
    fn byte_source_over_slice() {
        let mut slice: &[u8] = &[1, 2, 3, 4];
        let mut buf = [0u8; 3];
        assert_eq!(ByteSource::read_at(&mut slice, 2, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[3, 4]);
    }
}
