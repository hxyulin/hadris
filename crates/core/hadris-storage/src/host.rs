use std::fs::{File, Metadata};
use std::io::{self, Seek, SeekFrom};

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

#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
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
    use core::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405C;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            device: *mut c_void,
            code: u32,
            input: *const c_void,
            input_len: u32,
            output: *mut c_void,
            output_len: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }

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
