use core::fmt;

use hadris_fs::ErrorKind;
#[cfg(feature = "write")]
use hadris_fs::{DateTime, NoClock};

use hadris_fat_raw::exfat::{MAX_LABEL_UNITS, valid_unit};

/// An exFAT volume label: 1 to 11 UTF-16 code units, stored as given.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct VolumeLabel {
    units: [u16; MAX_LABEL_UNITS],
    len: u8,
}

impl VolumeLabel {
    /// Checks `text`. Fails with `ErrorKind::InvalidInput` when it is empty
    /// or holds a control character or one of `"*/:<>?\|`, which exFAT
    /// forbids in labels as in file names, and with
    /// `ErrorKind::NameTooLong` above 11 UTF-16 code units.
    pub fn new(text: &str) -> Result<Self, ErrorKind> {
        if text.is_empty() {
            return Err(ErrorKind::InvalidInput);
        }
        let mut label = Self {
            units: [0; MAX_LABEL_UNITS],
            len: 0,
        };
        for unit in text.encode_utf16() {
            if !valid_unit(unit) {
                return Err(ErrorKind::InvalidInput);
            }
            if label.len as usize == MAX_LABEL_UNITS {
                return Err(ErrorKind::NameTooLong);
            }
            label.units[label.len as usize] = unit;
            label.len += 1;
        }
        Ok(label)
    }

    /// A label as read from a volume, unchecked.
    #[cfg(all(feature = "alloc", any(feature = "sync", feature = "async")))]
    pub(crate) fn from_disk(units: [u16; MAX_LABEL_UNITS], len: u8) -> Self {
        Self {
            units,
            len: len.min(MAX_LABEL_UNITS as u8),
        }
    }

    /// The label's UTF-16 code units.
    pub fn as_utf16(&self) -> &[u16] {
        &self.units[..self.len as usize]
    }
}

impl TryFrom<&str> for VolumeLabel {
    type Error = ErrorKind;

    /// Checks `text` as [`VolumeLabel::new`] does.
    fn try_from(text: &str) -> Result<Self, ErrorKind> {
        Self::new(text)
    }
}

impl fmt::Display for VolumeLabel {
    /// Writes the label, with unpaired surrogates as U+FFFD.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        for ch in hadris_fat_raw::name::utf16_chars(self.as_utf16().iter().copied()) {
            f.write_char(ch)?;
        }
        Ok(())
    }
}

impl fmt::Debug for VolumeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VolumeLabel({:?})", format_args!("{self}"))
    }
}

/// How `format` lays out an exFAT volume, and how `write` builds one from
/// a tree.
///
/// Every field has a default, so `ExFatOptions::new()` formats any device
/// of at least 1 MiB: 512-byte sectors (or the device's block size when it
/// is 512 to 4096 bytes), 4 KiB clusters below 256 MiB, 32 KiB below
/// 32 GiB and 128 KiB above, the FAT and the cluster heap aligned to 1 MiB
/// on volumes of 64 MiB or more and to the cluster size below, and the
/// recommended up-case table. The volume fills the device, and its
/// partition offset is the device's `disk_offset`.
///
/// The serial derives from the seed, or from the time when there is none,
/// so the same options give the same bytes every time. `write` also stamps
/// the nodes the tree gives no time with the time, [`NoClock::TIME`] by
/// default.
///
/// ```rust
/// # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_fat::exfat::sync::{ExFatFs, format};
/// use hadris_fat::exfat::{ExFatOptions, VolumeLabel};
/// use hadris_fs::MountOptions;
/// use hadris_fs::sync::FileSystem;
/// use hadris_storage::{BlockSize, MemDevice};
///
/// let mut dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
/// let options = ExFatOptions::new()
///     .with_label(VolumeLabel::new("Photos")?)
///     .with_cluster_size(32 * 1024)
///     .with_serial(0x1234_5678);
/// let geometry = format(&mut dev, &options)?;
/// assert_eq!(geometry.volume_serial(), 0x1234_5678);
/// let mut fs = ExFatFs::mount(dev, MountOptions::new())?;
/// let mut buf = [0u8; 64];
/// assert_eq!(fs.label(&mut buf)?, Some("Photos"));
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
/// # fn main() {}
/// ```
#[cfg(feature = "write")]
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub struct ExFatOptions {
    pub(crate) size: Option<u64>,
    pub(crate) label: Option<VolumeLabel>,
    pub(crate) time: DateTime,
    pub(crate) seed: Option<u64>,
    pub(crate) serial: Option<u32>,
    pub(crate) sector_size: Option<u32>,
    pub(crate) cluster_size: Option<u32>,
    pub(crate) fat_count: u8,
    pub(crate) partition_offset: Option<u64>,
    pub(crate) alignment: Option<u32>,
}

#[cfg(feature = "write")]
impl ExFatOptions {
    /// The defaults: sizes chosen from the device, the whole device, no
    /// label, [`NoClock::TIME`], one FAT and the device's partition offset.
    pub const fn new() -> Self {
        Self {
            size: None,
            label: None,
            time: NoClock::TIME,
            seed: None,
            serial: None,
            sector_size: None,
            cluster_size: None,
            fat_count: 1,
            partition_offset: None,
            alignment: None,
        }
    }

    /// Makes the volume `bytes` long, rounded down to whole sectors,
    /// instead of filling the device. A growable device grows to it; any
    /// other fails with `ErrorKind::NoSpace` when it is smaller.
    pub const fn with_size(mut self, bytes: u64) -> Self {
        self.size = Some(bytes);
        self
    }

    /// Writes `label` as the root directory's Volume Label entry.
    pub const fn with_label(mut self, label: VolumeLabel) -> Self {
        self.label = Some(label);
        self
    }

    /// Stamps the nodes `write` copies without times with `time`, and
    /// derives the serial from it when there is no seed.
    pub const fn with_time(mut self, time: DateTime) -> Self {
        self.time = time;
        self
    }

    /// Derives the serial from `seed` instead of the time. `write` mixes in
    /// the tree's paths, sizes and times, so different trees get different
    /// serials.
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Sets the volume serial number instead of deriving it.
    pub const fn with_serial(mut self, serial: u32) -> Self {
        self.serial = Some(serial);
        self
    }

    /// Sets the sector size: 512, 1024, 2048 or 4096 bytes. It need not
    /// match the device's block size.
    pub const fn with_sector_size(mut self, bytes: u32) -> Self {
        self.sector_size = Some(bytes);
        self
    }

    /// Sets the cluster size in bytes: a power of two, at least the sector
    /// size and at most 32 MiB.
    pub const fn with_cluster_size(mut self, bytes: u32) -> Self {
        self.cluster_size = Some(bytes);
        self
    }

    /// Sets the number of FATs: 1, or 2 for a TexFAT volume, which also
    /// gets a second Allocation Bitmap. Both copies are kept equal.
    pub const fn with_fat_count(mut self, count: u8) -> Self {
        self.fat_count = count;
        self
    }

    /// Records the volume as starting `bytes` into its media, as
    /// `PartitionOffset`, instead of the device's `disk_offset`. It must be
    /// a whole number of sectors.
    pub const fn with_partition_offset(mut self, bytes: u64) -> Self {
        self.partition_offset = Some(bytes);
        self
    }

    /// Aligns the FAT and the cluster heap to `bytes` from the start of the
    /// volume: a power of two of at least the sector size.
    pub const fn with_alignment(mut self, bytes: u32) -> Self {
        self.alignment = Some(bytes);
        self
    }
}

#[cfg(feature = "write")]
impl Default for ExFatOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_checked() {
        let label = VolumeLabel::new("Données").unwrap();
        assert_eq!(std::format!("{label}"), "Données");
        assert_eq!(label.as_utf16().len(), 7);
        assert_eq!(VolumeLabel::new("").err(), Some(ErrorKind::InvalidInput));
        assert_eq!(VolumeLabel::new("a/b").err(), Some(ErrorKind::InvalidInput));
        assert_eq!(
            VolumeLabel::new("abcdefghijkl").err(),
            Some(ErrorKind::NameTooLong)
        );
        assert!(VolumeLabel::new("abcdefghijk").is_ok());
    }
}
