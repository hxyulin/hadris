use hadris_fs::ErrorKind;
#[cfg(feature = "write")]
use hadris_fs::{Clock, NoClock};

#[cfg(feature = "write")]
use crate::FatKind;

/// A volume label: up to 11 ASCII characters, stored in uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VolumeLabel([u8; 11]);

impl VolumeLabel {
    /// Checks and uppercases `text`. Fails with `ErrorKind::InvalidInput`
    /// when it is empty, starts with a space, or holds a character that is
    /// not ASCII, a control character or one of `"*+,./:;<=>?[\]|`, and with
    /// `ErrorKind::NameTooLong` when it is longer than 11 bytes.
    pub fn new(text: &str) -> Result<Self, ErrorKind> {
        if text.is_empty() || text.starts_with(' ') {
            return Err(ErrorKind::InvalidInput);
        }
        if text.len() > 11 {
            return Err(ErrorKind::NameTooLong);
        }
        let mut label = [b' '; 11];
        for (slot, byte) in label.iter_mut().zip(text.bytes()) {
            if !byte.is_ascii() || byte.is_ascii_control() || b"\"*+,./:;<=>?[\\]|".contains(&byte)
            {
                return Err(ErrorKind::InvalidInput);
            }
            *slot = byte.to_ascii_uppercase();
        }
        Ok(Self(label))
    }

    /// A label as read from a volume, unchecked.
    #[cfg(all(feature = "alloc", any(feature = "sync", feature = "async")))]
    pub(crate) const fn from_disk(bytes: [u8; 11]) -> Self {
        Self(bytes)
    }

    /// The label as stored: 11 bytes padded with spaces.
    pub fn as_bytes(&self) -> &[u8; 11] {
        &self.0
    }

    /// The label without its padding.
    pub fn as_str(&self) -> &str {
        let len = self.0.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
        core::str::from_utf8(&self.0[..len]).unwrap_or_default()
    }
}

/// How `format` lays out a FAT12, FAT16 or FAT32 volume on a device.
///
/// Every field has a default, so `FormatOptions::new()` formats any device
/// large enough: FAT12 below 16 MiB, FAT16 below 512 MiB and FAT32 from
/// there, with the cluster size Microsoft's tools choose for the size,
/// adjusted until the cluster count suits the variant. The sector size is
/// the device's block size when that is 512 to 4096 bytes, else 512.
///
/// The clock stamps the label entry and derives the volume id when none is
/// given, so the default [`NoClock`] formats the same device to the same
/// bytes every time.
///
/// ```rust
/// # #[cfg(all(feature = "sync", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_fat::sync::format;
/// use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
/// use hadris_storage::{BlockSize, MemDevice};
///
/// let dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
/// let options = FormatOptions::new()
///     .with_kind(FatKind::Fat32)
///     .with_label(VolumeLabel::new("BOOT")?)
///     .with_volume_id(0x1234_5678);
/// let fs = format(dev, options)?;
/// assert_eq!(fs.kind(), FatKind::Fat32);
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "std")))]
/// # fn main() {}
/// ```
#[cfg(feature = "write")]
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
#[derive(Clone, Copy)]
pub struct FormatOptions {
    pub(crate) kind: Option<FatKind>,
    pub(crate) label: Option<VolumeLabel>,
    pub(crate) volume_id: Option<u32>,
    pub(crate) sector_size: Option<u32>,
    pub(crate) cluster_size: Option<u32>,
    pub(crate) oem_name: [u8; 8],
    pub(crate) reserved_sectors: Option<u16>,
    pub(crate) hidden_sectors: u32,
    pub(crate) fat_count: u8,
    pub(crate) root_entries: u16,
    pub(crate) media: u8,
    pub(crate) clock: &'static dyn Clock,
}

#[cfg(feature = "write")]
impl core::fmt::Debug for FormatOptions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FormatOptions")
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("volume_id", &self.volume_id)
            .field("sector_size", &self.sector_size)
            .field("cluster_size", &self.cluster_size)
            .field("oem_name", &self.oem_name)
            .field("reserved_sectors", &self.reserved_sectors)
            .field("hidden_sectors", &self.hidden_sectors)
            .field("fat_count", &self.fat_count)
            .field("root_entries", &self.root_entries)
            .field("media", &self.media)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "write")]
impl FormatOptions {
    /// The defaults: the variant and cluster size chosen from the size, no
    /// label, OEM name `HADRISFT`, two FATs, 512 root entries on FAT12/16,
    /// media `0xF8`, no hidden sectors and [`NoClock`].
    pub const fn new() -> Self {
        Self {
            kind: None,
            label: None,
            volume_id: None,
            sector_size: None,
            cluster_size: None,
            oem_name: *b"HADRISFT",
            reserved_sectors: None,
            hidden_sectors: 0,
            fat_count: 2,
            root_entries: 512,
            media: 0xF8,
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
    /// Formats as `kind` instead of choosing from the size. Fails with
    /// `ErrorKind::NoSpace` or `ErrorKind::LimitExceeded` when no cluster
    /// size gives a cluster count `kind` allows.
    pub fn with_kind(mut self, kind: FatKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Writes `label` to the boot sector and as the root directory's label
    /// entry. Without one the boot sector holds `NO NAME`.
    pub fn with_label(mut self, label: VolumeLabel) -> Self {
        self.label = Some(label);
        self
    }

    /// Sets the volume serial number instead of deriving it from the clock.
    pub fn with_volume_id(mut self, id: u32) -> Self {
        self.volume_id = Some(id);
        self
    }

    /// Sets the FAT sector size: 512, 1024, 2048 or 4096 bytes. It need not
    /// match the device's block size.
    pub fn with_sector_size(mut self, bytes: u32) -> Self {
        self.sector_size = Some(bytes);
        self
    }

    /// Sets the cluster size in bytes: a power of two, a multiple of the
    /// sector size, at most 128 sectors and 32 KiB. It is then not adjusted.
    pub fn with_cluster_size(mut self, bytes: u32) -> Self {
        self.cluster_size = Some(bytes);
        self
    }

    /// Sets the 8-byte OEM name of the boot sector.
    pub fn with_oem_name(mut self, name: [u8; 8]) -> Self {
        self.oem_name = name;
        self
    }

    /// Sets the reserved sectors before the first FAT: at least 1, and at
    /// least 8 on FAT32. The default is 1, or 32 on FAT32.
    pub fn with_reserved_sectors(mut self, sectors: u16) -> Self {
        self.reserved_sectors = Some(sectors);
        self
    }

    /// Sets the sectors before the volume on its disk, for a partition.
    pub fn with_hidden_sectors(mut self, sectors: u32) -> Self {
        self.hidden_sectors = sectors;
        self
    }

    /// Sets the number of FATs, 1 or 2.
    pub fn with_fat_count(mut self, count: u8) -> Self {
        self.fat_count = count;
        self
    }

    /// Sets the entries of the FAT12/16 root directory, rounded up to fill
    /// whole sectors. FAT32 ignores it.
    pub fn with_root_entries(mut self, entries: u16) -> Self {
        self.root_entries = entries;
        self
    }

    /// Sets the media descriptor: `0xF0` or `0xF8` to `0xFF`. The default
    /// `0xF8` is a fixed disk.
    pub fn with_media(mut self, media: u8) -> Self {
        self.media = media;
        self
    }

    /// Uses `clock` for the label entry's times, the volume id, and the
    /// returned `FatFs`.
    pub fn with_clock(mut self, clock: &'static dyn Clock) -> Self {
        self.clock = clock;
        self
    }
}
