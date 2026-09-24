use hadris_io::sync::{Read, Seek, Write};

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

use hadris_io::sync::MaybeSend;

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

#[cfg(feature = "std")]
type FileResult = Result<(), hadris_io::Error<std::io::Error>>;

/// Windows refuses `FlushFileBuffers` on a handle without write access.
#[cfg(feature = "std")]
fn is_read_only_handle(err: &std::io::Error) -> bool {
    cfg!(windows) && err.kind() == std::io::ErrorKind::PermissionDenied
}

/// A host file is a block device with 512-byte blocks.
///
/// Device errors are the `std::io::Error` itself, so `raw_os_error()`
/// survives. A file opened read-only fails writes with the OS error. The block count
/// comes from [`file_len`](crate::file_len), so disk devices are measured
/// too; it is 0 when the size cannot be determined.
#[cfg(feature = "std")]
impl BlockDevice for std::fs::File {
    fn block_size(&self) -> crate::BlockSize {
        const { crate::BlockSize::new(512).unwrap() }
    }

    fn block_count(&self) -> u64 {
        crate::file_len(self).map_or(0, |len| len / 512)
    }

    fn read_blocks(&mut self, first: crate::BlockIndex, buf: &mut [u8]) -> FileResult {
        let offset = crate::device::byte_offset(self.block_size(), first)?;
        std::io::Seek::seek(self, std::io::SeekFrom::Start(offset))
            .and_then(|_| std::io::Read::read_exact(self, buf))
            .map_err(|err| {
                hadris_io::Error::device(err, "reading the file failed")
                    .with_location(hadris_io::Location::Block(first.get()))
            })
    }

    fn write_blocks(&mut self, first: crate::BlockIndex, buf: &[u8]) -> FileResult {
        let offset = crate::device::byte_offset(self.block_size(), first)?;
        std::io::Seek::seek(self, std::io::SeekFrom::Start(offset))
            .and_then(|_| std::io::Write::write_all(self, buf))
            .map_err(|err| {
                hadris_io::Error::device(err, "writing the file failed")
                    .with_location(hadris_io::Location::Block(first.get()))
            })
    }

    /// Calls `sync_data`, so a flush reaches stable storage. On Windows a
    /// file opened read-only has nothing to write and succeeds without
    /// syncing.
    fn flush(&mut self) -> FileResult {
        match self.sync_data() {
            Ok(()) => Ok(()),
            Err(err) if is_read_only_handle(&err) => Ok(()),
            Err(err) => Err(hadris_io::Error::device(err, "syncing the file failed")),
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod device_tests {
    use hadris_io::sync::{Read as _, Seek as _, Write as _};
    use hadris_io::{Error, ErrorKind, ErrorType, Location, SeekFrom, StdIo};

    use crate::sync::{BlockDevice, ByteView, Cache, Slice, StreamDevice};
    use crate::{BlockIndex, BlockSize, MemDevice, ReadOnly};

    fn kind<T, E>(result: Result<T, Error<E>>) -> ErrorKind {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(err) => err.kind(),
        }
    }

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
        device.read_blocks(BlockIndex::new(1), &mut buf).unwrap();
        assert_eq!(buf, [4, 5, 6, 7, 8, 9, 10, 11]);

        device.write_blocks(BlockIndex::new(3), &[9; 4]).unwrap();
        assert_eq!(&device.get_ref()[12..18], &[9, 9, 9, 9, 16, 17]);

        let err = device
            .read_blocks(BlockIndex::new(3), &mut buf)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(err.location(), Some(Location::Block(3)));
        assert_eq!(
            kind(device.read_blocks(BlockIndex::new(0), &mut buf[..3])),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(device.read_blocks(BlockIndex::new(u64::MAX), &mut buf)),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(device.write_blocks(BlockIndex::new(4), &[0; 4])),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn read_only_buffers_reject_writes() {
        let bytes = counting(8);
        let mut device = MemDevice::new(&bytes[..], B4);
        assert_eq!(
            kind(device.write_blocks(BlockIndex::new(0), &[0; 4])),
            ErrorKind::ReadOnly
        );
    }

    /// A device that implements no write method.
    struct Rom<'a>(&'a [u8]);

    impl ErrorType for Rom<'_> {
        type Error = core::convert::Infallible;
    }

    impl BlockDevice for Rom<'_> {
        fn block_size(&self) -> BlockSize {
            B4
        }

        fn block_count(&self) -> u64 {
            self.0.len() as u64 / 4
        }

        fn read_blocks(
            &mut self,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            let at = first.get() as usize * 4;
            let bytes = self.0.get(at..at + buf.len());
            buf.copy_from_slice(bytes.ok_or(Error::new(ErrorKind::InvalidInput, "past the end"))?);
            Ok(())
        }
    }

    #[test]
    fn read_only_devices_need_no_write_method() {
        let bytes = counting(8);
        let mut rom = Rom(&bytes);
        assert_eq!(
            kind(rom.write_blocks(BlockIndex::new(0), &[0; 4])),
            ErrorKind::ReadOnly
        );
        assert_eq!(rom.flush(), Ok(()));
    }

    #[test]
    fn slice_offsets_and_bounds() {
        let mut device = MemDevice::new(counting(16), B4);
        assert!(Slice::new(&mut device, BlockIndex::new(3), 2).is_err());

        let mut slice = Slice::new(&mut device, BlockIndex::new(1), 2).unwrap();
        assert_eq!(slice.block_count(), 2);
        let mut buf = [0_u8; 4];
        slice.read_blocks(BlockIndex::new(1), &mut buf).unwrap();
        assert_eq!(buf, [8, 9, 10, 11]);
        let err = slice.read_blocks(BlockIndex::new(2), &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(err.location(), Some(Location::Block(2)));
        assert_eq!(
            kind(slice.write_blocks(BlockIndex::new(2), &buf)),
            ErrorKind::InvalidInput
        );

        slice.write_blocks(BlockIndex::new(0), &[0xAA; 4]).unwrap();
        assert_eq!(&device.get_ref()[4..8], &[0xAA; 4]);
    }

    #[test]
    fn stream_device_over_std_and_read_only_streams() {
        let stream = StdIo::new(std::io::Cursor::new(counting(10)));
        let mut device = StreamDevice::new(stream, B4).unwrap();
        assert_eq!(device.block_count(), 2);
        device.write_blocks(BlockIndex::new(1), &[7; 4]).unwrap();
        let mut buf = [0_u8; 8];
        device.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        assert_eq!(buf, [0, 1, 2, 3, 7, 7, 7, 7]);
        assert_eq!(
            kind(device.write_blocks(BlockIndex::new(2), &[0; 4])),
            ErrorKind::InvalidInput
        );

        let bytes = counting(8);
        let stream = ReadOnly::new(hadris_io::Cursor::new(&bytes));
        let mut device = StreamDevice::new(stream, B4).unwrap();
        device
            .read_blocks(BlockIndex::new(1), &mut buf[..4])
            .unwrap();
        assert_eq!(&buf[..4], &[4, 5, 6, 7]);
        assert_eq!(
            kind(device.write_blocks(BlockIndex::new(0), &[0; 4])),
            ErrorKind::ReadOnly
        );
    }

    #[test]
    fn stream_device_reports_short_streams_at_their_block() {
        let stream = StdIo::new(std::io::Cursor::new(counting(8)));
        let mut device = StreamDevice::with_block_count(stream, B4, 4);
        let err = device
            .read_blocks(BlockIndex::new(3), &mut [0; 4])
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(err.location(), Some(Location::Block(3)));
        let err: std::io::Error = err.into();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn cache_writes_back_on_eviction_and_flush() {
        let device = MemDevice::new(counting(16), B4);
        let mut cache = Cache::new(device, 2);

        cache.write_blocks(BlockIndex::new(3), &[3; 4]).unwrap();
        assert!(!cache.is_dirty());
        assert_eq!(&cache.get_ref().get_ref()[12..], &[3; 4]);

        cache.write_blocks(BlockIndex::new(0), &[1; 4]).unwrap();
        cache.write_blocks(BlockIndex::new(1), &[2; 4]).unwrap();
        assert!(cache.is_dirty());
        assert_eq!(&cache.get_ref().get_ref()[..8], &counting(8)[..]);

        let mut buf = [0_u8; 4];
        cache.read_blocks(BlockIndex::new(1), &mut buf).unwrap();
        cache.read_blocks(BlockIndex::new(2), &mut buf).unwrap();
        assert_eq!(buf, [8, 9, 10, 11]);
        assert_eq!(&cache.get_ref().get_ref()[..4], &[1; 4]);
        assert_eq!(&cache.get_ref().get_ref()[4..8], &[4, 5, 6, 7]);

        let mut all = [0_u8; 16];
        cache.read_blocks(BlockIndex::new(0), &mut all).unwrap();
        assert_eq!(&all[..8], &[1, 1, 1, 1, 2, 2, 2, 2]);
        assert_eq!(
            kind(cache.read_blocks(BlockIndex::new(4), &mut buf)),
            ErrorKind::InvalidInput
        );

        let device = cache.finish().unwrap();
        assert_eq!(&device.get_ref()[..8], &[1, 1, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn cache_reports_read_only_on_the_first_write() {
        let bytes = counting(8);
        let mut cache = Cache::new(MemDevice::new(&bytes[..], B4), 4);
        assert_eq!(
            kind(cache.write_blocks(BlockIndex::new(0), &[0; 4])),
            ErrorKind::ReadOnly
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

        let err = view.read_at(12, &mut buf).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(err.location(), Some(Location::Byte(12)));
        assert_eq!(kind(view.write_at(15, &[0; 2])), ErrorKind::InvalidInput);
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
        assert!(matches!(
            view.write_all(&[5]),
            Err(hadris_io::ExactError::WriteZero)
        ));
        assert_eq!(&view.get_ref().get_ref()[5..], &[1, 2, 3]);

        let bytes = counting(8);
        let mut view = ByteView::new(MemDevice::new(&bytes[..], B4));
        assert_eq!(kind(view.write(&[1])), ErrorKind::ReadOnly);
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
        file.write_blocks(BlockIndex::new(1), &[9; 512]).unwrap();
        let mut block = [0u8; 512];
        file.read_blocks(BlockIndex::new(1), &mut block).unwrap();
        assert_eq!(block, [9; 512]);
        file.flush().unwrap();

        let mut read_only = std::fs::File::open(&path).unwrap();
        read_only.flush().unwrap();
        let err = read_only
            .write_blocks(BlockIndex::new(0), &block)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Io);
        assert!(err.device_error().unwrap().raw_os_error().is_some());
        std::fs::remove_file(&path).unwrap();
    }

    /// A disk image attached with `hdiutil` is a device node whose length
    /// `stat` and `lseek` report as 0. Skips when it cannot be attached.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn apple_disk_devices_are_measured() {
        let path = std::env::temp_dir().join(std::format!(
            "hadris-storage-disk-{}.img",
            std::process::id()
        ));
        let mut image = std::vec![0u8; 1 << 20];
        image[512..1024].fill(7);
        std::fs::write(&path, &image).unwrap();
        let attached = std::process::Command::new("hdiutil")
            .args([
                "attach",
                "-nomount",
                "-imagekey",
                "diskimage-class=CRawDiskImage",
            ])
            .arg(&path)
            .output();
        let device = attached
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| {
                std::string::String::from_utf8_lossy(&out.stdout)
                    .split_whitespace()
                    .next()
                    .map(std::string::ToString::to_string)
            });
        let Some(device) = device else {
            std::fs::remove_file(&path).unwrap();
            std::eprintln!("skipped: hdiutil cannot attach an image");
            return;
        };
        let measured = std::fs::File::open(&device).map(|mut file| {
            let mut block = [0u8; 512];
            let read = file
                .read_blocks(BlockIndex::new(1), &mut block)
                .map(|()| block);
            (file.block_count(), read.ok())
        });
        let _ = std::process::Command::new("hdiutil")
            .args(["detach", &device])
            .output();
        std::fs::remove_file(&path).unwrap();
        let (count, block) = measured.unwrap();
        assert_eq!(count, 2048);
        assert_eq!(block, Some([7u8; 512]));
    }

    /// A regular file measures its length, empty or not; a device that
    /// seeks to 0 and answers no size request is an error, not 0 bytes.
    #[test]
    fn file_len_measures_or_refuses() {
        let path = std::env::temp_dir().join(std::format!(
            "hadris-storage-len-{}.img",
            std::process::id()
        ));
        std::fs::write(&path, [1u8; 1000]).unwrap();
        assert_eq!(
            crate::file_len(&std::fs::File::open(&path).unwrap()).unwrap(),
            1000
        );
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            crate::file_len(&std::fs::File::open(&path).unwrap()).unwrap(),
            0
        );
        std::fs::remove_file(&path).unwrap();

        #[cfg(unix)]
        {
            let null = std::fs::File::open("/dev/null").unwrap();
            let err = crate::file_len(&null).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
            assert_eq!(null.block_count(), 0);
        }
    }

    /// The disk device CI attaches (a loop device on Linux, an `md` device
    /// on FreeBSD, a VHD on Windows) at `HADRIS_TEST_DISK`, holding
    /// `HADRIS_TEST_DISK_LEN` bytes. Skips when unset, unless
    /// `HADRIS_REQUIRE_TEST_DISK` is set.
    #[test]
    fn attached_disk_device_is_measured() {
        let Some(disk) = std::env::var_os("HADRIS_TEST_DISK") else {
            assert!(
                std::env::var_os("HADRIS_REQUIRE_TEST_DISK").is_none(),
                "HADRIS_REQUIRE_TEST_DISK is set but HADRIS_TEST_DISK is not"
            );
            std::eprintln!("skipped: HADRIS_TEST_DISK is not set");
            return;
        };
        let len: u64 = std::env::var("HADRIS_TEST_DISK_LEN")
            .expect("HADRIS_TEST_DISK_LEN")
            .parse()
            .unwrap();
        let mut file = std::fs::File::open(&disk).unwrap();
        assert_eq!(crate::file_len(&file).unwrap(), len);
        assert_eq!(file.block_count(), len / 512);
        let mut block = [0u8; 512];
        file.read_blocks(BlockIndex::new(len / 512 - 1), &mut block)
            .unwrap();
    }
}
