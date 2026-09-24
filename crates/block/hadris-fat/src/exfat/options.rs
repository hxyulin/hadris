use core::fmt;

use hadris_fs::ErrorKind;
#[cfg(feature = "write")]
use hadris_fs::{Clock, NoClock};

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

impl fmt::Display for VolumeLabel {
    /// Writes the label, with unpaired surrogates as U+FFFD.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        for ch in crate::raw::name::utf16_chars(self.as_utf16().iter().copied()) {
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

/// How `format` lays out an exFAT volume on a device.
///
/// Every field has a default, so `FormatOptions::new()` formats any device
/// of at least 1 MiB: 512-byte sectors (or the device's block size when it
/// is 512 to 4096 bytes), 4 KiB clusters below 256 MiB, 32 KiB below
/// 32 GiB and 128 KiB above, the FAT and the cluster heap aligned to 1 MiB
/// on volumes of 64 MiB or more and to the cluster size below, and the
/// recommended up-case table.
///
/// The clock stamps nothing on an empty volume but derives the volume
/// serial number when none is given, so the default [`NoClock`] formats
/// the same device to the same bytes every time.
///
/// ```rust
/// # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_fat::exfat::sync::format;
/// use hadris_fat::exfat::{FormatOptions, VolumeLabel};
/// use hadris_storage::{BlockSize, MemDevice};
///
/// let dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
/// let options = FormatOptions::new()
///     .with_label(VolumeLabel::new("Photos")?)
///     .with_cluster_size(32 * 1024)
///     .with_volume_id(0x1234_5678);
/// let mut fs = format(dev, options)?;
/// assert_eq!(fs.volume_label()?.unwrap().to_string(), "Photos");
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
/// # fn main() {}
/// ```
#[cfg(feature = "write")]
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
#[derive(Clone, Copy)]
pub struct FormatOptions {
    pub(crate) label: Option<VolumeLabel>,
    pub(crate) volume_id: Option<u32>,
    pub(crate) sector_size: Option<u32>,
    pub(crate) cluster_size: Option<u32>,
    pub(crate) alignment: Option<u32>,
    pub(crate) partition_offset: u64,
    pub(crate) fat_count: u8,
    pub(crate) clock: &'static dyn Clock,
}

#[cfg(feature = "write")]
impl core::fmt::Debug for FormatOptions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FormatOptions")
            .field("label", &self.label)
            .field("volume_id", &self.volume_id)
            .field("sector_size", &self.sector_size)
            .field("cluster_size", &self.cluster_size)
            .field("alignment", &self.alignment)
            .field("partition_offset", &self.partition_offset)
            .field("fat_count", &self.fat_count)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "write")]
impl FormatOptions {
    /// The defaults: sizes chosen from the device, no label, a serial
    /// derived from the clock, no partition offset, one FAT and
    /// [`NoClock`].
    pub const fn new() -> Self {
        Self {
            label: None,
            volume_id: None,
            sector_size: None,
            cluster_size: None,
            alignment: None,
            partition_offset: 0,
            fat_count: 1,
            clock: &NoClock,
        }
    }
}

#[cfg(feature = "write")]
impl Default for FormatOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "write")]
impl FormatOptions {
    /// Writes `label` as the root directory's Volume Label entry.
    pub fn with_label(mut self, label: VolumeLabel) -> Self {
        self.label = Some(label);
        self
    }

    /// Sets the volume serial number instead of deriving it from the clock.
    pub fn with_volume_id(mut self, id: u32) -> Self {
        self.volume_id = Some(id);
        self
    }

    /// Sets the sector size: 512, 1024, 2048 or 4096 bytes. It need not
    /// match the device's block size.
    pub fn with_sector_size(mut self, bytes: u32) -> Self {
        self.sector_size = Some(bytes);
        self
    }

    /// Sets the cluster size in bytes: a power of two, at least the sector
    /// size and at most 32 MiB.
    pub fn with_cluster_size(mut self, bytes: u32) -> Self {
        self.cluster_size = Some(bytes);
        self
    }

    /// Aligns the FAT and the cluster heap to `bytes` from the start of the
    /// volume: a power of two of at least the sector size.
    pub fn with_alignment(mut self, bytes: u32) -> Self {
        self.alignment = Some(bytes);
        self
    }

    /// Sets `PartitionOffset`, the sectors before the volume on its media.
    pub fn with_partition_offset(mut self, sectors: u64) -> Self {
        self.partition_offset = sectors;
        self
    }

    /// Sets the number of FATs: 1, or 2 for a TexFAT volume, which also
    /// gets a second Allocation Bitmap. Both copies are kept equal.
    pub fn with_fat_count(mut self, count: u8) -> Self {
        self.fat_count = count;
        self
    }

    /// Uses `clock` for the volume serial number and the returned
    /// `ExFatFs`.
    pub fn with_clock(mut self, clock: &'static dyn Clock) -> Self {
        self.clock = clock;
        self
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
