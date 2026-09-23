use hadris_io::SeekFrom;
use hadris_io::legacy::sync as legacy;
use hadris_io::sync::{Read, Seek, Write};

use crate::PartitionView;

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

use hadris_io::sync::MaybeSend;

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

impl<S: legacy::Read + legacy::Seek> legacy::Read for PartitionView<'_, S> {
    fn read(&mut self, buffer: &mut [u8]) -> hadris_io::legacy::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source.seek(SeekFrom::Start(absolute))?;
        let read = self.source.read(&mut buffer[..length])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<S: legacy::Read + legacy::Seek> legacy::Seek for PartitionView<'_, S> {
    fn seek(&mut self, from: SeekFrom) -> hadris_io::legacy::Result<u64> {
        self.position = self.seek_position(from)?;
        Ok(self.position)
    }
}

impl<S: legacy::Read + legacy::Write + legacy::Seek> legacy::Write for PartitionView<'_, S> {
    fn write(&mut self, buffer: &[u8]) -> hadris_io::legacy::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source.seek(SeekFrom::Start(absolute))?;
        let written = self.source.write(&buffer[..length])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> hadris_io::legacy::Result<()> {
        self.source.flush()
    }
}

/// A host file is a block device with 512-byte blocks.
///
/// Errors are the `std::io::Error` itself, so `raw_os_error()` survives. A
/// file opened read-only fails writes with the OS error. The block count is
/// measured by seeking to the end, which also works for block device nodes.
#[cfg(feature = "std")]
impl BlockDevice for std::fs::File {
    fn block_size(&self) -> crate::BlockSize {
        const { crate::BlockSize::new(512).unwrap() }
    }

    fn block_count(&self) -> u64 {
        let mut file = self;
        let Ok(here) = std::io::Seek::stream_position(&mut file) else {
            return 0;
        };
        let end = std::io::Seek::seek(&mut file, std::io::SeekFrom::End(0)).unwrap_or(0);
        let _ = std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(here));
        end / 512
    }

    fn read_blocks(&mut self, first: crate::BlockIndex, buf: &mut [u8]) -> std::io::Result<()> {
        let offset = crate::device::byte_offset(self.block_size(), first)?;
        std::io::Seek::seek(self, std::io::SeekFrom::Start(offset))?;
        std::io::Read::read_exact(self, buf)
    }

    fn write_blocks(
        &mut self,
        first: crate::BlockIndex,
        buf: &[u8],
    ) -> Result<(), crate::WriteError<std::io::Error>> {
        let offset =
            crate::device::byte_offset(self.block_size(), first).map_err(std::io::Error::from)?;
        std::io::Seek::seek(self, std::io::SeekFrom::Start(offset))?;
        Ok(std::io::Write::write_all(self, buf)?)
    }

    fn flush(&mut self) -> Result<(), crate::WriteError<std::io::Error>> {
        Ok(std::io::Write::flush(self)?)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use hadris_io::legacy::sync::{Read, Seek, Write};
    use hadris_io::{SeekFrom, StdIo};
    use std::vec;

    use crate::PartitionView;

    #[test]
    fn partition_view_translates_and_bounds_io() {
        let mut source = StdIo::new(std::io::Cursor::new(vec![0_u8, 1, 2, 3, 4, 5, 6, 7]));
        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();

        let mut buffer = [0_u8; 8];
        assert_eq!(view.read(&mut buffer).unwrap(), 4);
        assert_eq!(&buffer[..4], &[2, 3, 4, 5]);
        assert_eq!(view.read(&mut buffer).unwrap(), 0);

        view.seek(SeekFrom::Start(1)).unwrap();
        view.write_all(&[9, 8]).unwrap();
        let source = view.into_inner();
        assert_eq!(&source.get_ref().get_ref()[..], &[0, 1, 2, 9, 8, 5, 6, 7]);
    }

    #[test]
    fn partition_view_rejects_invalid_ranges_and_seeks() {
        let mut source = StdIo::new(std::io::Cursor::new(vec![0_u8; 8]));
        assert!(PartitionView::new(&mut source, u64::MAX, 2).is_err());
        assert!(PartitionView::new(&mut source, 0, 0).is_err());

        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();
        assert!(view.seek(SeekFrom::Start(5)).is_err());
        assert!(view.seek(SeekFrom::End(-5)).is_err());
    }
}

#[cfg(all(test, feature = "std"))]
mod device_tests {
    use hadris_io::sync::{Read as _, Seek as _, Write as _};
    use hadris_io::{ErrorType, SeekFrom, StdIo};

    use crate::sync::{BlockDevice, ByteView, Cache, Slice, StreamDevice};
    use crate::{BlockIndex, BlockSize, MemDevice, OutOfRange, ReadOnly, StorageError, WriteError};

    const B4: BlockSize = match BlockSize::new(4) {
        Some(size) => size,
        None => panic!(),
    };

    fn counting(len: u8) -> std::vec::Vec<u8> {
        (0..len).collect()
    }

    #[test]
    fn mem_device_reads_writes_and_bounds() {
        let mut device = MemDevice::new(counting(18), B4);
        assert_eq!(device.block_count(), 4);

        let mut buf = [0_u8; 8];
        device.read_blocks(BlockIndex(1), &mut buf).unwrap();
        assert_eq!(buf, [4, 5, 6, 7, 8, 9, 10, 11]);

        device.write_blocks(BlockIndex(3), &[9; 4]).unwrap();
        assert_eq!(&device.get_ref()[12..18], &[9, 9, 9, 9, 16, 17]);

        assert_eq!(device.read_blocks(BlockIndex(3), &mut buf), Err(OutOfRange));
        assert_eq!(
            device.read_blocks(BlockIndex(0), &mut buf[..3]),
            Err(OutOfRange)
        );
        assert_eq!(
            device.read_blocks(BlockIndex(u64::MAX), &mut buf),
            Err(OutOfRange)
        );
        assert_eq!(
            device.write_blocks(BlockIndex(4), &[0; 4]),
            Err(WriteError::Device(OutOfRange))
        );
    }

    #[test]
    fn read_only_buffers_reject_writes() {
        let bytes = counting(8);
        let mut device = MemDevice::new(&bytes[..], B4);
        assert_eq!(
            device.write_blocks(BlockIndex(0), &[0; 4]),
            Err(WriteError::ReadOnly)
        );
    }

    /// A device that implements no write method.
    struct Rom<'a>(&'a [u8]);

    impl ErrorType for Rom<'_> {
        type Error = OutOfRange;
    }

    impl BlockDevice for Rom<'_> {
        fn block_size(&self) -> BlockSize {
            B4
        }

        fn block_count(&self) -> u64 {
            self.0.len() as u64 / 4
        }

        fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
            let at = first.0 as usize * 4;
            buf.copy_from_slice(self.0.get(at..at + buf.len()).ok_or(OutOfRange)?);
            Ok(())
        }
    }

    #[test]
    fn read_only_devices_need_no_write_method() {
        let bytes = counting(8);
        let mut rom = Rom(&bytes);
        assert_eq!(
            rom.write_blocks(BlockIndex(0), &[0; 4]),
            Err(WriteError::ReadOnly)
        );
        assert_eq!(rom.flush(), Ok(()));
    }

    #[test]
    fn slice_offsets_and_bounds() {
        let mut device = MemDevice::new(counting(16), B4);
        assert!(Slice::new(&mut device, BlockIndex(3), 2).is_err());

        let mut slice = Slice::new(&mut device, BlockIndex(1), 2).unwrap();
        assert_eq!(slice.block_count(), 2);
        let mut buf = [0_u8; 4];
        slice.read_blocks(BlockIndex(1), &mut buf).unwrap();
        assert_eq!(buf, [8, 9, 10, 11]);
        assert_eq!(
            slice.read_blocks(BlockIndex(2), &mut buf),
            Err(StorageError::OutOfRange)
        );
        assert_eq!(
            slice.write_blocks(BlockIndex(2), &buf),
            Err(WriteError::Device(StorageError::OutOfRange))
        );

        slice.write_blocks(BlockIndex(0), &[0xAA; 4]).unwrap();
        assert_eq!(&device.get_ref()[4..8], &[0xAA; 4]);
    }

    #[test]
    fn stream_device_over_std_and_read_only_streams() {
        let stream = StdIo::new(std::io::Cursor::new(counting(10)));
        let mut device = StreamDevice::new(stream, B4).unwrap();
        assert_eq!(device.block_count(), 2);
        device.write_blocks(BlockIndex(1), &[7; 4]).unwrap();
        let mut buf = [0_u8; 8];
        device.read_blocks(BlockIndex(0), &mut buf).unwrap();
        assert_eq!(buf, [0, 1, 2, 3, 7, 7, 7, 7]);
        assert!(matches!(
            device.write_blocks(BlockIndex(2), &[0; 4]),
            Err(WriteError::Device(StorageError::OutOfRange))
        ));

        let bytes = counting(8);
        let stream = ReadOnly::new(hadris_io::Cursor::new(&bytes));
        let mut device = StreamDevice::new(stream, B4).unwrap();
        device.read_blocks(BlockIndex(1), &mut buf[..4]).unwrap();
        assert_eq!(&buf[..4], &[4, 5, 6, 7]);
        assert_eq!(
            device.write_blocks(BlockIndex(0), &[0; 4]),
            Err(WriteError::ReadOnly)
        );
    }

    #[test]
    fn stream_device_keeps_std_errors() {
        let stream = StdIo::new(std::io::Cursor::new(counting(8)));
        let mut device = StreamDevice::with_block_count(stream, B4, 4);
        let err: std::io::Error = device
            .read_blocks(BlockIndex(3), &mut [0; 4])
            .unwrap_err()
            .into();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn cache_writes_back_on_eviction_and_flush() {
        let device = MemDevice::new(counting(16), B4);
        let mut cache = Cache::new(device, 2);

        cache.write_blocks(BlockIndex(3), &[3; 4]).unwrap();
        assert!(!cache.is_dirty());
        assert_eq!(&cache.get_ref().get_ref()[12..], &[3; 4]);

        cache.write_blocks(BlockIndex(0), &[1; 4]).unwrap();
        cache.write_blocks(BlockIndex(1), &[2; 4]).unwrap();
        assert!(cache.is_dirty());
        assert_eq!(&cache.get_ref().get_ref()[..8], &counting(8)[..]);

        let mut buf = [0_u8; 4];
        cache.read_blocks(BlockIndex(1), &mut buf).unwrap();
        cache.read_blocks(BlockIndex(2), &mut buf).unwrap();
        assert_eq!(buf, [8, 9, 10, 11]);
        assert_eq!(&cache.get_ref().get_ref()[..4], &[1; 4]);
        assert_eq!(&cache.get_ref().get_ref()[4..8], &[4, 5, 6, 7]);

        let mut all = [0_u8; 16];
        cache.read_blocks(BlockIndex(0), &mut all).unwrap();
        assert_eq!(&all[..8], &[1, 1, 1, 1, 2, 2, 2, 2]);
        assert_eq!(cache.read_blocks(BlockIndex(4), &mut buf), Err(OutOfRange));

        let device = cache.finish().unwrap();
        assert_eq!(&device.get_ref()[..8], &[1, 1, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn cache_reports_read_only_on_the_first_write() {
        let bytes = counting(8);
        let mut cache = Cache::new(MemDevice::new(&bytes[..], B4), 4);
        assert_eq!(
            cache.write_blocks(BlockIndex(0), &[0; 4]),
            Err(WriteError::ReadOnly)
        );
        assert!(!cache.is_dirty());
    }

    #[test]
    fn byte_view_handles_partial_blocks() {
        let mut view = ByteView::new(MemDevice::new(counting(16), B4));
        assert_eq!(view.len(), 16);

        let mut buf = [0_u8; 7];
        view.read_at(3, &mut buf).unwrap();
        assert_eq!(buf, [3, 4, 5, 6, 7, 8, 9]);

        view.write_at(2, &[0xEE; 11]).unwrap();
        assert_eq!(
            view.get_ref().get_ref()[..],
            [
                0, 1, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 13, 14, 15
            ]
        );

        assert_eq!(view.read_at(12, &mut buf), Err(StorageError::UnexpectedEof));
        assert_eq!(view.write_at(15, &[0; 2]), Err(StorageError::OutOfRange));
    }

    #[test]
    fn byte_view_is_a_bounded_stream() {
        let mut view = ByteView::new(MemDevice::new(counting(8), B4));
        view.seek(SeekFrom::Start(6)).unwrap();
        let mut buf = [0_u8; 4];
        assert_eq!(view.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[6, 7]);
        assert_eq!(view.read(&mut buf).unwrap(), 0);

        view.seek(SeekFrom::End(-3)).unwrap();
        assert_eq!(view.write(&[1, 2, 3, 4]).unwrap(), 3);
        assert_eq!(view.write_all(&[5]), Err(hadris_io::ExactError::WriteZero));
        assert_eq!(&view.get_ref().get_ref()[5..], &[1, 2, 3]);

        let bytes = counting(8);
        let mut view = ByteView::new(MemDevice::new(&bytes[..], B4));
        assert_eq!(view.write(&[1]), Err(StorageError::ReadOnly));
    }

    #[test]
    fn std_file_is_a_device() {
        let path = std::env::temp_dir().join(std::format!(
            "hadris-storage-file-{}.img",
            std::process::id()
        ));
        std::fs::write(
            &path,
            counting(0)
                .iter()
                .chain(&[5u8; 1024])
                .copied()
                .collect::<std::vec::Vec<u8>>(),
        )
        .unwrap();
        let mut file = std::fs::File::options()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert_eq!(file.block_count(), 2);
        file.write_blocks(BlockIndex(1), &[9; 512]).unwrap();
        let mut block = [0u8; 512];
        file.read_blocks(BlockIndex(1), &mut block).unwrap();
        assert_eq!(block, [9; 512]);

        let mut read_only = std::fs::File::open(&path).unwrap();
        match read_only.write_blocks(BlockIndex(0), &block) {
            Err(WriteError::Device(err)) => assert!(err.raw_os_error().is_some()),
            other => panic!("expected an OS error, got {other:?}"),
        }
        std::fs::remove_file(&path).unwrap();
    }
}
