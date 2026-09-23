//! Validation shared by every mode.

use hadris_fs::ErrorKind;
use hadris_storage::BlockSize;

use crate::TableKind;
use crate::error::{Detail, TableError};
use crate::raw::{self, RawGptHeader, RawMbr, RawMbrEntry};

/// The largest GPT entry array accepted, far above the 16 KiB tools write.
pub(crate) const MAX_ARRAY_BYTES: u64 = 64 << 20;

/// Index of the first logical partition.
pub(crate) const FIRST_LOGICAL: usize = 4;

/// The most logical partitions followed in an EBR chain.
pub(crate) const MAX_LOGICAL: usize = 256;

/// Block sizes must be powers of two of at least 512 bytes, so a GPT
/// entry's first 128 bytes never straddle a block.
pub(crate) fn check_block_size(size: BlockSize) -> Result<(), TableError> {
    let bytes = size.get();
    if bytes >= 512 && bytes.is_power_of_two() {
        Ok(())
    } else {
        Err(TableError::new(ErrorKind::Unsupported, Detail::BlockSize))
    }
}

pub(crate) const fn blocks_for(bytes: u64, size: BlockSize) -> u64 {
    bytes.div_ceil(size.get() as u64)
}

/// Which table block 0 describes.
pub(crate) fn classify(record: &RawMbr) -> Result<TableKind, TableError> {
    if !record.has_signature() {
        return Err(TableError::new(ErrorKind::NotFound, Detail::NoTable));
    }
    let mut protective = false;
    let mut other = false;
    let mut extended = 0;
    for entry in &record.entries {
        if entry.boot_indicator & !raw::MBR_ACTIVE != 0 {
            return Err(TableError::new(ErrorKind::Corrupt, Detail::MbrEntry));
        }
        let kind = crate::MbrType::new(entry.kind);
        if kind.is_empty() {
            continue;
        }
        if kind.is_protective() {
            protective = true;
        } else {
            other = true;
            if kind.is_extended() {
                if entry.start_lba() == 0 {
                    return Err(TableError::new(ErrorKind::Corrupt, Detail::EbrChain));
                }
                extended += 1;
            }
        }
    }
    if extended > 1 {
        return Err(TableError::new(ErrorKind::Corrupt, Detail::MbrEntry));
    }
    Ok(match (protective, other) {
        (true, true) => TableKind::Hybrid,
        (true, false) => TableKind::Gpt,
        _ => TableKind::Mbr,
    })
}

/// The entry array a validated header points to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Array {
    pub(crate) lba: u64,
    pub(crate) blocks: u64,
    pub(crate) bytes: u64,
    pub(crate) count: u32,
    pub(crate) entry_size: u32,
}

/// Validates the GPT header in `block`, read from block `lba` of a disk of
/// `block_count` blocks, up to but not including the entry array CRC.
pub(crate) fn check_header(
    block: &[u8],
    lba: u64,
    block_count: u64,
    size: BlockSize,
) -> Result<(RawGptHeader, Array), Detail> {
    let header: RawGptHeader = bytemuck::pod_read_unaligned(&block[..size_of::<RawGptHeader>()]);
    if header.signature != raw::GPT_SIGNATURE || header.revision() >> 16 != 1 {
        return Err(Detail::GptHeader);
    }
    let crc = raw::gpt_header_crc(block, header.header_size()).ok_or(Detail::GptHeader)?;
    if crc != header.header_crc32() {
        return Err(Detail::GptHeaderCrc);
    }
    let first = header.first_usable_lba();
    let last = header.last_usable_lba();
    if header.my_lba() != lba
        || last >= block_count
        || first > last.saturating_add(1)
        || (first..=last).contains(&lba)
    {
        return Err(Detail::GptHeader);
    }
    let entry_size = header.size_of_partition_entry();
    if entry_size < raw::GPT_ENTRY_SIZE || !entry_size.is_power_of_two() {
        return Err(Detail::GptEntries);
    }
    let count = header.num_partition_entries();
    let bytes = u64::from(count) * u64::from(entry_size);
    let blocks = blocks_for(bytes, size);
    let start = header.partition_entry_lba();
    let end = start.checked_add(blocks).ok_or(Detail::GptEntries)?;
    let placed = if lba == 1 {
        start >= 2 && end <= first
    } else {
        start > last && end <= lba
    };
    if bytes > MAX_ARRAY_BYTES || !placed || end > block_count {
        return Err(Detail::GptEntries);
    }
    let array = Array {
        lba: start,
        blocks,
        bytes,
        count,
        entry_size,
    };
    Ok((header, array))
}

/// Whether a backup header describes the same table as the primary.
pub(crate) fn same_table(primary: &RawGptHeader, backup: &RawGptHeader) -> bool {
    backup.alternate_lba() == 1
        && primary.alternate_lba() == backup.my_lba()
        && primary.first_usable_lba == backup.first_usable_lba
        && primary.last_usable_lba == backup.last_usable_lba
        && primary.disk_guid == backup.disk_guid
        && primary.num_partition_entries == backup.num_partition_entries
        && primary.size_of_partition_entry == backup.size_of_partition_entry
        && primary.partition_entry_array_crc32 == backup.partition_entry_array_crc32
}

/// A logical partition inside the extended partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Logical {
    /// The block of its extended boot record. The first logical's record is
    /// always the first block of the extended partition.
    pub(crate) ebr: u64,
    pub(crate) start: u64,
    pub(crate) len: u64,
    pub(crate) kind: u8,
    pub(crate) boot: u8,
}

impl Logical {
    pub(crate) const fn end(&self) -> u64 {
        self.start.saturating_add(self.len)
    }

    pub(crate) fn entry(&self) -> RawMbrEntry {
        RawMbrEntry {
            boot_indicator: self.boot,
            kind: self.kind,
            sector_count: (self.len as u32).to_le_bytes(),
            ..RawMbrEntry::default()
        }
    }
}
