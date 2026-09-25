use hadris_fs::ErrorKind;
#[cfg(feature = "write")]
use hadris_fs::{DateTime, NoClock};

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

impl TryFrom<&str> for VolumeLabel {
    type Error = ErrorKind;

    /// Checks `text` as [`VolumeLabel::new`] does.
    fn try_from(text: &str) -> Result<Self, ErrorKind> {
        Self::new(text)
    }
}

/// How `format` lays out a FAT12, FAT16 or FAT32 volume, and how
/// `write` builds one from a tree.
///
/// Every field has a default, so `FatOptions::new()` formats any device
/// large enough: FAT12 below 16 MiB, FAT16 below 512 MiB and FAT32 from
/// there, with the cluster size Microsoft's tools choose for the size,
/// adjusted until the cluster count suits the variant. The sector size is
/// the device's block size when that is 512 to 4096 bytes, else 512. The
/// volume fills the device, and its partition offset is the device's
/// `disk_offset`.
///
/// The time stamps the label entry and, by `write`, every node the tree
/// gives no time. The serial derives from the seed, or from the time when
/// there is none, so the same options give the same bytes every time. The
/// default time is [`NoClock::TIME`].
///
/// ```rust
/// # #[cfg(all(feature = "sync", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_fat::sync::{FatFs, format};
/// use hadris_fat::{FatKind, FatOptions, VolumeLabel};
/// use hadris_fs::MountOptions;
/// use hadris_storage::{BlockSize, MemDevice};
///
/// let mut dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
/// let options = FatOptions::new()
///     .with_kind(FatKind::Fat32)
///     .with_label(VolumeLabel::new("BOOT")?)
///     .with_serial(0x1234_5678);
/// let geometry = format(&mut dev, &options)?;
/// assert_eq!(geometry.kind(), FatKind::Fat32);
/// let fs = FatFs::mount(dev, MountOptions::new())?;
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "std")))]
/// # fn main() {}
/// ```
#[cfg(feature = "write")]
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub struct FatOptions {
    pub(crate) kind: Option<FatKind>,
    pub(crate) size: Option<u64>,
    pub(crate) label: Option<VolumeLabel>,
    pub(crate) time: DateTime,
    pub(crate) seed: Option<u64>,
    pub(crate) serial: Option<u32>,
    pub(crate) sector_size: Option<u32>,
    pub(crate) cluster_size: Option<u32>,
    pub(crate) reserved_sectors: Option<u16>,
    pub(crate) fat_count: u8,
    pub(crate) root_entries: u16,
    pub(crate) media: u8,
    pub(crate) oem_name: [u8; 8],
    pub(crate) partition_offset: Option<u64>,
    pub(crate) alignment: Option<u32>,
}

#[cfg(feature = "write")]
impl FatOptions {
    /// The defaults: the variant and cluster size chosen from the size, the
    /// whole device, no label, [`NoClock::TIME`], OEM name `HADRISFT`, two
    /// FATs, 512 root entries on FAT12/16, media `0xF8` and the device's
    /// partition offset.
    pub const fn new() -> Self {
        Self {
            kind: None,
            size: None,
            label: None,
            time: NoClock::TIME,
            seed: None,
            serial: None,
            sector_size: None,
            cluster_size: None,
            reserved_sectors: None,
            fat_count: 2,
            root_entries: 512,
            media: 0xF8,
            oem_name: *b"HADRISFT",
            partition_offset: None,
            alignment: None,
        }
    }

    /// Formats as `kind` instead of choosing from the size. Fails with
    /// `ErrorKind::NoSpace` or `ErrorKind::LimitExceeded` when no cluster
    /// size gives a cluster count `kind` allows.
    pub const fn with_kind(mut self, kind: FatKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Makes the volume `bytes` long, rounded down to whole sectors,
    /// instead of filling the device. A growable device grows to it; any
    /// other fails with `ErrorKind::NoSpace` when it is smaller.
    pub const fn with_size(mut self, bytes: u64) -> Self {
        self.size = Some(bytes);
        self
    }

    /// Writes `label` to the boot sector and as the root directory's label
    /// entry. Without one the boot sector holds `NO NAME`.
    pub const fn with_label(mut self, label: VolumeLabel) -> Self {
        self.label = Some(label);
        self
    }

    /// Stamps the label entry, and the nodes `write` copies without times,
    /// with `time`, and derives the serial from it when there is no seed.
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

    /// Sets the FAT sector size: 512, 1024, 2048 or 4096 bytes. It need not
    /// match the device's block size.
    pub const fn with_sector_size(mut self, bytes: u32) -> Self {
        self.sector_size = Some(bytes);
        self
    }

    /// Sets the cluster size in bytes: a power of two, a multiple of the
    /// sector size, at most 128 sectors and 32 KiB. It is then not adjusted.
    pub const fn with_cluster_size(mut self, bytes: u32) -> Self {
        self.cluster_size = Some(bytes);
        self
    }

    /// Sets the reserved sectors before the first FAT: at least 1, and at
    /// least 8 on FAT32. The default is 1, or 32 on FAT32.
    pub const fn with_reserved_sectors(mut self, sectors: u16) -> Self {
        self.reserved_sectors = Some(sectors);
        self
    }

    /// Sets the number of FATs, 1 or 2.
    pub const fn with_fat_count(mut self, count: u8) -> Self {
        self.fat_count = count;
        self
    }

    /// Sets the entries of the FAT12/16 root directory, rounded up to fill
    /// whole sectors. FAT32 ignores it.
    pub const fn with_root_entries(mut self, entries: u16) -> Self {
        self.root_entries = entries;
        self
    }

    /// Sets the media descriptor: `0xF0` or `0xF8` to `0xFF`. The default
    /// `0xF8` is a fixed disk.
    pub const fn with_media(mut self, media: u8) -> Self {
        self.media = media;
        self
    }

    /// Sets the 8-byte OEM name of the boot sector.
    pub const fn with_oem_name(mut self, name: [u8; 8]) -> Self {
        self.oem_name = name;
        self
    }

    /// Records the volume as starting `bytes` into its disk, as the hidden
    /// sectors of the boot sector, instead of the device's `disk_offset`.
    /// It must be a whole number of sectors.
    pub const fn with_partition_offset(mut self, bytes: u64) -> Self {
        self.partition_offset = Some(bytes);
        self
    }

    /// Starts the data region `bytes` from the start of the volume, or a
    /// multiple of it, by adding reserved sectors: a power of two of at
    /// least the sector size.
    pub const fn with_alignment(mut self, bytes: u32) -> Self {
        self.alignment = Some(bytes);
        self
    }
}

#[cfg(feature = "write")]
impl Default for FatOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The serial of a volume made at `time`, or from `seed`.
#[cfg(feature = "write")]
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn serial(time: DateTime, seed: Option<u64>) -> u32 {
    match seed {
        Some(seed) => {
            let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            (z as u32) ^ ((z >> 32) as u32)
        }
        None => {
            let seconds = time.unix_seconds() as u64;
            (seconds as u32) ^ ((seconds >> 32) as u32) ^ time.nanoseconds().rotate_left(16)
        }
    }
}
