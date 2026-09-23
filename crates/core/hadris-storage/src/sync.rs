use hadris_io::SeekFrom;
use hadris_io::sync::{Read, Seek, Write};

use crate::PartitionView;

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

impl<S: Read + Seek> Read for PartitionView<'_, S> {
    fn read(&mut self, buffer: &mut [u8]) -> hadris_io::Result<usize> {
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

impl<S: Read + Seek> Seek for PartitionView<'_, S> {
    fn seek(&mut self, from: SeekFrom) -> hadris_io::Result<u64> {
        self.position = self.seek_position(from)?;
        Ok(self.position)
    }
}

impl<S: Read + Write + Seek> Write for PartitionView<'_, S> {
    fn write(&mut self, buffer: &[u8]) -> hadris_io::Result<usize> {
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

    fn flush(&mut self) -> hadris_io::Result<()> {
        self.source.flush()
    }
}

#[cfg(test)]
mod tests {
    use hadris_io::sync::{Read, Seek, Write};
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

    use crate::sync::{BlockDevice, ByteView, Cache, Slice, StreamDevice};
    use crate::{Access, BlockIndex, BlockSize, MemDevice, ReadOnly};
    use hadris_io::ErrorKind;

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
        assert_eq!(device.access(), Access::ReadWrite);

        let mut buf = [0_u8; 8];
        device.read_blocks(BlockIndex(1), &mut buf).unwrap();
        assert_eq!(buf, [4, 5, 6, 7, 8, 9, 10, 11]);

        device.write_blocks(BlockIndex(3), &[9; 4]).unwrap();
        assert_eq!(&device.get_ref()[12..18], &[9, 9, 9, 9, 16, 17]);

        let kind = |r: hadris_io::Result<()>| r.unwrap_err().kind();
        assert_eq!(
            kind(device.read_blocks(BlockIndex(3), &mut buf)),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(device.read_blocks(BlockIndex(0), &mut buf[..3])),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(device.read_blocks(BlockIndex(u64::MAX), &mut buf)),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn read_only_buffers_reject_writes() {
        let bytes = counting(8);
        let mut device = MemDevice::new(&bytes[..], B4);
        assert_eq!(device.access(), Access::ReadOnly);
        let error = device.write_blocks(BlockIndex(0), &[0; 4]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
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
        assert!(slice.read_blocks(BlockIndex(2), &mut buf).is_err());

        slice.write_blocks(BlockIndex(0), &[0xAA; 4]).unwrap();
        assert_eq!(&device.get_ref()[4..8], &[0xAA; 4]);
    }

    #[test]
    fn stream_device_over_std_and_read_only_streams() {
        let stream = StdIo::new(std::io::Cursor::new(counting(10)));
        let mut device = StreamDevice::new(stream, B4).unwrap();
        assert_eq!(device.block_count(), 2);
        assert_eq!(device.access(), Access::ReadWrite);
        device.write_blocks(BlockIndex(1), &[7; 4]).unwrap();
        let mut buf = [0_u8; 8];
        device.read_blocks(BlockIndex(0), &mut buf).unwrap();
        assert_eq!(buf, [0, 1, 2, 3, 7, 7, 7, 7]);

        let bytes = counting(8);
        let stream = ReadOnly::new(hadris_io::Cursor::new(&bytes));
        let mut device = StreamDevice::new(stream, B4).unwrap();
        assert_eq!(device.access(), Access::ReadOnly);
        device.read_blocks(BlockIndex(1), &mut buf[..4]).unwrap();
        assert_eq!(&buf[..4], &[4, 5, 6, 7]);
        let error = device.write_blocks(BlockIndex(0), &[0; 4]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
    }

    #[test]
    fn cache_writes_back_on_eviction_and_flush() {
        let device = MemDevice::new(counting(16), B4);
        let mut cache = Cache::new(device, 2);

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

        let device = cache.finish().unwrap();
        assert_eq!(&device.get_ref()[..8], &[1, 1, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn cache_rejects_writes_to_read_only_devices() {
        let bytes = counting(8);
        let mut cache = Cache::new(MemDevice::new(&bytes[..], B4), 4);
        assert!(cache.write_blocks(BlockIndex(0), &[0; 4]).is_err());
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

        assert_eq!(
            view.read_at(12, &mut buf).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
        assert!(view.write_at(15, &[0; 2]).is_err());
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
        assert_eq!(
            view.write_all(&[5]).unwrap_err().kind(),
            ErrorKind::WriteZero
        );
        assert_eq!(&view.get_ref().get_ref()[5..], &[1, 2, 3]);
    }
}
