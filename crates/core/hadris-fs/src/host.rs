//! The host tier (`std`): the host's defaults for mounting and
//! reproducible builds and, with `sync`, trees from and to host directories
//! and host files as tree content.
//!
//! Errors are [`PathError`]s that carry the tree path and the host path.
//!
//! ```rust,no_run
//! # #[cfg(feature = "sync")] {
//! use hadris_fs::host::{self, TreeOptions};
//!
//! let options = match host::source_date_epoch()? {
//!     Some(epoch) => TreeOptions::new().with_clamp(epoch),
//!     None => TreeOptions::new(),
//! };
//! let (tree, skipped) = host::read_tree("rootfs", &options)?;
//! assert!(skipped.is_empty());
//! let report = host::write_tree("copy-of-rootfs", &tree)?;
//! for warning in report.warnings() {
//!     eprintln!("{warning}");
//! }
//! # }
//! # Ok::<(), hadris_fs::PathError>(())
//! ```

#[cfg(feature = "sync")]
mod tree;

#[cfg(feature = "sync")]
pub use tree::{OnError, Symlinks, TreeOptions, file, read_tree, write_tree};

use crate::{DateTime, ErrorKind, MountOptions, PathError, SystemClock};

/// `SOURCE_DATE_EPOCH` from the environment, the time reproducible builds
/// pass to `with_time` and `TreeOptions::with_clamp`. `None` when it is
/// not set; [`ErrorKind::InvalidInput`] when it is not a decimal number of
/// seconds.
pub fn source_date_epoch() -> Result<Option<DateTime>, PathError> {
    let Some(value) = std::env::var_os("SOURCE_DATE_EPOCH") else {
        return Ok(None);
    };
    let bad = || {
        PathError::new(
            ErrorKind::InvalidInput,
            "SOURCE_DATE_EPOCH is not a decimal number of seconds",
        )
    };
    let text = value.to_str().ok_or_else(bad)?;
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad());
    }
    let seconds: i64 = text.parse().map_err(|_| bad())?;
    DateTime::from_unix_seconds(seconds)
        .map(Some)
        .map_err(|_| bad())
}

/// The host's defaults for mounting: [`MountOptions::new`] with
/// [`SystemClock`] and the host's current UTC offset for FAT times, as
/// Windows and Linux vfat assume. Each stays overridable with the `with_`
/// methods.
pub fn mount_options() -> MountOptions {
    static CLOCK: SystemClock = SystemClock;
    let options = MountOptions::new().with_clock(&CLOCK);
    options
        .clone()
        .with_utc_offset(local_utc_offset())
        .unwrap_or(options)
}

/// The host's current offset from UTC in minutes, east positive. 0 when the
/// host cannot tell.
pub fn local_utc_offset() -> i16 {
    zone::offset_minutes().clamp(-24 * 60, 24 * 60)
}

#[cfg(all(
    unix,
    any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    )
))]
mod zone {
    pub(super) fn offset_minutes() -> i16 {
        let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
            return 0;
        };
        let Ok(now) = libc::time_t::try_from(now.as_secs()) else {
            return 0;
        };
        let mut tm = core::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: `localtime_r` reads `now` and fills `tm`, both valid for
        // the call; it returns null without writing when it fails.
        let filled = unsafe { libc::localtime_r(&now, tm.as_mut_ptr()) };
        if filled.is_null() {
            return 0;
        }
        // SAFETY: `localtime_r` succeeded, so it initialized `tm`.
        let tm = unsafe { tm.assume_init() };
        i16::try_from(tm.tm_gmtoff / 60).unwrap_or(0)
    }
}

#[cfg(windows)]
mod zone {
    #[repr(C)]
    struct SystemTime([u16; 8]);

    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTimeZoneInformation(info: *mut TimeZoneInformation) -> u32;
    }

    pub(super) fn offset_minutes() -> i16 {
        let mut info = TimeZoneInformation {
            bias: 0,
            standard_name: [0; 32],
            standard_date: SystemTime([0; 8]),
            standard_bias: 0,
            daylight_name: [0; 32],
            daylight_date: SystemTime([0; 8]),
            daylight_bias: 0,
        };
        // SAFETY: `info` is a valid, writable TIME_ZONE_INFORMATION.
        let id = unsafe { GetTimeZoneInformation(&mut info) };
        let bias = match id {
            1 => info.bias + info.standard_bias,
            2 => info.bias + info.daylight_bias,
            u32::MAX => return 0,
            _ => info.bias,
        };
        i16::try_from(-bias).unwrap_or(0)
    }
}

#[cfg(not(any(
    windows,
    all(
        unix,
        any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly",
        )
    )
)))]
mod zone {
    pub(super) fn offset_minutes() -> i16 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_offset_is_within_a_day() {
        let offset = local_utc_offset();
        assert!((-24 * 60..=24 * 60).contains(&offset));
        assert_eq!(mount_options().utc_offset(), Some(offset));
    }
}
