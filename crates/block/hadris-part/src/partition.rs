use hadris_storage::BlockSize;

use crate::raw::{self, RawGptEntry, RawMbrEntry};
use crate::{Guid, MbrType, PartitionName};

/// One of the two copies of a GPT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GptCopy {
    /// The header at block 1 and its entry array.
    Primary,
    /// The header in the last block and its entry array.
    Backup,
}

/// The kind of partition table on a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TableKind {
    /// An MBR, possibly with logical partitions.
    Mbr,
    /// A GPT behind a protective MBR.
    Gpt,
    /// A GPT behind a hybrid MBR.
    Hybrid,
}

/// The type of a partition, in its table's own terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PartitionKind {
    /// An MBR type code.
    Mbr(MbrType),
    /// A GPT partition type GUID.
    Gpt(Guid),
}

impl From<MbrType> for PartitionKind {
    fn from(kind: MbrType) -> Self {
        Self::Mbr(kind)
    }
}

impl From<Guid> for PartitionKind {
    fn from(kind: Guid) -> Self {
        Self::Gpt(kind)
    }
}

/// Flags of a partition that both tables can express, at least in part.
///
/// An MBR stores only [`BOOTABLE`](Self::BOOTABLE), as the active flag.
/// A GPT stores all three in attribute bits 2, 0 and 1.
///
/// Combine flags with `|` or [`union`](Self::union).
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PartitionFlags(u32);

impl PartitionFlags {
    /// Active (MBR) or legacy BIOS bootable (GPT).
    pub const BOOTABLE: Self = Self(1 << 0);
    /// Required by the platform (GPT).
    pub const REQUIRED: Self = Self(1 << 1);
    /// Firmware must not expose it as a block device (GPT).
    pub const NO_BLOCK_IO: Self = Self(1 << 2);

    const NAMES: [(Self, &'static str); 3] = [
        (Self::BOOTABLE, "BOOTABLE"),
        (Self::REQUIRED, "REQUIRED"),
        (Self::NO_BLOCK_IO, "NO_BLOCK_IO"),
    ];

    /// No flag set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Every flag set.
    pub const fn all() -> Self {
        Self(Self::BOOTABLE.0 | Self::REQUIRED.0 | Self::NO_BLOCK_IO.0)
    }

    /// Returns the flags of `bits`, dropping bits that are not a flag.
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::all().0)
    }

    /// Returns the raw bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns whether every flag of `other` is set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns the flags of `self` and `other`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns `self` without the flags of `other`.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for PartitionFlags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

impl core::ops::BitOrAssign for PartitionFlags {
    fn bitor_assign(&mut self, other: Self) {
        *self = self.union(other);
    }
}

impl core::ops::BitAnd for PartitionFlags {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl core::ops::BitAndAssign for PartitionFlags {
    fn bitand_assign(&mut self, other: Self) {
        self.0 &= other.0;
    }
}

impl core::fmt::Debug for PartitionFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PartitionFlags(")?;
        let mut first = true;
        for (flag, name) in Self::NAMES {
            if self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

impl PartitionFlags {
    pub(crate) const fn from_gpt(attributes: u64) -> Self {
        let mut bits = 0;
        if attributes & raw::GPT_ATTR_LEGACY_BIOS_BOOTABLE != 0 {
            bits |= Self::BOOTABLE.bits();
        }
        if attributes & raw::GPT_ATTR_REQUIRED != 0 {
            bits |= Self::REQUIRED.bits();
        }
        if attributes & raw::GPT_ATTR_NO_BLOCK_IO != 0 {
            bits |= Self::NO_BLOCK_IO.bits();
        }
        Self(bits)
    }

    pub(crate) const fn to_gpt(self, attributes: u64) -> u64 {
        let mut out = attributes
            & !(raw::GPT_ATTR_LEGACY_BIOS_BOOTABLE
                | raw::GPT_ATTR_REQUIRED
                | raw::GPT_ATTR_NO_BLOCK_IO);
        if self.contains(Self::BOOTABLE) {
            out |= raw::GPT_ATTR_LEGACY_BIOS_BOOTABLE;
        }
        if self.contains(Self::REQUIRED) {
            out |= raw::GPT_ATTR_REQUIRED;
        }
        if self.contains(Self::NO_BLOCK_IO) {
            out |= raw::GPT_ATTR_NO_BLOCK_IO;
        }
        out
    }

    pub(crate) const fn from_mbr(boot_indicator: u8) -> Self {
        if boot_indicator & raw::MBR_ACTIVE != 0 {
            Self::BOOTABLE
        } else {
            Self::empty()
        }
    }

    pub(crate) const fn fits_mbr(self) -> bool {
        Self::BOOTABLE.contains(self)
    }

    pub(crate) const fn to_mbr(self) -> u8 {
        if self.contains(Self::BOOTABLE) {
            raw::MBR_ACTIVE
        } else {
            0
        }
    }
}

/// One partition of a table, located in blocks of the disk.
///
/// A copy of the entry, independent of the table it came from. Pass it to
/// `open` in a mode module (`hadris_part::sync::open`) for a block device
/// of the partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partition {
    slot: usize,
    start: u64,
    len: u64,
    block_size: BlockSize,
    kind: PartitionKind,
    flags: PartitionFlags,
    attributes: u64,
    unique_guid: Option<Guid>,
    name: Option<PartitionName>,
}

impl Partition {
    pub(crate) fn from_mbr(
        index: usize,
        start: u64,
        entry: &RawMbrEntry,
        block_size: BlockSize,
    ) -> Self {
        Self {
            slot: index,
            start,
            len: u64::from(entry.sector_count()),
            block_size,
            kind: PartitionKind::Mbr(MbrType::new(entry.kind)),
            flags: PartitionFlags::from_mbr(entry.boot_indicator),
            attributes: 0,
            unique_guid: None,
            name: None,
        }
    }

    pub(crate) fn from_gpt(index: usize, entry: &RawGptEntry, block_size: BlockSize) -> Self {
        Self {
            slot: index,
            start: entry.first_lba(),
            len: entry.block_len(),
            block_size,
            kind: PartitionKind::Gpt(entry.type_guid()),
            flags: PartitionFlags::from_gpt(entry.attributes()),
            attributes: entry.attributes(),
            unique_guid: Some(entry.unique_guid()),
            name: Some(PartitionName::from_le_bytes(entry.name)),
        }
    }

    /// The zero-based slot of the entry in its table: the GPT entry index, the
    /// MBR slot (0 to 3), or 4 and up for MBR logical partitions in chain
    /// order. Linux partition numbers are one higher. The `Gpt` and `Mbr` edit
    /// methods and [`Disk::partition`](crate::Disk::partition) take this slot.
    pub const fn slot(&self) -> usize {
        self.slot
    }

    /// The first block, counted from the start of the disk.
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// The length in blocks.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the partition has no blocks.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The block after the last one, saturating at `u64::MAX`.
    pub const fn end(&self) -> u64 {
        self.start.saturating_add(self.len)
    }

    /// The disk's block size.
    pub const fn block_size(&self) -> BlockSize {
        self.block_size
    }

    /// The length in bytes, saturating at `u64::MAX`.
    pub const fn size_bytes(&self) -> u64 {
        self.len.saturating_mul(self.block_size.get() as u64)
    }

    /// The partition type.
    pub const fn kind(&self) -> PartitionKind {
        self.kind
    }

    /// The flags the table stores for it.
    pub const fn flags(&self) -> PartitionFlags {
        self.flags
    }

    /// All 64 GPT attribute bits, including type-specific bits 48 to 63; 0
    /// for MBR partitions.
    pub const fn attributes(&self) -> u64 {
        self.attributes
    }

    /// The unique partition GUID, for GPT partitions.
    pub const fn unique_guid(&self) -> Option<Guid> {
        self.unique_guid
    }

    /// The partition name, for GPT partitions.
    pub const fn name(&self) -> Option<&PartitionName> {
        self.name.as_ref()
    }
}
