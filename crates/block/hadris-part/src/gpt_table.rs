use alloc::borrow::Cow;
use alloc::vec::Vec;

use hadris_fs::ErrorKind;
use hadris_storage::BlockSize;

use crate::codec::{blocks_for, check_block_size};
use crate::disk::Partitions;
use crate::error::{Detail, TableError};
use crate::raw::{self, RawGptEntry, RawGptHeader};
use crate::{GptCopy, Guid, Partition, PartitionFlags, PartitionName};

/// A new GPT partition: type, unique GUID, first block, length, name and
/// flags.
///
/// ```rust
/// use hadris_part::gpt::types;
/// use hadris_part::{GptEntry, Guid, PartitionFlags, PartitionName};
///
/// let entry = GptEntry::new(types::EFI_SYSTEM, Guid::from_bytes([1; 16]), 2048, 204_800)
///     .with_name(PartitionName::new("EFI").unwrap())
///     .with_flags(PartitionFlags::REQUIRED);
/// assert_eq!(entry.len(), 204_800);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GptEntry {
    type_guid: Guid,
    unique_guid: Guid,
    start: u64,
    len: u64,
    name: PartitionName,
    attributes: u64,
}

impl GptEntry {
    /// A partition of `len` blocks from block `start`.
    pub const fn new(type_guid: Guid, unique_guid: Guid, start: u64, len: u64) -> Self {
        Self {
            type_guid,
            unique_guid,
            start,
            len,
            name: PartitionName::EMPTY,
            attributes: 0,
        }
    }

    /// Sets the name.
    pub const fn with_name(mut self, name: PartitionName) -> Self {
        self.name = name;
        self
    }

    /// Sets attribute bits 0 to 2 from `flags`, keeping the others.
    pub const fn with_flags(mut self, flags: PartitionFlags) -> Self {
        self.attributes = flags.to_gpt(self.attributes);
        self
    }

    /// Sets all 64 attribute bits, including type-specific bits 48 to 63.
    pub const fn with_attributes(mut self, attributes: u64) -> Self {
        self.attributes = attributes;
        self
    }

    /// The partition type GUID.
    pub const fn type_guid(&self) -> Guid {
        self.type_guid
    }

    /// The unique partition GUID.
    pub const fn unique_guid(&self) -> Guid {
        self.unique_guid
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

    fn raw(&self) -> RawGptEntry {
        RawGptEntry {
            type_guid: self.type_guid.to_bytes(),
            unique_guid: self.unique_guid.to_bytes(),
            first_lba: self.start.to_le_bytes(),
            last_lba: (self.start + self.len - 1).to_le_bytes(),
            attributes: self.attributes.to_le_bytes(),
            name: self.name.to_le_bytes(),
        }
    }
}

/// A GUID partition table.
///
/// The fields are private: every edit keeps the table consistent, and the
/// CRCs are computed whenever the table is written. Edits check that a
/// partition lies in the usable area and overlaps no other, and leave the
/// table unchanged when they fail.
///
/// ```rust
/// use hadris_part::gpt::types;
/// use hadris_part::{Gpt, GptEntry, Guid};
/// use hadris_storage::BlockSize;
///
/// let mut gpt = Gpt::new(Guid::from_bytes([9; 16]), 1 << 16, BlockSize::new(512).unwrap()).unwrap();
/// assert_eq!((gpt.first_usable(), gpt.last_usable()), (34, 65_502));
/// let esp = gpt.add(GptEntry::new(types::EFI_SYSTEM, Guid::from_bytes([1; 16]), 2048, 8192)).unwrap();
/// gpt.resize(esp, 16_384).unwrap();
/// assert!(gpt.add(GptEntry::new(types::LINUX_FILESYSTEM, Guid::from_bytes([2; 16]), 10_000, 100)).is_err());
/// gpt.remove(esp).unwrap();
/// assert_eq!(gpt.partitions().count(), 0);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpt {
    block_size: BlockSize,
    block_count: u64,
    disk_guid: Guid,
    first_usable: u64,
    last_usable: u64,
    backup_lba: u64,
    primary_entries_lba: u64,
    backup_entries_lba: u64,
    entry_size: u32,
    entries: Vec<RawGptEntry>,
    damaged: Option<GptCopy>,
}

impl Gpt {
    /// An empty table with 128 entry slots for a disk of `block_count`
    /// blocks, the primary entries from block 2 and the backup header in the
    /// last block.
    ///
    /// Fails with [`ErrorKind::Unsupported`] unless the block size is a
    /// power of two of at least 512 bytes, and with [`ErrorKind::NoSpace`]
    /// when the disk leaves no usable block.
    pub fn new(
        disk_guid: Guid,
        block_count: u64,
        block_size: BlockSize,
    ) -> Result<Self, TableError> {
        check_block_size(block_size)?;
        let array = blocks_for(
            u64::from(raw::GPT_DEFAULT_ENTRIES) * u64::from(raw::GPT_ENTRY_SIZE),
            block_size,
        );
        let first_usable = 2 + array;
        let too_small = TableError::new(ErrorKind::NoSpace, Detail::DiskTooSmall);
        let last_usable = block_count
            .checked_sub(2 + array)
            .filter(|&last| last >= first_usable)
            .ok_or(too_small)?;
        Ok(Self {
            block_size,
            block_count,
            disk_guid,
            first_usable,
            last_usable,
            backup_lba: block_count - 1,
            primary_entries_lba: 2,
            backup_entries_lba: block_count - 1 - array,
            entry_size: raw::GPT_ENTRY_SIZE,
            entries: alloc::vec![RawGptEntry::default(); raw::GPT_DEFAULT_ENTRIES as usize],
            damaged: None,
        })
    }

    /// Builds a table from the validated header `header` of copy `source`.
    /// `backup_entries` is where a valid backup copy keeps its array.
    pub(crate) fn from_disk(
        header: &RawGptHeader,
        source: GptCopy,
        entries: Vec<RawGptEntry>,
        damaged: Option<GptCopy>,
        backup_entries: Option<u64>,
        block_count: u64,
        block_size: BlockSize,
    ) -> Self {
        let array = blocks_for(
            u64::from(header.num_partition_entries()) * u64::from(header.size_of_partition_entry()),
            block_size,
        );
        let last_usable = header.last_usable_lba();
        let fits = |backup: u64| {
            backup < block_count && backup.checked_sub(array).is_some_and(|a| a > last_usable)
        };
        let (primary_entries_lba, backup_lba) = match source {
            GptCopy::Primary => {
                let alternate = header.alternate_lba();
                let backup = if fits(alternate) {
                    alternate
                } else {
                    block_count.saturating_sub(1)
                };
                (header.partition_entry_lba(), backup)
            }
            GptCopy::Backup => (2, header.my_lba()),
        };
        let backup_entries_lba = match (source, backup_entries) {
            (GptCopy::Backup, _) => header.partition_entry_lba(),
            (_, Some(lba)) if damaged.is_none() => lba,
            _ => backup_lba.saturating_sub(array),
        };
        Self {
            block_size,
            block_count,
            disk_guid: header.disk_guid(),
            first_usable: header.first_usable_lba(),
            last_usable,
            backup_lba,
            primary_entries_lba,
            backup_entries_lba,
            entry_size: header.size_of_partition_entry(),
            entries,
            damaged,
        }
    }

    /// The disk's block size.
    pub const fn block_size(&self) -> BlockSize {
        self.block_size
    }

    /// The number of blocks of the disk.
    pub const fn block_count(&self) -> u64 {
        self.block_count
    }

    /// The disk GUID.
    pub const fn disk_guid(&self) -> Guid {
        self.disk_guid
    }

    /// Replaces the disk GUID.
    pub fn set_disk_guid(&mut self, disk_guid: Guid) {
        self.disk_guid = disk_guid;
    }

    /// The first block partitions may use.
    pub const fn first_usable(&self) -> u64 {
        self.first_usable
    }

    /// The last block partitions may use, inclusive.
    pub const fn last_usable(&self) -> u64 {
        self.last_usable
    }

    /// The block of the backup header.
    pub const fn backup_lba(&self) -> u64 {
        self.backup_lba
    }

    /// The number of entry slots.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// The copy that failed validation when the table was read, if one did.
    /// The table came from the other copy, and writing it repairs both.
    pub const fn damaged_copy(&self) -> Option<GptCopy> {
        self.damaged
    }

    /// The used partitions, in slot order.
    pub fn partitions(&self) -> Partitions<'_> {
        Partitions::gpt(self)
    }

    /// The partition in slot `index`, if the slot is used.
    pub fn entry(&self, index: usize) -> Option<Partition> {
        let entry = self.entries.get(index)?;
        (!entry.is_unused()).then(|| Partition::from_gpt(index, entry, self.block_size))
    }

    /// The raw entry in slot `index`, as it will be written.
    pub fn raw_entry(&self, index: usize) -> Option<&RawGptEntry> {
        self.entries.get(index)
    }

    /// Adds a partition in the first unused slot and returns its index.
    ///
    /// Fails with [`ErrorKind::LimitExceeded`] when every slot is used and
    /// [`ErrorKind::InvalidInput`] when the entry has the unused (nil) type,
    /// no blocks, lies outside the usable area or overlaps a partition.
    pub fn add(&mut self, entry: GptEntry) -> Result<usize, TableError> {
        let slot = self
            .entries
            .iter()
            .position(RawGptEntry::is_unused)
            .ok_or(TableError::new(ErrorKind::LimitExceeded, Detail::TableFull))?;
        self.check(slot, &entry)?;
        self.entries[slot] = entry.raw();
        Ok(slot)
    }

    /// Clears slot `index`.
    pub fn remove(&mut self, index: usize) -> Result<(), TableError> {
        self.used(index)?;
        self.entries[index] = RawGptEntry::default();
        Ok(())
    }

    /// Changes the length of partition `index` to `len` blocks, keeping its
    /// first block.
    pub fn resize(&mut self, index: usize, len: u64) -> Result<(), TableError> {
        self.edit(index, |entry| entry.len = len)
    }

    /// Changes the type of partition `index`.
    pub fn set_type(&mut self, index: usize, type_guid: Guid) -> Result<(), TableError> {
        self.edit(index, |entry| entry.type_guid = type_guid)
    }

    /// Changes the unique GUID of partition `index`.
    pub fn set_unique_guid(&mut self, index: usize, unique_guid: Guid) -> Result<(), TableError> {
        self.edit(index, |entry| entry.unique_guid = unique_guid)
    }

    /// Renames partition `index`.
    pub fn set_name(&mut self, index: usize, name: PartitionName) -> Result<(), TableError> {
        self.edit(index, |entry| entry.name = name)
    }

    /// Replaces attribute bits 0 to 2 of partition `index` with `flags`.
    pub fn set_flags(&mut self, index: usize, flags: PartitionFlags) -> Result<(), TableError> {
        self.edit(index, |entry| {
            entry.attributes = flags.to_gpt(entry.attributes)
        })
    }

    /// Replaces all 64 attribute bits of partition `index`.
    pub fn set_attributes(&mut self, index: usize, attributes: u64) -> Result<(), TableError> {
        self.edit(index, |entry| entry.attributes = attributes)
    }

    fn used(&self, index: usize) -> Result<&RawGptEntry, TableError> {
        self.entries
            .get(index)
            .filter(|e| !e.is_unused())
            .ok_or(TableError::new(
                ErrorKind::NotFound,
                Detail::NoSuchPartition { index },
            ))
    }

    fn edit(&mut self, index: usize, f: impl FnOnce(&mut GptEntry)) -> Result<(), TableError> {
        let raw = self.used(index)?;
        let mut entry = GptEntry {
            type_guid: raw.type_guid(),
            unique_guid: raw.unique_guid(),
            start: raw.first_lba(),
            len: raw.block_len(),
            name: PartitionName::from_le_bytes(raw.name),
            attributes: raw.attributes(),
        };
        f(&mut entry);
        self.check(index, &entry)?;
        self.entries[index] = entry.raw();
        Ok(())
    }

    fn check(&self, index: usize, entry: &GptEntry) -> Result<(), TableError> {
        if entry.type_guid.is_nil() {
            return Err(TableError::invalid(Detail::Kind));
        }
        if entry.len == 0 {
            return Err(TableError::invalid(Detail::Size));
        }
        let end = entry
            .start
            .checked_add(entry.len)
            .ok_or(TableError::invalid(Detail::OutOfBounds { index }))?;
        if entry.start < self.first_usable || end - 1 > self.last_usable {
            return Err(TableError::invalid(Detail::OutOfBounds { index }));
        }
        for (other, existing) in self.entries.iter().enumerate() {
            if other == index || existing.is_unused() {
                continue;
            }
            let first = existing.first_lba();
            let last = existing.last_lba();
            if entry.start <= last && first < end {
                return Err(TableError::invalid(Detail::Overlap { index, other }));
            }
        }
        Ok(())
    }

    /// Whether both copies fit around the usable area of a disk of
    /// `block_count` blocks.
    pub(crate) fn fits(&self, block_count: u64) -> bool {
        let array = blocks_for(
            self.entries.len() as u64 * u64::from(self.entry_size),
            self.block_size,
        );
        let primary_end = self.primary_entries_lba.checked_add(array);
        let backup_end = self.backup_entries_lba.checked_add(array);
        self.primary_entries_lba >= 2
            && primary_end.is_some_and(|end| end <= self.first_usable)
            && self.backup_entries_lba > self.last_usable
            && backup_end.is_some_and(|end| end <= self.backup_lba)
            && self.backup_lba < block_count
    }

    pub(crate) const fn primary_entries_lba(&self) -> u64 {
        self.primary_entries_lba
    }

    pub(crate) const fn backup_entries_lba(&self) -> u64 {
        self.backup_entries_lba
    }

    /// The entry array as written, `entry_size` bytes per entry.
    pub(crate) fn array_bytes(&self) -> Cow<'_, [u8]> {
        let entries: &[u8] = bytemuck::cast_slice(&self.entries);
        if self.entry_size == raw::GPT_ENTRY_SIZE {
            return Cow::Borrowed(entries);
        }
        let size = self.entry_size as usize;
        let mut bytes = alloc::vec![0u8; self.entries.len() * size];
        for (chunk, entry) in bytes.chunks_exact_mut(size).zip(&self.entries) {
            chunk[..128].copy_from_slice(bytemuck::bytes_of(entry));
        }
        Cow::Owned(bytes)
    }

    /// The header of copy `copy`, with both CRCs.
    pub(crate) fn header(&self, copy: GptCopy, array_crc: u32) -> RawGptHeader {
        let (my_lba, alternate_lba, entries_lba) = match copy {
            GptCopy::Backup => (self.backup_lba, 1, self.backup_entries_lba),
            _ => (1, self.backup_lba, self.primary_entries_lba),
        };
        let mut header = RawGptHeader {
            signature: raw::GPT_SIGNATURE,
            revision: raw::GPT_REVISION.to_le_bytes(),
            header_size: raw::GPT_HEADER_SIZE.to_le_bytes(),
            header_crc32: [0; 4],
            reserved: [0; 4],
            my_lba: my_lba.to_le_bytes(),
            alternate_lba: alternate_lba.to_le_bytes(),
            first_usable_lba: self.first_usable.to_le_bytes(),
            last_usable_lba: self.last_usable.to_le_bytes(),
            disk_guid: self.disk_guid.to_bytes(),
            partition_entry_lba: entries_lba.to_le_bytes(),
            num_partition_entries: (self.entries.len() as u32).to_le_bytes(),
            size_of_partition_entry: self.entry_size.to_le_bytes(),
            partition_entry_array_crc32: array_crc.to_le_bytes(),
        };
        header.header_crc32 = raw::crc32(bytemuck::bytes_of(&header)).to_le_bytes();
        header
    }
}
