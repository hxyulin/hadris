use core::fmt;

use crate::{Clock, CodePage, Cp437, DateTimeError, NoClock};

/// How a filesystem is mounted. One type serves every format, and each
/// format ignores what does not apply to it.
///
/// `MountOptions::new()` mounts read-write where the format and device
/// allow, stamps times from [`NoClock`], reads timestamps without a zone as
/// UTC, reads FAT short names in [`Cp437`], and does not cap the node
/// table. The defaults are the same on every target.
///
/// ```rust
/// use hadris_fs::{Ascii, MountOptions};
///
/// let options = MountOptions::new()
///     .read_only()
///     .with_utc_offset(60)?
///     .with_code_page(&Ascii)
///     .with_node_limit(1024);
/// assert!(options.is_read_only());
/// assert_eq!(options.utc_offset(), Some(60));
/// # Ok::<(), hadris_fs::DateTimeError>(())
/// ```
#[derive(Clone, Copy)]
pub struct MountOptions {
    read_only: bool,
    clock: &'static dyn Clock,
    utc_offset: Option<i16>,
    code_page: &'static dyn CodePage,
    node_limit: Option<usize>,
    backup_boot: bool,
}

impl MountOptions {
    /// The defaults described above.
    pub const fn new() -> Self {
        Self {
            read_only: false,
            clock: &NoClock,
            utc_offset: None,
            code_page: &Cp437,
            node_limit: None,
            backup_boot: false,
        }
    }

    /// Mounts for reading only: the driver never writes to the device, not
    /// even a dirty flag, and writing methods fail with
    /// [`ErrorKind::ReadOnly`](crate::ErrorKind::ReadOnly).
    pub const fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Stamps new and changed nodes with the time from `clock`.
    pub const fn with_clock(mut self, clock: &'static dyn Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Reads and writes timestamps stored without a zone, such as FAT's,
    /// as local time `minutes` east of UTC. Fails when `minutes` is beyond
    /// a day.
    pub const fn with_utc_offset(mut self, minutes: i16) -> Result<Self, DateTimeError> {
        match NoClock::TIME.with_utc_offset_minutes(Some(minutes)) {
            Ok(_) => {
                self.utc_offset = Some(minutes);
                Ok(self)
            }
            Err(err) => Err(err),
        }
    }

    /// Reads and generates FAT short names in `code_page`.
    pub const fn with_code_page(mut self, code_page: &'static dyn CodePage) -> Self {
        self.code_page = code_page;
        self
    }

    /// Caps the nodes a driver keeps pinned or open at once. Past the cap,
    /// `lookup`, `create` and `mkdir` fail with
    /// [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded).
    pub const fn with_node_limit(mut self, nodes: usize) -> Self {
        self.node_limit = Some(nodes);
        self
    }

    /// Mounts from the backup boot structures instead of the primary ones,
    /// where the format has them.
    pub const fn backup_boot(mut self) -> Self {
        self.backup_boot = true;
        self
    }

    /// Whether the mount is read-only.
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The clock.
    pub const fn clock(&self) -> &'static dyn Clock {
        self.clock
    }

    /// The UTC offset of zoneless timestamps in minutes, or `None` for UTC.
    pub const fn utc_offset(&self) -> Option<i16> {
        self.utc_offset
    }

    /// The code page of FAT short names.
    pub const fn code_page(&self) -> &'static dyn CodePage {
        self.code_page
    }

    /// The node cap, or `None` when there is none.
    pub const fn node_limit(&self) -> Option<usize> {
        self.node_limit
    }

    /// Whether to mount from the backup boot structures.
    pub const fn is_backup_boot(&self) -> bool {
        self.backup_boot
    }
}

impl Default for MountOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MountOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MountOptions")
            .field("read_only", &self.read_only)
            .field("utc_offset", &self.utc_offset)
            .field("node_limit", &self.node_limit)
            .field("backup_boot", &self.backup_boot)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ascii;

    #[test]
    fn defaults_and_builders() {
        let options = MountOptions::new();
        assert!(!options.is_read_only());
        assert_eq!(options.utc_offset(), None);
        assert_eq!(options.node_limit(), None);
        assert!(!options.is_backup_boot());
        assert_eq!(options.clock().now(), NoClock::TIME);
        assert_eq!(options.code_page().decode(0x82), '\u{E9}');
        let options = options
            .with_code_page(&Ascii)
            .with_node_limit(2)
            .backup_boot()
            .with_utc_offset(-300)
            .unwrap();
        assert_eq!(options.code_page().decode(0x82), '\u{F782}');
        assert_eq!(options.node_limit(), Some(2));
        assert!(options.is_backup_boot());
        assert_eq!(options.utc_offset(), Some(-300));
        assert!(MountOptions::new().with_utc_offset(i16::MAX).is_err());
    }
}
