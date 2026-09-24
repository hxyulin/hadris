use core::ops::ControlFlow;

use hadris_fs::ErrorKind;
use hadris_storage::{BlockIndex, BlockSize};

use super::storage::{BlockDevice, Slice};
use crate::codec::{self, Array, Logical, MAX_LOGICAL};
use crate::error::{Detail, Error};
use crate::raw::{RawGptEntry, RawGptHeader, RawMbr};
#[cfg(feature = "alloc")]
use crate::{Disk, DiskLayout, Gpt, Hybrid, Mbr, PartitionTable};
use crate::{GptCopy, MbrType, Partition, TableKind};

/// The largest block [`scan`] can read without an allocator.
const MAX_SCAN_BLOCK: usize = 4096;

/// A header and entry array that passed validation, or why they did not.
type Loaded = Result<(RawGptHeader, Array), Detail>;

/// A validated GPT copy.
struct Found {
    header: RawGptHeader,
    array: Array,
    copy: GptCopy,
    damaged: Option<GptCopy>,
    /// The array block of the backup copy, when it is valid.
    backup_entries: Option<u64>,
}

fn geometry<D: BlockDevice>(dev: &D) -> Result<(BlockSize, u64), Error<D::Error>> {
    let size = dev.block_size();
    codec::check_block_size(size)?;
    Ok((size, dev.block_count()))
}

io_transform! {

async fn read_block<D: BlockDevice>(
    dev: &mut D,
    lba: u64,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    dev.read_blocks(BlockIndex::new(lba), buf).await.map_err(Error::from)
}

async fn read_record<D: BlockDevice>(
    dev: &mut D,
    lba: u64,
    buf: &mut [u8],
) -> Result<RawMbr, Error<D::Error>> {
    read_block(dev, lba, buf).await?;
    Ok(bytemuck::pod_read_unaligned(&buf[..size_of::<RawMbr>()]))
}

/// The header and array at `lba` when both pass validation, the reason when
/// they do not, and `Err` only when the device fails.
async fn load_copy<D: BlockDevice>(
    dev: &mut D,
    lba: u64,
    block_count: u64,
    buf: &mut [u8],
) -> Result<Loaded, Error<D::Error>> {
    if lba == 0 || lba >= block_count {
        return Ok(Err(Detail::GptHeader));
    }
    read_block(dev, lba, buf).await?;
    let size = dev.block_size();
    let (header, array) = match codec::check_header(buf, lba, block_count, size) {
        Ok(found) => found,
        Err(detail) => return Ok(Err(detail)),
    };
    let mut crc = crate::raw::Crc32::new();
    let mut left = array.bytes;
    for block in 0..array.blocks {
        read_block(dev, array.lba + block, buf).await?;
        let take = left.min(buf.len() as u64) as usize;
        crc.update(&buf[..take]);
        left -= take as u64;
    }
    if crc.finish() != header.partition_entry_array_crc32() {
        return Ok(Err(Detail::GptEntriesCrc));
    }
    Ok(Ok((header, array)))
}

/// Finds a valid GPT copy, preferring the primary.
async fn find_gpt<D: BlockDevice>(
    dev: &mut D,
    block_count: u64,
    buf: &mut [u8],
) -> Result<Found, Error<D::Error>> {
    let primary = load_copy(dev, 1, block_count, buf).await?;
    let backup_lba = match &primary {
        Ok((header, _)) => header.alternate_lba(),
        Err(_) => block_count.saturating_sub(1),
    };
    let backup = load_copy(dev, backup_lba, block_count, buf).await?;
    match (primary, backup) {
        (Ok((header, array)), backup) => {
            let backup_entries = match backup {
                Ok((other, other_array)) if codec::same_table(&header, &other) => {
                    Some(other_array.lba)
                }
                _ => None,
            };
            Ok(Found {
                header,
                array,
                copy: GptCopy::Primary,
                damaged: backup_entries.is_none().then_some(GptCopy::Backup),
                backup_entries,
            })
        }
        (Err(_), Ok((header, array))) => Ok(Found {
            header,
            array,
            copy: GptCopy::Backup,
            damaged: Some(GptCopy::Primary),
            backup_entries: Some(array.lba),
        }),
        (Err(detail), Err(_)) => Err(Error::corrupt(detail)),
    }
}

/// Passes each entry of a validated array to `f`, with its slot.
async fn visit_entries<D: BlockDevice>(
    dev: &mut D,
    array: &Array,
    buf: &mut [u8],
    mut f: impl FnMut(usize, RawGptEntry) -> ControlFlow<()>,
) -> Result<(), Error<D::Error>> {
    let block = buf.len() as u64;
    let size = u64::from(array.entry_size);
    let count = u64::from(array.count);
    for index in 0..array.blocks {
        let first_byte = index * block;
        let mut slot = first_byte.div_ceil(size);
        if slot >= count || slot * size >= first_byte + block {
            continue;
        }
        read_block(dev, array.lba + index, buf).await?;
        while slot < count && slot * size < first_byte + block {
            let at = (slot * size - first_byte) as usize;
            let entry = bytemuck::pod_read_unaligned(&buf[at..at + size_of::<RawGptEntry>()]);
            if f(slot as usize, entry).is_break() {
                return Ok(());
            }
            slot += 1;
        }
    }
    Ok(())
}

/// Follows the extended boot records of the extended partition
/// `ext_start..ext_end`, passing each logical partition to `f`.
async fn walk_ebr<D: BlockDevice>(
    dev: &mut D,
    ext_start: u64,
    ext_end: u64,
    buf: &mut [u8],
    mut f: impl FnMut(Logical) -> ControlFlow<()>,
) -> Result<(), Error<D::Error>> {
    let chain = || Error::corrupt(Detail::EbrChain);
    let mut ebr = ext_start;
    for step in 0..=MAX_LOGICAL {
        if ebr >= ext_end || ebr >= dev.block_count() {
            return Err(chain());
        }
        let record = read_record(dev, ebr, buf).await?;
        if !record.has_signature() {
            return if step == 0 { Ok(()) } else { Err(chain()) };
        }
        let [logical, link, ..] = record.entries;
        if (logical.boot_indicator | link.boot_indicator) & !crate::raw::MBR_ACTIVE != 0 {
            return Err(Error::corrupt(Detail::MbrEntry));
        }
        if !logical.is_empty() {
            if MbrType::new(logical.kind).is_extended() {
                return Err(chain());
            }
            let found = Logical {
                ebr,
                start: ebr + u64::from(logical.start_lba()),
                len: u64::from(logical.sector_count()),
                kind: logical.kind,
                boot: logical.boot_indicator,
            };
            if f(found).is_break() {
                return Ok(());
            }
        }
        if link.is_empty() || !MbrType::new(link.kind).is_extended() {
            return Ok(());
        }
        let next = ext_start + u64::from(link.start_lba());
        if next <= ebr {
            return Err(chain());
        }
        ebr = next;
    }
    Err(chain())
}

/// The extended partition of an MBR record, as `(start, end)`.
fn extended_range(record: &RawMbr) -> Option<(u64, u64)> {
    let entry = record
        .entries
        .iter()
        .find(|e| MbrType::new(e.kind).is_extended())?;
    let start = u64::from(entry.start_lba());
    Some((start, start + u64::from(entry.sector_count())))
}

/// Reads the partition table of `dev`, with the device's block size.
///
/// A GPT whose primary copy fails validation is read from the backup, and
/// the other way round; [`Gpt::damaged_copy`] names the copy that failed,
/// and [`write`](write()) repairs it. Logical partitions are read from the chain of
/// extended boot records.
///
/// Fails with [`ErrorKind::NotFound`] when block 0 has no boot signature,
/// [`ErrorKind::Corrupt`] when the MBR, the EBR chain or both GPT copies
/// are invalid, [`ErrorKind::Unsupported`] unless the block size is a power
/// of two of at least 512 bytes, and [`ErrorKind::Io`] when the device fails.
#[cfg(feature = "alloc")]
pub async fn read<D: BlockDevice>(dev: &mut D) -> Result<Disk, Error<D::Error>> {
    let (size, block_count) = geometry(dev)?;
    let mut buf = alloc::vec![0u8; size.get() as usize];
    let record = read_record(dev, 0, &mut buf).await?;
    let kind = codec::classify(&record)?;
    let table = match kind {
        TableKind::Mbr => {
            let mut logical = alloc::vec::Vec::new();
            if let Some((start, end)) = extended_range(&record) {
                walk_ebr(dev, start, end, &mut buf, |l| {
                    logical.push(l);
                    ControlFlow::Continue(())
                })
                .await?;
            }
            PartitionTable::Mbr(Mbr::from_disk(&record, logical, block_count, size))
        }
        _ => {
            let found = find_gpt(dev, block_count, &mut buf).await?;
            let mut entries = alloc::vec::Vec::with_capacity(found.array.count as usize);
            visit_entries(dev, &found.array, &mut buf, |_, entry| {
                entries.push(entry);
                ControlFlow::Continue(())
            })
            .await?;
            let gpt = Gpt::from_disk(
                &found.header,
                found.copy,
                entries,
                found.damaged,
                found.backup_entries,
                block_count,
                size,
            );
            if kind == TableKind::Hybrid {
                PartitionTable::Hybrid(Hybrid::from_disk(gpt, record.entries))
            } else {
                PartitionTable::Gpt(gpt)
            }
        }
    };
    Ok(Disk::from_disk(table, &record))
}

/// Lists the partitions of `dev` without an allocator, passing each to `f`
/// until it breaks, and returns the kind of table.
///
/// It validates as [`read`] does, including both GPT CRCs and the fallback
/// to the backup GPT, and lists partitions in the same order. Blocks larger
/// than 4096 bytes fail with [`ErrorKind::Unsupported`].
pub async fn scan<D: BlockDevice>(
    dev: &mut D,
    mut f: impl FnMut(Partition) -> ControlFlow<()>,
) -> Result<TableKind, Error<D::Error>> {
    let (size, block_count) = geometry(dev)?;
    let mut storage = [0u8; MAX_SCAN_BLOCK];
    let buf = storage
        .get_mut(..size.get() as usize)
        .ok_or(Error::new(ErrorKind::Unsupported, Detail::BlockSize))?;
    let record = read_record(dev, 0, buf).await?;
    let kind = codec::classify(&record)?;
    if kind == TableKind::Mbr {
        for (slot, entry) in record.entries.iter().enumerate() {
            let entry_kind = MbrType::new(entry.kind);
            if entry_kind.is_empty() || entry_kind.is_extended() {
                continue;
            }
            let partition = Partition::from_mbr(slot, u64::from(entry.start_lba()), entry, size);
            if f(partition).is_break() {
                return Ok(kind);
            }
        }
        if let Some((start, end)) = extended_range(&record) {
            let mut index = codec::FIRST_LOGICAL;
            walk_ebr(dev, start, end, buf, |l| {
                let partition = Partition::from_mbr(index, l.start, &l.entry(), size);
                index += 1;
                f(partition)
            })
            .await?;
        }
        return Ok(kind);
    }
    let found = find_gpt(dev, block_count, buf).await?;
    visit_entries(dev, &found.array, buf, |slot, entry| {
        if entry.is_unused() {
            return ControlFlow::Continue(());
        }
        f(Partition::from_gpt(slot, &entry, size))
    })
    .await?;
    Ok(kind)
}

/// Writes `disk` to `dev` and flushes it: every block [`Disk::runs`]
/// lists, block 0 last.
///
/// A GPT gets both copies with fresh CRCs, which repairs a damaged copy.
/// Fails with [`ErrorKind::InvalidInput`] when the block sizes differ,
/// [`ErrorKind::NoSpace`] when the disk has more blocks than the device or
/// a GPT copy would not fit beside the usable area (as on a truncated
/// image),
/// and [`ErrorKind::ReadOnly`] when the device refuses writes.
#[cfg(feature = "alloc")]
pub async fn write<D: BlockDevice>(dev: &mut D, disk: &Disk) -> Result<(), Error<D::Error>> {
    if dev.block_size() != disk.block_size() {
        return Err(Error::new(ErrorKind::InvalidInput, Detail::BlockSize));
    }
    let gpt = match disk.table() {
        PartitionTable::Gpt(gpt) => Some(gpt),
        PartitionTable::Hybrid(hybrid) => Some(hybrid.gpt()),
        _ => None,
    };
    let fits = gpt.is_none_or(|gpt| gpt.fits(dev.block_count()));
    if disk.block_count() > dev.block_count() || !fits {
        return Err(Error::new(ErrorKind::NoSpace, Detail::DiskTooSmall));
    }
    for run in disk.runs() {
        dev.write_blocks(BlockIndex::new(run.lba()), run.bytes()).await?;
    }
    dev.flush().await?;
    Ok(())
}

/// Builds `layout` for the geometry of `dev`, writes it, and returns the
/// disk.
///
/// ```rust
/// # #[cfg(feature = "sync")]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_part::gpt::types;
/// use hadris_part::{DiskLayout, Guid, PartitionSpec, Size};
/// use hadris_storage::{BlockSize, MemDevice};
///
/// let mut dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
/// let layout = DiskLayout::gpt(Guid::from_bytes([5; 16]))
///     .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(4)).with_name("EFI"))
///     .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining));
/// let disk = hadris_part::sync::create(&mut dev, &layout)?;
/// let esp = disk.partition(0).unwrap();
/// let slice = hadris_part::sync::open(&mut dev, &esp)?;
/// assert_eq!(hadris_storage::sync::BlockDevice::block_count(&slice), 8192);
/// assert_eq!(hadris_part::sync::read(&mut dev)?, disk);
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "sync"))]
/// # fn main() {}
/// ```
#[cfg(feature = "alloc")]
pub async fn create<D: BlockDevice>(
    dev: &mut D,
    layout: &DiskLayout,
) -> Result<Disk, Error<D::Error>> {
    let disk = layout.build(dev.block_count(), dev.block_size())?;
    write(dev, &disk).await?;
    Ok(disk)
}

} // io_transform!

/// A block device of `partition` on `dev`, the disk it was read from.
///
/// `dev` can be owned or `&mut`. Fails with [`ErrorKind::InvalidInput`]
/// when the block size is not the partition's or the partition does not
/// fit on `dev`; an owned device is dropped then, so pass `&mut dev` to
/// keep it.
pub fn open<D: BlockDevice>(dev: D, partition: &Partition) -> Result<Slice<D>, Error<D::Error>> {
    if dev.block_size() != partition.block_size() {
        return Err(Error::new(ErrorKind::InvalidInput, Detail::BlockSize));
    }
    Slice::new(dev, BlockIndex::new(partition.start()), partition.len()).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            Detail::OutOfBounds {
                index: partition.index(),
            },
        )
    })
}
