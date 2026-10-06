use alloc::vec::Vec;

use hadris_fs::ErrorKind;
use hadris_storage::BlockSize;

use crate::codec::{FIRST_LOGICAL, check_block_size};
use crate::error::{Detail, TableError};
use crate::{
    Disk, Gpt, GptEntry, Guid, Hybrid, HybridMbr, Mbr, MbrEntry, MbrType, PartitionFlags,
    PartitionKind, PartitionName,
};

/// The size of a partition in a [`DiskLayout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Size {
    /// A number of blocks.
    Blocks(u64),
    /// A number of bytes, rounded up to whole blocks.
    Bytes(u64),
    /// KiB (1024 bytes), rounded up to whole blocks.
    KiB(u64),
    /// MiB, rounded up to whole blocks.
    MiB(u64),
    /// GiB, rounded up to whole blocks.
    GiB(u64),
    /// Everything up to the end of the usable area. Only the last partition
    /// may take it.
    Remaining,
}

impl Size {
    fn blocks(self, block_size: BlockSize) -> Result<Option<u64>, TableError> {
        let bytes = match self {
            Self::Blocks(blocks) => return Ok(Some(blocks)),
            Self::Remaining => return Ok(None),
            Self::Bytes(n) => Some(n),
            Self::KiB(n) => n.checked_mul(1 << 10),
            Self::MiB(n) => n.checked_mul(1 << 20),
            Self::GiB(n) => n.checked_mul(1 << 30),
        };
        let bytes = bytes.ok_or(TableError::new(ErrorKind::LimitExceeded, Detail::Size))?;
        Ok(Some(bytes.div_ceil(u64::from(block_size.get()))))
    }
}

/// Where a [`DiskLayout`] starts each partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Alignment {
    /// Right after the previous partition.
    Block,
    /// On a 1 MiB boundary, as current partitioning tools do.
    #[default]
    MiB1,
    /// On a multiple of this many blocks.
    Blocks(u64),
}

impl Alignment {
    fn blocks(self, block_size: BlockSize) -> Result<u64, TableError> {
        match self {
            Self::Block => Ok(1),
            Self::MiB1 => Ok(((1 << 20) / u64::from(block_size.get())).max(1)),
            Self::Blocks(0) => Err(TableError::invalid(Detail::Size)),
            Self::Blocks(n) => Ok(n),
        }
    }
}

/// One partition of a [`DiskLayout`].
///
/// The kind is a GPT type GUID for GPT and hybrid layouts and an
/// [`MbrType`] for MBR layouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionSpec {
    kind: PartitionKind,
    size: Size,
    start: Option<u64>,
    name: Option<Result<PartitionName, TableError>>,
    unique_guid: Option<Guid>,
    flags: PartitionFlags,
    mirror: Option<MbrType>,
}

impl PartitionSpec {
    /// A partition of type `kind` and size `size`.
    pub fn new(kind: impl Into<PartitionKind>, size: Size) -> Self {
        Self {
            kind: kind.into(),
            size,
            start: None,
            name: None,
            unique_guid: None,
            flags: PartitionFlags::empty(),
            mirror: None,
        }
    }

    /// Names a GPT partition. A name that [`PartitionName::new`] refuses
    /// fails [`DiskLayout::build`].
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(PartitionName::new(name));
        self
    }

    /// Sets the unique GUID of a GPT partition. Without it, the GUID is
    /// derived from the disk GUID and the partition's position, so a layout
    /// always gives the same disk.
    pub fn with_unique_guid(mut self, unique_guid: Guid) -> Self {
        self.unique_guid = Some(unique_guid);
        self
    }

    /// Sets the flags.
    pub fn with_flags(mut self, flags: PartitionFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Starts the partition at block `start` instead of the next aligned
    /// block.
    pub fn with_start(mut self, start: u64) -> Self {
        self.start = Some(start);
        self
    }

    /// Mirrors the partition in the MBR of a hybrid layout with type
    /// `kind`; at most three partitions can be mirrored.
    pub fn with_mirror(mut self, kind: MbrType) -> Self {
        self.mirror = Some(kind);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Mbr,
    Gpt(Guid),
    Hybrid(Guid),
}

/// A partition layout, turned into a [`Disk`] for a device's geometry.
///
/// Partitions are placed in order, each at the next aligned block unless it
/// gives its own start. An MBR layout with more than four partitions puts
/// the fourth and later ones in an extended partition.
///
/// ```rust
/// use hadris_part::gpt::types;
/// use hadris_part::{Alignment, DiskLayout, Guid, PartitionSpec, Size};
/// use hadris_storage::BlockSize;
///
/// let disk = DiskLayout::gpt(Guid::from_bytes([1; 16]))
///     .with_alignment(Alignment::MiB1)
///     .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(100)).with_name("EFI"))
///     .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining))
///     .build(1 << 21, BlockSize::new(512).unwrap())
///     .unwrap();
/// let root = disk.partition(1).unwrap();
/// assert_eq!(root.start(), 206_848);
/// assert_eq!(root.end(), (1 << 21) - 33);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskLayout {
    scheme: Scheme,
    alignment: Alignment,
    partitions: Vec<PartitionSpec>,
}

impl DiskLayout {
    /// An MBR layout.
    pub fn mbr() -> Self {
        Self::with_scheme(Scheme::Mbr)
    }

    /// A GPT layout with disk GUID `disk_guid`.
    pub fn gpt(disk_guid: Guid) -> Self {
        Self::with_scheme(Scheme::Gpt(disk_guid))
    }

    /// A GPT layout whose MBR mirrors the partitions given
    /// [`PartitionSpec::with_mirror`], with the protective entry in slot 0.
    pub fn hybrid(disk_guid: Guid) -> Self {
        Self::with_scheme(Scheme::Hybrid(disk_guid))
    }

    fn with_scheme(scheme: Scheme) -> Self {
        Self {
            scheme,
            alignment: Alignment::default(),
            partitions: Vec::new(),
        }
    }

    /// Sets the alignment of partition starts; the default is 1 MiB.
    pub fn with_alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Appends a partition.
    pub fn partition(mut self, spec: PartitionSpec) -> Self {
        self.partitions.push(spec);
        self
    }

    /// Places the partitions on a disk of `block_count` blocks of
    /// `block_size` bytes.
    ///
    /// Fails with [`ErrorKind::NoSpace`] when they do not fit,
    /// [`ErrorKind::InvalidInput`] for a kind of the wrong table, a flag the
    /// table cannot store, an overlap from an explicit start, a zero size or
    /// a `Remaining` size before the last partition, and with the errors of
    /// [`Gpt`], [`Mbr`] and [`Hybrid`] edits.
    pub fn build(&self, block_count: u64, block_size: BlockSize) -> Result<Disk, TableError> {
        check_block_size(block_size)?;
        let align = self.alignment.blocks(block_size)?;
        let last = self.partitions.len().saturating_sub(1);
        for (i, spec) in self.partitions.iter().enumerate() {
            if spec.size == Size::Remaining && i != last {
                return Err(TableError::invalid(Detail::Size));
            }
        }
        match self.scheme {
            Scheme::Mbr => self
                .build_mbr(block_count, block_size, align)
                .map(Disk::new),
            Scheme::Gpt(guid) => self
                .build_gpt(guid, block_count, block_size, align)
                .map(Disk::new),
            Scheme::Hybrid(guid) => {
                let gpt = self.build_gpt(guid, block_count, block_size, align)?;
                let mut config = HybridMbr::new();
                for (index, spec) in self.partitions.iter().enumerate() {
                    if let Some(kind) = spec.mirror {
                        config.add_mirrored(index, kind, spec.flags & PartitionFlags::BOOTABLE)?;
                    }
                }
                Hybrid::new(gpt, &config).map(Disk::new)
            }
        }
    }

    fn build_gpt(
        &self,
        disk_guid: Guid,
        block_count: u64,
        block_size: BlockSize,
        align: u64,
    ) -> Result<Gpt, TableError> {
        let mut gpt = Gpt::new(disk_guid, block_count, block_size)?;
        let mut cursor = gpt.first_usable();
        let end = gpt.last_usable() + 1;
        for (index, spec) in self.partitions.iter().enumerate() {
            let PartitionKind::Gpt(type_guid) = spec.kind else {
                return Err(TableError::invalid(Detail::Kind));
            };
            if spec.mirror.is_some() && !matches!(self.scheme, Scheme::Hybrid(_)) {
                return Err(TableError::invalid(Detail::Mirror));
            }
            let start = spec.start.unwrap_or(align_up(cursor, align));
            let len = length(spec.size, block_size, start, end)?;
            let name = spec.name.unwrap_or(Ok(PartitionName::EMPTY))?;
            let unique = spec
                .unique_guid
                .unwrap_or_else(|| disk_guid.derive(index as u64));
            let entry = GptEntry::new(type_guid, unique, start, len)
                .with_name(name)
                .with_flags(spec.flags);
            gpt.add(entry).map_err(fit(spec))?;
            cursor = start + len;
        }
        Ok(gpt)
    }

    fn build_mbr(
        &self,
        block_count: u64,
        block_size: BlockSize,
        align: u64,
    ) -> Result<Mbr, TableError> {
        let mut mbr = Mbr::new(block_count, block_size)?;
        for spec in &self.partitions {
            let PartitionKind::Mbr(kind) = spec.kind else {
                return Err(TableError::invalid(Detail::Kind));
            };
            if spec.name.is_some() || spec.unique_guid.is_some() || spec.mirror.is_some() {
                return Err(TableError::invalid(Detail::Kind));
            }
            if kind.is_extended() {
                return Err(TableError::invalid(Detail::Extended));
            }
        }
        let primaries = if self.partitions.len() > 4 {
            3
        } else {
            self.partitions.len()
        };
        let mut cursor = align;
        let end = block_count;
        for spec in &self.partitions[..primaries] {
            let start = spec.start.unwrap_or(align_up(cursor, align));
            let len = length(spec.size, block_size, start, end)?;
            let len = match spec.size {
                Size::Remaining => len.min(u64::from(u32::MAX)),
                _ => len,
            };
            mbr.add(entry(spec, start, len)).map_err(fit(spec))?;
            cursor = start + len;
        }
        let logicals = &self.partitions[primaries..];
        if logicals.is_empty() {
            return Ok(mbr);
        }
        let ext_start = align_up(cursor, align);
        let mut placed = Vec::with_capacity(logicals.len());
        let mut ebr = ext_start;
        for (k, spec) in logicals.iter().enumerate() {
            let start = spec.start.unwrap_or(align_up(ebr.saturating_add(1), align));
            if start <= ext_start {
                let index = FIRST_LOGICAL + k;
                return Err(TableError::invalid(Detail::OutOfBounds).at(index));
            }
            let len = length(spec.size, block_size, start, end)?;
            let len = match spec.size {
                Size::Remaining => {
                    let room = u64::from(u32::MAX).saturating_sub(start - ext_start);
                    len.min(room.max(1))
                }
                _ => len,
            };
            placed.push((entry(spec, start, len), spec));
            ebr = start + len;
        }
        let ext_end = placed
            .iter()
            .map(|(e, _)| e.start() + e.len())
            .max()
            .unwrap_or(ebr);
        let extended = MbrEntry::new(MbrType::EXTENDED_LBA, ext_start, ext_end - ext_start);
        mbr.add(extended)
            .map_err(fit(&self.partitions[primaries]))?;
        for (logical, spec) in placed {
            mbr.add_logical(logical).map_err(fit(spec))?;
        }
        Ok(mbr)
    }
}

fn entry(spec: &PartitionSpec, start: u64, len: u64) -> MbrEntry {
    let PartitionKind::Mbr(kind) = spec.kind else {
        return MbrEntry::new(MbrType::EMPTY, start, len);
    };
    MbrEntry::new(kind, start, len).with_flags(spec.flags)
}

const fn align_up(block: u64, align: u64) -> u64 {
    match block.checked_next_multiple_of(align) {
        Some(aligned) => aligned,
        None => u64::MAX,
    }
}

/// The length of a partition of size `size` from `start`, with `end` the
/// block after the usable area.
fn length(size: Size, block_size: BlockSize, start: u64, end: u64) -> Result<u64, TableError> {
    let too_small = TableError::new(ErrorKind::NoSpace, Detail::DiskTooSmall);
    let len = match size.blocks(block_size)? {
        Some(0) => return Err(TableError::invalid(Detail::Size)),
        Some(len) => len,
        None => end.checked_sub(start).filter(|&n| n > 0).ok_or(too_small)?,
    };
    match start.checked_add(len) {
        Some(stop) if stop <= end => Ok(len),
        _ => Err(too_small),
    }
}

/// A partition the layout placed itself past the usable area means the
/// disk is too small; one the caller placed is invalid input.
fn fit(spec: &PartitionSpec) -> impl Fn(TableError) -> TableError {
    let placed = spec.start.is_none();
    move |err| match err.detail() {
        Detail::OutOfBounds if placed => TableError::new(ErrorKind::NoSpace, Detail::DiskTooSmall),
        _ => err,
    }
}
