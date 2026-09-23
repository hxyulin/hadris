//! The MBR partition table, with extended and logical partitions.

use alloc::vec::Vec;

use hadris_fs::ErrorKind;
use hadris_storage::BlockSize;

use crate::codec::check_block_size;
use crate::codec::{FIRST_LOGICAL, Logical};
use crate::disk::Partitions;
use crate::error::{Detail, TableError};
use crate::raw::{Chs, RawMbr, RawMbrEntry};
use crate::{MbrType, Partition, PartitionFlags};

/// A new MBR partition: its type, first block, length and flags.
///
/// ```rust
/// use hadris_part::{MbrEntry, MbrType, PartitionFlags};
///
/// let entry = MbrEntry::new(MbrType::FAT32_LBA, 2048, 204_800)
///     .with_flags(PartitionFlags::BOOTABLE);
/// assert_eq!(entry.start(), 2048);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrEntry {
    kind: MbrType,
    start: u64,
    len: u64,
    flags: PartitionFlags,
}

impl MbrEntry {
    /// A partition of `len` blocks from block `start`.
    pub const fn new(kind: MbrType, start: u64, len: u64) -> Self {
        Self {
            kind,
            start,
            len,
            flags: PartitionFlags::empty(),
        }
    }

    /// Sets the flags; an MBR stores only [`PartitionFlags::BOOTABLE`].
    pub const fn with_flags(mut self, flags: PartitionFlags) -> Self {
        self.flags = flags;
        self
    }

    /// The partition type.
    pub const fn kind(&self) -> MbrType {
        self.kind
    }

    /// The first block.
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// The length in blocks.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the length is zero.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The flags.
    pub const fn flags(&self) -> PartitionFlags {
        self.flags
    }
}

/// An MBR partition table: four primary slots, one of which may be an
/// extended partition holding a chain of logical partitions.
///
/// Indices 0 to 3 are the primary slots; logical partitions follow from 4
/// in order of their first block, as Linux numbers them. Every edit checks
/// that the partition stays on the disk, fits the 32-bit MBR fields and
/// overlaps no other partition, and leaves the table unchanged when it
/// fails. Block 0 is not reserved, so an entry may cover the MBR itself as
/// hybrid ISO images do.
///
/// ```rust
/// use hadris_part::{Mbr, MbrEntry, MbrType};
/// use hadris_storage::BlockSize;
///
/// let mut mbr = Mbr::new(1 << 20, BlockSize::new(512).unwrap()).unwrap();
/// mbr.add(MbrEntry::new(MbrType::FAT32_LBA, 2048, 4096)).unwrap();
/// mbr.add(MbrEntry::new(MbrType::EXTENDED_LBA, 8192, 100_000)).unwrap();
/// let logical = mbr.add_logical(MbrEntry::new(MbrType::LINUX, 10_240, 2048)).unwrap();
/// assert_eq!(logical, 4);
/// assert!(mbr.add(MbrEntry::new(MbrType::LINUX, 4096, 8192)).is_err());
/// assert_eq!(mbr.partitions().count(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mbr {
    block_count: u64,
    block_size: BlockSize,
    primary: [RawMbrEntry; 4],
    logical: Vec<Logical>,
}

impl Mbr {
    /// An empty table for a disk of `block_count` blocks.
    ///
    /// Fails with [`ErrorKind::Unsupported`] unless the block size is a
    /// power of two of at least 512 bytes.
    pub fn new(block_count: u64, block_size: BlockSize) -> Result<Self, TableError> {
        check_block_size(block_size)?;
        Ok(Self {
            block_count,
            block_size,
            primary: [RawMbrEntry::default(); 4],
            logical: Vec::new(),
        })
    }

    pub(crate) fn from_disk(
        record: &RawMbr,
        logical: Vec<Logical>,
        block_count: u64,
        block_size: BlockSize,
    ) -> Self {
        Self {
            block_count,
            block_size,
            primary: record.entries,
            logical,
        }
    }

    /// The number of blocks of the disk.
    pub const fn block_count(&self) -> u64 {
        self.block_count
    }

    /// The disk's block size.
    pub const fn block_size(&self) -> BlockSize {
        self.block_size
    }

    /// The primary and logical partitions, skipping empty slots and the
    /// extended partition itself.
    pub fn partitions(&self) -> Partitions<'_> {
        Partitions::mbr(self)
    }

    /// The partition with table index `index`, including the extended
    /// partition.
    pub fn entry(&self, index: usize) -> Option<Partition> {
        if index < FIRST_LOGICAL {
            let entry = &self.primary[index];
            return (!entry.is_empty()).then(|| {
                Partition::from_mbr(index, u64::from(entry.start_lba()), entry, self.block_size)
            });
        }
        let logical = self.logical.get(index - FIRST_LOGICAL)?;
        Some(Partition::from_mbr(
            index,
            logical.start,
            &logical.entry(),
            self.block_size,
        ))
    }

    /// The extended partition, if there is one.
    pub fn extended(&self) -> Option<Partition> {
        self.extended_slot().and_then(|slot| self.entry(slot))
    }

    /// The raw entry of primary slot `slot` (0 to 3) as it will be written.
    pub fn raw_entry(&self, slot: usize) -> Option<&RawMbrEntry> {
        self.primary.get(slot)
    }

    pub(crate) fn primary_entries(&self) -> &[RawMbrEntry; 4] {
        &self.primary
    }

    pub(crate) fn logical_count(&self) -> usize {
        self.logical.len()
    }

    fn extended_slot(&self) -> Option<usize> {
        self.primary
            .iter()
            .position(|e| MbrType::new(e.kind).is_extended())
    }

    fn extended_range(&self) -> Option<(u64, u64)> {
        let slot = self.extended_slot()?;
        let e = &self.primary[slot];
        Some((
            u64::from(e.start_lba()),
            u64::from(e.start_lba()) + u64::from(e.sector_count()),
        ))
    }

    /// Adds a primary partition in the first free slot and returns its index.
    ///
    /// An extended type ([`MbrType::is_extended`]) makes it the extended
    /// partition; a table has at most one. Fails with
    /// [`ErrorKind::LimitExceeded`] when all four slots are used.
    pub fn add(&mut self, entry: MbrEntry) -> Result<usize, TableError> {
        let slot = self
            .primary
            .iter()
            .position(RawMbrEntry::is_empty)
            .ok_or(TableError::new(ErrorKind::LimitExceeded, Detail::TableFull))?;
        self.check_primary(slot, &entry)?;
        self.primary[slot] = primary_entry(&entry);
        Ok(slot)
    }

    /// Adds a logical partition inside the extended partition and returns
    /// its index.
    ///
    /// Its extended boot record goes in the block before it, or in the first
    /// block of the extended partition when it becomes the first logical
    /// partition, so that block must be free. Indices of logical partitions
    /// after it move up by one.
    pub fn add_logical(&mut self, entry: MbrEntry) -> Result<usize, TableError> {
        let kind = entry.kind;
        if kind.is_empty() || kind.is_extended() || kind.is_protective() {
            return Err(TableError::invalid(Detail::Kind));
        }
        check_flags(entry.flags)?;
        let (ext_start, ext_end) = self
            .extended_range()
            .ok_or(TableError::invalid(Detail::Extended))?;
        let mut logical = self.logical.clone();
        logical.push(Logical {
            ebr: 0,
            start: entry.start,
            len: entry.len,
            kind: kind.code(),
            boot: entry.flags.to_mbr(),
        });
        place_logicals(ext_start, ext_end, &mut logical)?;
        let index = logical
            .iter()
            .position(|l| l.start == entry.start)
            .map_or(FIRST_LOGICAL, |k| FIRST_LOGICAL + k);
        self.logical = logical;
        Ok(index)
    }

    /// Removes the partition with index `index`.
    ///
    /// The extended partition can go only once it holds no logical
    /// partitions. Removing a logical partition renumbers the ones after it.
    pub fn remove(&mut self, index: usize) -> Result<(), TableError> {
        if index < FIRST_LOGICAL {
            let entry = &self.primary[index];
            if entry.is_empty() {
                return Err(no_such(index));
            }
            if MbrType::new(entry.kind).is_extended() && !self.logical.is_empty() {
                return Err(TableError::invalid(Detail::ExtendedInUse));
            }
            self.primary[index] = RawMbrEntry::default();
            return Ok(());
        }
        let k = index - FIRST_LOGICAL;
        if k >= self.logical.len() {
            return Err(no_such(index));
        }
        let (ext_start, ext_end) = self
            .extended_range()
            .ok_or(TableError::invalid(Detail::Extended))?;
        let mut logical = self.logical.clone();
        logical.remove(k);
        place_logicals(ext_start, ext_end, &mut logical)?;
        self.logical = logical;
        Ok(())
    }

    /// Changes the length of partition `index` to `len` blocks, keeping its
    /// first block.
    pub fn resize(&mut self, index: usize, len: u64) -> Result<(), TableError> {
        self.edit(index, |entry| entry.len = len)
    }

    /// Changes the type of partition `index`.
    ///
    /// The extended partition keeps an extended type, and no other partition
    /// can take one.
    pub fn set_kind(&mut self, index: usize, kind: MbrType) -> Result<(), TableError> {
        let current = self.entry(index).ok_or(no_such(index))?;
        let was_extended =
            matches!(current.kind(), crate::PartitionKind::Mbr(k) if k.is_extended());
        if was_extended != kind.is_extended() {
            return Err(TableError::invalid(Detail::Extended));
        }
        self.edit(index, |entry| entry.kind = kind)
    }

    /// Replaces the flags of partition `index`.
    pub fn set_flags(&mut self, index: usize, flags: PartitionFlags) -> Result<(), TableError> {
        self.edit(index, |entry| entry.flags = flags)
    }

    fn edit(&mut self, index: usize, f: impl FnOnce(&mut MbrEntry)) -> Result<(), TableError> {
        let current = self.entry(index).ok_or(no_such(index))?;
        let crate::PartitionKind::Mbr(kind) = current.kind() else {
            return Err(no_such(index));
        };
        let mut entry =
            MbrEntry::new(kind, current.start(), current.len()).with_flags(current.flags());
        f(&mut entry);
        if index < FIRST_LOGICAL {
            self.check_primary(index, &entry)?;
            if kind.is_extended() {
                let end = entry.start.saturating_add(entry.len);
                let mut logical = self.logical.clone();
                place_logicals(entry.start, end, &mut logical)?;
                self.logical = logical;
            }
            self.primary[index] = primary_entry(&entry);
            return Ok(());
        }
        check_flags(entry.flags)?;
        if entry.kind.is_empty() || entry.kind.is_protective() {
            return Err(TableError::invalid(Detail::Kind));
        }
        let (ext_start, ext_end) = self
            .extended_range()
            .ok_or(TableError::invalid(Detail::Extended))?;
        let mut logical = self.logical.clone();
        let target = &mut logical[index - FIRST_LOGICAL];
        target.len = entry.len;
        target.kind = entry.kind.code();
        target.boot = entry.flags.to_mbr();
        place_logicals(ext_start, ext_end, &mut logical)?;
        self.logical = logical;
        Ok(())
    }

    fn check_primary(&self, slot: usize, entry: &MbrEntry) -> Result<(), TableError> {
        if entry.kind.is_empty() || entry.kind.is_protective() {
            return Err(TableError::invalid(Detail::Kind));
        }
        check_flags(entry.flags)?;
        if entry.len == 0 {
            return Err(TableError::invalid(Detail::Size));
        }
        if u32::try_from(entry.start).is_err() || u32::try_from(entry.len).is_err() {
            return Err(TableError::new(
                ErrorKind::LimitExceeded,
                Detail::FieldOverflow,
            ));
        }
        let end = entry.start + entry.len;
        if entry.kind.is_extended() && entry.start == 0 {
            return Err(TableError::invalid(Detail::Extended));
        }
        if end > self.block_count {
            return Err(TableError::invalid(Detail::OutOfBounds { index: slot }));
        }
        for (other, existing) in self.primary.iter().enumerate() {
            if other == slot || existing.is_empty() {
                continue;
            }
            if entry.kind.is_extended() && MbrType::new(existing.kind).is_extended() {
                return Err(TableError::invalid(Detail::Extended));
            }
            let start = u64::from(existing.start_lba());
            let stop = start + u64::from(existing.sector_count());
            if entry.start < stop && start < end {
                return Err(TableError::invalid(Detail::Overlap { index: slot, other }));
            }
        }
        Ok(())
    }

    /// The extended boot records to write: each block and its record.
    pub(crate) fn ebr_records(&self) -> Vec<(u64, RawMbr)> {
        let mut records = Vec::new();
        let Some((ext_start, _)) = self.extended_range() else {
            return records;
        };
        if self.logical.is_empty() {
            records.push((ext_start, RawMbr::default()));
            return records;
        }
        for (k, logical) in self.logical.iter().enumerate() {
            let ebr = if k == 0 { ext_start } else { logical.ebr };
            let mut record = RawMbr::default();
            record.entries[0] = located(
                logical.kind,
                logical.boot,
                logical.start,
                logical.start - ebr,
                logical.len,
            );
            if let Some(next) = self.logical.get(k + 1) {
                record.entries[1] = located(
                    MbrType::EXTENDED.code(),
                    0,
                    next.ebr,
                    next.ebr - ext_start,
                    next.end() - next.ebr,
                );
            }
            records.push((ebr, record));
        }
        records
    }
}

/// An entry whose fields count from `relative` while its CHS addresses use
/// the absolute block `absolute`.
fn located(kind: u8, boot: u8, absolute: u64, relative: u64, len: u64) -> RawMbrEntry {
    let chs = |lba: u64| u32::try_from(lba).map_or(Chs::OUT_OF_RANGE, Chs::from_lba);
    RawMbrEntry {
        boot_indicator: boot,
        start_chs: chs(absolute),
        kind,
        end_chs: chs(absolute + len.saturating_sub(1)),
        start_lba: (relative as u32).to_le_bytes(),
        sector_count: (len as u32).to_le_bytes(),
    }
}

fn primary_entry(entry: &MbrEntry) -> RawMbrEntry {
    let mut raw = RawMbrEntry::new(entry.kind.code(), entry.start as u32, entry.len as u32);
    raw.boot_indicator = entry.flags.to_mbr();
    raw
}

fn check_flags(flags: PartitionFlags) -> Result<(), TableError> {
    if flags.fits_mbr() {
        Ok(())
    } else {
        Err(TableError::invalid(Detail::Flags))
    }
}

fn no_such(index: usize) -> TableError {
    TableError::new(ErrorKind::NotFound, Detail::NoSuchPartition { index })
}

/// Sorts logical partitions, places their extended boot records and checks
/// that everything fits in the extended partition `ext_start..ext_end`.
pub(crate) fn place_logicals(
    ext_start: u64,
    ext_end: u64,
    logical: &mut [Logical],
) -> Result<(), TableError> {
    logical.sort_by_key(|l| l.start);
    let mut prev_end = ext_start;
    for (k, l) in logical.iter_mut().enumerate() {
        let index = FIRST_LOGICAL + k;
        if l.len == 0 {
            return Err(TableError::invalid(Detail::Size));
        }
        if l.start <= ext_start || l.end() > ext_end || l.start.checked_add(l.len).is_none() {
            return Err(TableError::invalid(Detail::OutOfBounds { index }));
        }
        if k == 0 {
            l.ebr = ext_start;
        } else if !(prev_end <= l.ebr && l.ebr < l.start) || l.ebr == ext_start {
            l.ebr = l.start - 1;
        }
        if l.ebr < prev_end {
            let other = if k == 0 { index } else { index - 1 };
            return Err(TableError::invalid(Detail::Overlap { index, other }));
        }
        prev_end = l.end();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Mbr {
        Mbr::new(100_000, BlockSize::new(512).unwrap()).unwrap()
    }

    #[test]
    fn primaries_reject_overlap_bounds_and_overflow() {
        let mut mbr = table();
        assert_eq!(mbr.add(MbrEntry::new(MbrType::LINUX, 100, 100)), Ok(0));
        let err = mbr
            .add(MbrEntry::new(MbrType::LINUX, 150, 100))
            .unwrap_err();
        assert_eq!(err.detail(), Detail::Overlap { index: 1, other: 0 });
        let err = mbr
            .add(MbrEntry::new(MbrType::LINUX, 99_950, 100))
            .unwrap_err();
        assert_eq!(err.detail(), Detail::OutOfBounds { index: 1 });
        let err = mbr
            .add(MbrEntry::new(MbrType::LINUX, 1 << 32, 1))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
        assert_eq!(
            mbr.add(MbrEntry::new(MbrType::GPT_PROTECTIVE, 300, 1))
                .unwrap_err()
                .detail(),
            Detail::Kind
        );
        assert_eq!(
            mbr.add(MbrEntry::new(MbrType::LINUX, 300, 1).with_flags(PartitionFlags::REQUIRED))
                .unwrap_err()
                .detail(),
            Detail::Flags
        );
        assert_eq!(mbr.partitions().count(), 1);
    }

    #[test]
    fn logical_partitions_chain_inside_the_extended_partition() {
        let mut mbr = table();
        assert_eq!(
            mbr.add_logical(MbrEntry::new(MbrType::LINUX, 1000, 10))
                .unwrap_err()
                .detail(),
            Detail::Extended
        );
        mbr.add(MbrEntry::new(MbrType::EXTENDED_LBA, 1000, 1000))
            .unwrap();
        assert_eq!(
            mbr.add(MbrEntry::new(MbrType::EXTENDED, 5000, 10))
                .unwrap_err()
                .detail(),
            Detail::Extended
        );
        assert_eq!(
            mbr.add_logical(MbrEntry::new(MbrType::LINUX, 1500, 100)),
            Ok(4)
        );
        assert_eq!(
            mbr.add_logical(MbrEntry::new(MbrType::LINUX_SWAP, 1100, 100)),
            Ok(4)
        );
        assert_eq!(mbr.entry(5).unwrap().start(), 1500);
        assert_eq!(
            mbr.add_logical(MbrEntry::new(MbrType::LINUX, 1000, 10))
                .unwrap_err()
                .detail(),
            Detail::OutOfBounds { index: 4 }
        );
        assert_eq!(
            mbr.add_logical(MbrEntry::new(MbrType::LINUX, 1950, 100))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            mbr.add_logical(MbrEntry::new(MbrType::LINUX, 1200, 300))
                .unwrap_err()
                .detail(),
            Detail::Overlap { index: 5, other: 4 }
        );
        let records = mbr.ebr_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].0, 1000);
        assert_eq!(records[0].1.entries[0].start_lba(), 100);
        assert_eq!(records[0].1.entries[1].start_lba(), 499);
        assert_eq!(records[0].1.entries[1].sector_count(), 101);
        assert_eq!(records[1].0, 1499);
        assert_eq!(records[1].1.entries[0].start_lba(), 1);
        assert!(records[1].1.entries[1].is_empty());

        assert_eq!(mbr.remove(0).unwrap_err().detail(), Detail::ExtendedInUse);
        assert_eq!(
            mbr.resize(0, 400).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        mbr.remove(4).unwrap();
        assert_eq!(mbr.entry(4).unwrap().start(), 1500);
        assert_eq!(mbr.ebr_records()[0].0, 1000);
        assert_eq!(mbr.ebr_records()[0].1.entries[0].start_lba(), 500);
        mbr.remove(4).unwrap();
        mbr.remove(0).unwrap();
        assert!(mbr.extended().is_none());
    }
}
