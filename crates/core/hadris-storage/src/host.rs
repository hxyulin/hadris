//! Host image files and disk devices.

use std::fs::{File, Metadata};
use std::io::{self, Seek, SeekFrom};
#[cfg(feature = "sync")]
use std::path::Path;

#[cfg(feature = "sync")]
use hadris_io::{Error, ErrorKind, ErrorType, Location};

#[cfg(feature = "sync")]
use crate::device::{byte_offset, check_blocks};
#[cfg(feature = "sync")]
use crate::{BlockIndex, BlockSize};

#[cfg(feature = "sync")]
const BLOCK: BlockSize = crate::device::BLOCK_512;

/// A host image file or disk device.
///
/// Regular image files default to 512-byte blocks. Disk devices use their
/// logical block size reported by the OS. Use [`open_with_block_size`](Self::open_with_block_size)
/// or [`with_block_size`](Self::with_block_size) for an image with different
/// geometry or a device whose geometry cannot be queried. Trailing bytes
/// that do not fill a block are not addressable.
///
/// [`open`](Self::open) opens a path read-only. [`new`](Self::new) takes a
/// [`File`] the caller opened, and the device is writable when the file
/// accepts writes. Both measure the size with [`file_len`], so a disk
/// device, whose metadata reports 0 bytes, is measured too; a device whose
/// size cannot be determined is refused, since `block_count` cannot fail.
///
/// A writable regular file grows when written past its end, so
/// `max_block_count` is unbounded for it; a disk device does not grow.
/// Device errors are the `std::io::Error` itself, so `raw_os_error()`
/// survives. A write or flush the OS refuses as read-only or not permitted
/// has kind [`ErrorKind::ReadOnly`], so a driver switches to read-only.
///
/// ```rust
/// use hadris_storage::BlockIndex;
/// use hadris_storage::host::FileDevice;
/// use hadris_storage::sync::BlockDevice;
///
/// # let path = std::env::temp_dir().join(format!("hadris-file-device-doc-{}", std::process::id()));
/// # std::fs::write(&path, [0u8; 1024]).unwrap();
/// let file = std::fs::File::options().read(true).write(true).open(&path)?;
/// let mut dev = FileDevice::new(file)?;
/// assert_eq!(dev.block_count(), 2);
/// dev.write_blocks(BlockIndex::new(2), &[7; 512])?;
/// assert_eq!(dev.block_count(), 3);
///
/// let dev = FileDevice::open(&path)?;
/// assert!(!dev.writable());
/// # std::fs::remove_file(&path).unwrap();
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg(feature = "sync")]
#[derive(Debug)]
pub struct FileDevice {
    file: File,
    len: u64,
    block_size: BlockSize,
    writable: bool,
    grows: bool,
}

#[cfg(feature = "sync")]
impl FileDevice {
    /// Opens the image or device at `path` read-only.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::measure(File::open(path)?, false, None)
    }

    /// Wraps a file opened by the caller, measuring its size.
    ///
    /// The device is writable when the file was opened for writing. Fails
    /// with [`io::ErrorKind::Unsupported`] when the size or logical block
    /// size of a disk device cannot be determined.
    pub fn new(file: File) -> io::Result<Self> {
        let writable = accepts_writes(&file);
        Self::measure(file, writable, None)
    }

    /// Opens a path read-only with the supplied logical block size.
    ///
    /// For disk images, pass the logical sector size of the original disk.
    /// For physical devices, the caller must supply the actual logical block size.
    pub fn open_with_block_size(path: impl AsRef<Path>, block_size: BlockSize) -> io::Result<Self> {
        Self::measure(File::open(path)?, false, Some(block_size))
    }

    /// Wraps an opened file with the supplied logical block size.
    ///
    /// The device is writable when the file accepts writes. For physical
    /// devices, the caller must supply the actual logical block size.
    pub fn with_block_size(file: File, block_size: BlockSize) -> io::Result<Self> {
        let writable = accepts_writes(&file);
        Self::measure(file, writable, Some(block_size))
    }

    /// Recovers the file.
    pub fn into_inner(self) -> File {
        self.file
    }

    fn measure(file: File, writable: bool, explicit: Option<BlockSize>) -> io::Result<Self> {
        let len = file_len(&file)?;
        let meta = file.metadata().ok();
        let detected = disk_block_size(&file, meta.as_ref());
        let grows = detected.is_none() && meta.as_ref().is_some_and(|meta| meta.is_file());
        let block_size = explicit
            .or(detected)
            .or(grows.then_some(BLOCK))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "cannot determine the device's logical block size",
                )
            })?;
        Ok(Self {
            file,
            len,
            block_size,
            writable,
            grows,
        })
    }
}

/// Whether `file` was opened for writing. A write of no bytes transfers
/// nothing, and the OS refuses it on a handle without write access.
#[cfg(feature = "sync")]
fn accepts_writes(file: &File) -> bool {
    io::Write::write(&mut &*file, &[]).is_ok()
}

/// A failed write or flush of `err`, kept as the device error. An OS
/// refusal to write, as on a read-only filesystem or write-protected media,
/// has kind [`ErrorKind::ReadOnly`].
#[cfg(feature = "sync")]
pub(crate) fn write_error(err: io::Error, message: &'static str) -> Error<io::Error> {
    let refused = matches!(
        err.kind(),
        io::ErrorKind::ReadOnlyFilesystem | io::ErrorKind::PermissionDenied
    );
    let err = Error::device(err, message);
    if refused {
        err.with_kind(ErrorKind::ReadOnly)
    } else {
        err
    }
}

#[cfg(feature = "sync")]
impl ErrorType for FileDevice {
    type Error = io::Error;
}

#[cfg(feature = "sync")]
impl crate::sync::BlockDevice for FileDevice {
    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.len / u64::from(self.block_size.get())
    }

    fn max_block_count(&self) -> u64 {
        if self.writable && self.grows {
            u64::MAX / u64::from(self.block_size.get())
        } else {
            self.block_count()
        }
    }

    fn writable(&self) -> bool {
        self.writable
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<io::Error>> {
        check_blocks(self.block_size, self.block_count(), first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| io::Read::read_exact(&mut self.file, buf))
            .map_err(|err| {
                Error::device(err, "reading the file failed")
                    .with_location(Location::Block(first.get()))
            })
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<io::Error>> {
        if !self.writable {
            return Err(Error::new(
                ErrorKind::ReadOnly,
                "file is not open for writing",
            ));
        }
        check_blocks(self.block_size, self.max_block_count(), first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| io::Write::write_all(&mut self.file, buf))
            .map_err(|err| {
                write_error(err, "writing the file failed")
                    .with_location(Location::Block(first.get()))
            })?;
        self.len = self.len.max(offset + buf.len() as u64);
        Ok(())
    }

    /// Calls `sync_data`, so a flush reaches stable storage. A read-only
    /// device has nothing to write and succeeds without syncing.
    fn flush(&mut self) -> Result<(), Error<io::Error>> {
        if !self.writable {
            return Ok(());
        }
        self.file
            .sync_data()
            .map_err(|err| write_error(err, "syncing the file failed"))
    }
}

/// The length in bytes of a host file or disk device.
///
/// A regular file reports its metadata length. A disk device is asked for
/// its size: `DKIOCGETBLOCKCOUNT` and `DKIOCGETBLOCKSIZE` on macOS,
/// `DIOCGMEDIASIZE` on FreeBSD, and `IOCTL_DISK_GET_LENGTH_INFO` on Windows
/// for `\\.\PhysicalDriveN` and volume paths. Anything else, including a
/// Linux block device, is measured by seeking to the end; the position is
/// restored afterwards.
///
/// Fails with [`io::ErrorKind::Unsupported`] when a file that is not a
/// regular file measures 0 bytes, since its size could not be determined.
pub fn file_len(file: &File) -> io::Result<u64> {
    let meta = file.metadata().ok();
    let regular = meta
        .as_ref()
        .filter(|meta| meta.is_file())
        .map(Metadata::len);
    if let Some(len) = regular.filter(|&len| len > 0) {
        return Ok(len);
    }
    if let Some(len) = disk_len(file, meta.as_ref()) {
        return Ok(len);
    }
    if let Some(len) = regular {
        return Ok(len);
    }
    let mut file = file;
    let here = file.stream_position()?;
    let end = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(here))?;
    if end == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot determine the size of a device that seeks to 0",
        ));
    }
    Ok(end)
}

#[cfg(any(
    all(feature = "sync", unix),
    target_vendor = "apple",
    target_os = "freebsd"
))]
fn is_device(meta: Option<&Metadata>) -> bool {
    use std::os::unix::fs::FileTypeExt;
    meta.is_some_and(|meta| {
        let kind = meta.file_type();
        kind.is_block_device() || kind.is_char_device()
    })
}

#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
unsafe extern "C" {
    fn ioctl(fd: core::ffi::c_int, request: core::ffi::c_ulong, ...) -> core::ffi::c_int;
}

#[cfg(target_vendor = "apple")]
fn disk_len(file: &File, meta: Option<&Metadata>) -> Option<u64> {
    use core::ffi::c_ulong;
    use std::os::fd::AsRawFd;

    const DKIOCGETBLOCKSIZE: c_ulong = 0x4004_6418;
    const DKIOCGETBLOCKCOUNT: c_ulong = 0x4008_6419;

    if !is_device(meta) {
        return None;
    }
    let fd = file.as_raw_fd();
    let mut size: u32 = 0;
    let mut count: u64 = 0;
    // SAFETY: `fd` is open for the life of `file`, and each request writes
    // one value of the type its pointer points to.
    let ok = unsafe {
        ioctl(fd, DKIOCGETBLOCKSIZE, &mut size as *mut u32) == 0
            && ioctl(fd, DKIOCGETBLOCKCOUNT, &mut count as *mut u64) == 0
    };
    if ok {
        count.checked_mul(u64::from(size))
    } else {
        None
    }
}

#[cfg(target_os = "freebsd")]
fn disk_len(file: &File, meta: Option<&Metadata>) -> Option<u64> {
    use core::ffi::c_ulong;
    use std::os::fd::AsRawFd;

    const DIOCGMEDIASIZE: c_ulong = 0x4008_6481;

    if !is_device(meta) {
        return None;
    }
    let mut len: i64 = 0;
    // SAFETY: `fd` is open for the life of `file`, and the request writes
    // one `off_t`.
    let ok = unsafe { ioctl(file.as_raw_fd(), DIOCGMEDIASIZE, &mut len as *mut i64) == 0 };
    if ok { u64::try_from(len).ok() } else { None }
}

#[cfg(windows)]
fn disk_len(file: &File, _meta: Option<&Metadata>) -> Option<u64> {
    use std::os::windows::io::AsRawHandle;

    const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405C;

    let mut len: i64 = 0;
    let mut returned: u32 = 0;
    // SAFETY: the handle is open for the life of `file`, and the request
    // writes one `GET_LENGTH_INFO`, a single 64-bit integer, into `len`.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            IOCTL_DISK_GET_LENGTH_INFO,
            core::ptr::null(),
            0,
            (&mut len as *mut i64).cast(),
            8,
            &mut returned,
            core::ptr::null_mut(),
        ) != 0
    };
    if ok && returned == 8 {
        u64::try_from(len).ok()
    } else {
        None
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "freebsd", windows)))]
fn disk_len(_file: &File, _meta: Option<&Metadata>) -> Option<u64> {
    None
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        device: *mut core::ffi::c_void,
        code: u32,
        input: *const core::ffi::c_void,
        input_len: u32,
        output: *mut core::ffi::c_void,
        output_len: u32,
        returned: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
}

#[cfg(all(feature = "sync", unix))]
fn disk_block_size(file: &File, meta: Option<&Metadata>) -> Option<BlockSize> {
    if !is_device(meta) {
        return None;
    }
    query_block_size(file).and_then(BlockSize::new)
}

#[cfg(all(
    feature = "sync",
    any(target_vendor = "apple", target_os = "freebsd", target_os = "linux")
))]
fn query_block_size(file: &File) -> Option<u32> {
    use std::os::fd::AsRawFd;

    #[cfg(target_vendor = "apple")]
    const REQUEST: core::ffi::c_ulong = 0x4004_6418;
    #[cfg(target_os = "freebsd")]
    const REQUEST: core::ffi::c_ulong = 0x4004_6480;
    #[cfg(target_os = "linux")]
    let request = libc::BLKSSZGET;
    #[cfg(not(target_os = "linux"))]
    let request = REQUEST;

    let mut size: u32 = 0;
    // SAFETY: the open descriptor remains valid, and the request writes one
    // 32-bit logical sector size into the supplied pointer.
    #[cfg(target_os = "linux")]
    let ok = unsafe { libc::ioctl(file.as_raw_fd(), request, &mut size as *mut u32) == 0 };
    #[cfg(not(target_os = "linux"))]
    let ok = unsafe { ioctl(file.as_raw_fd(), request, &mut size as *mut u32) == 0 };
    ok.then_some(size)
}

#[cfg(all(
    feature = "sync",
    unix,
    not(any(target_vendor = "apple", target_os = "freebsd", target_os = "linux"))
))]
fn query_block_size(_file: &File) -> Option<u32> {
    None
}

#[cfg(all(feature = "sync", windows))]
fn disk_block_size(file: &File, _meta: Option<&Metadata>) -> Option<BlockSize> {
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    #[derive(Default)]
    struct DiskGeometry {
        cylinders: i64,
        media_type: u32,
        tracks_per_cylinder: u32,
        sectors_per_track: u32,
        bytes_per_sector: u32,
    }

    const IOCTL_DISK_GET_DRIVE_GEOMETRY: u32 = 0x0007_0000;
    let mut geometry = DiskGeometry::default();
    let mut returned = 0;
    let len = core::mem::size_of::<DiskGeometry>() as u32;
    // SAFETY: the handle stays open, and the request writes a DISK_GEOMETRY
    // into an aligned buffer with the matching C layout and length.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            IOCTL_DISK_GET_DRIVE_GEOMETRY,
            core::ptr::null(),
            0,
            (&mut geometry as *mut DiskGeometry).cast(),
            len,
            &mut returned,
            core::ptr::null_mut(),
        ) != 0
    };
    if ok && returned >= len {
        BlockSize::new(geometry.bytes_per_sector)
    } else {
        None
    }
}

#[cfg(all(feature = "sync", not(any(unix, windows))))]
fn disk_block_size(_file: &File, _meta: Option<&Metadata>) -> Option<BlockSize> {
    None
}
