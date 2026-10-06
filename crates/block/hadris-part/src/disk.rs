use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::iter::FusedIterator;

use hadris_fs::ErrorKind;
use hadris_storage::BlockSize;

use crate::codec::FIRST_LOGICAL;
use crate::error::{Detail, TableError};
use crate::raw::{self, RawMbr, RawMbrEntry};
use crate::{Gpt, GptCopy, Hybrid, Mbr, MbrType, Partition, TableKind};

/// The partition table of a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartitionTable {
    /// An MBR, possibly with logical partitions.
    Mbr(Mbr),
    /// A GPT behind a protective MBR.
    Gpt(Gpt),
    /// A GPT behind a hybrid MBR that mirrors some of its partitions.
    Hybrid(Hybrid),
}

impl PartitionTable {
    /// Which kind of table this is.
    pub const fn kind(&self) -> TableKind {
        match self {
            Self::Mbr(_) => TableKind::Mbr,
            Self::Gpt(_) => TableKind::Gpt,
            Self::Hybrid(_) => TableKind::Hybrid,
        }
    }

    /// The disk's block size.
    pub const fn block_size(&self) -> BlockSize {
        match self {
            Self::Mbr(mbr) => mbr.block_size(),
            Self::Gpt(gpt) => gpt.block_size(),
            Self::Hybrid(hybrid) => hybrid.gpt().block_size(),
        }
    }

    /// The number of blocks of the disk.
    pub const fn block_count(&self) -> u64 {
        match self {
            Self::Mbr(mbr) => mbr.block_count(),
            Self::Gpt(gpt) => gpt.block_count(),
            Self::Hybrid(hybrid) => hybrid.gpt().block_count(),
        }
    }

    /// The partitions: MBR primaries and logicals, or the GPT entries of a
    /// GPT or hybrid table.
    pub fn partitions(&self) -> Partitions<'_> {
        match self {
            Self::Mbr(mbr) => mbr.partitions(),
            Self::Gpt(gpt) => gpt.partitions(),
            Self::Hybrid(hybrid) => hybrid.partitions(),
        }
    }
}

impl From<Mbr> for PartitionTable {
    fn from(table: Mbr) -> Self {
        Self::Mbr(table)
    }
}

impl From<Gpt> for PartitionTable {
    fn from(table: Gpt) -> Self {
        Self::Gpt(table)
    }
}

impl From<Hybrid> for PartitionTable {
    fn from(table: Hybrid) -> Self {
        Self::Hybrid(table)
    }
}

/// A partitioned disk: its table and the boot code in block 0.
///
/// `Disk` does no I/O. `read`, `write` and `open` in the mode modules
/// (`hadris_part::sync`, `r#async`) move it to and from a
/// block device, and [`runs`](Self::runs) gives its bytes for writers that
/// are not block devices.
///
/// ```rust
/// use hadris_part::gpt::types;
/// use hadris_part::{DiskLayout, Guid, PartitionSpec, Size};
/// use hadris_storage::BlockSize;
///
/// let disk = DiskLayout::gpt(Guid::from_bytes([3; 16]))
///     .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(1)).with_name("EFI"))
///     .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining))
///     .build(8192, BlockSize::new(512).unwrap())
///     .unwrap();
/// let esp = disk.partition(0).unwrap();
/// assert_eq!((esp.start(), esp.len(), esp.size_bytes()), (2048, 2048, 1 << 20));
/// assert_eq!(disk.partitions().count(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    table: PartitionTable,
    bootstrap: [u8; 446],
}

impl Disk {
    /// A disk with `table` and no boot code.
    pub fn new(table: impl Into<PartitionTable>) -> Self {
        Self {
            table: table.into(),
            bootstrap: [0; 446],
        }
    }

    pub(crate) fn from_disk(table: PartitionTable, record: &RawMbr) -> Self {
        let mut bootstrap = [0; 446];
        bootstrap[..440].copy_from_slice(&record.bootstrap);
        bootstrap[440..444].copy_from_slice(&record.disk_signature);
        bootstrap[444..].copy_from_slice(&record.reserved);
        Self { table, bootstrap }
    }

    /// The partition table.
    pub const fn table(&self) -> &PartitionTable {
        &self.table
    }

    /// The partition table, for edits.
    pub fn table_mut(&mut self) -> &mut PartitionTable {
        &mut self.table
    }

    /// The partition table, dropping the boot code.
    pub fn into_table(self) -> PartitionTable {
        self.table
    }

    /// The disk's block size.
    pub const fn block_size(&self) -> BlockSize {
        self.table.block_size()
    }

    /// The number of blocks of the disk.
    pub const fn block_count(&self) -> u64 {
        self.table.block_count()
    }

    /// The partitions, as [`PartitionTable::partitions`] lists them.
    pub fn partitions(&self) -> Partitions<'_> {
        self.table.partitions()
    }

    /// The `n`-th partition that [`partitions`](Self::partitions) lists.
    pub fn partition(&self, n: usize) -> Option<Partition> {
        self.partitions().nth(n)
    }

    /// The first 446 bytes of block 0: boot code, and on MBR disks the disk
    /// signature at bytes 440 to 443.
    pub const fn bootstrap(&self) -> &[u8; 446] {
        &self.bootstrap
    }

    /// Replaces the start of the boot code area with `code` and zeroes the
    /// rest. Fails with [`ErrorKind::LimitExceeded`] beyond 446 bytes.
    pub fn set_bootstrap(&mut self, code: &[u8]) -> Result<(), TableError> {
        let area = self
            .bootstrap
            .get_mut(..code.len())
            .ok_or(TableError::new(ErrorKind::LimitExceeded, Detail::Bootstrap))?;
        area.copy_from_slice(code);
        self.bootstrap[code.len()..].fill(0);
        Ok(())
    }

    /// The bytes the table occupies on disk, as runs of whole blocks.
    ///
    /// For a GPT: the backup entry array and header, the primary entry
    /// array and header, then block 0. For an MBR: each extended boot record,
    /// then block 0. Headers and records are padded with zeros to a whole
    /// block. Writing the runs in order leaves block 0 for last, so an
    /// interrupted write never shows a table whose structures are missing.
    pub fn runs(&self) -> Runs<'_> {
        let block = self.block_size().get() as usize;
        let mut runs = Vec::new();
        let padded = |bytes: &[u8]| {
            let mut out = alloc::vec![0u8; bytes.len().div_ceil(block) * block];
            out[..bytes.len()].copy_from_slice(bytes);
            out
        };
        let (gpt, entries) = match &self.table {
            PartitionTable::Mbr(mbr) => {
                for (lba, record) in mbr.ebr_records() {
                    runs.push(Run::owned(lba, padded(bytemuck::bytes_of(&record))));
                }
                (None, *mbr.primary_entries())
            }
            PartitionTable::Gpt(gpt) => {
                let size = gpt.block_count().saturating_sub(1).min(u64::from(u32::MAX)) as u32;
                let mut entries = [RawMbrEntry::default(); 4];
                entries[0] = RawMbrEntry::new(MbrType::GPT_PROTECTIVE.code(), 1, size);
                (Some(gpt), entries)
            }
            PartitionTable::Hybrid(hybrid) => (Some(hybrid.gpt()), *hybrid.entries()),
        };
        if let Some(gpt) = gpt {
            let array = gpt.array_bytes();
            let crc = raw::crc32(&array);
            let array = if array.len() % block == 0 {
                array
            } else {
                Cow::Owned(padded(&array))
            };
            runs.push(Run {
                lba: gpt.backup_entries_lba(),
                bytes: array.clone(),
            });
            let backup = gpt.header(GptCopy::Backup, crc);
            runs.push(Run::owned(
                gpt.backup_lba(),
                padded(bytemuck::bytes_of(&backup)),
            ));
            runs.push(Run {
                lba: gpt.primary_entries_lba(),
                bytes: array,
            });
            let primary = gpt.header(GptCopy::Primary, crc);
            runs.push(Run::owned(1, padded(bytemuck::bytes_of(&primary))));
        }
        let mut record = RawMbr {
            entries,
            ..RawMbr::default()
        };
        record.bootstrap.copy_from_slice(&self.bootstrap[..440]);
        record
            .disk_signature
            .copy_from_slice(&self.bootstrap[440..444]);
        record.reserved.copy_from_slice(&self.bootstrap[444..]);
        runs.push(Run::owned(0, padded(bytemuck::bytes_of(&record))));
        Runs {
            inner: runs.into_iter(),
        }
    }
}

/// Bytes of a partition table that start at block [`lba`](Self::lba) and
/// fill whole blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run<'a> {
    lba: u64,
    bytes: Cow<'a, [u8]>,
}

impl Run<'_> {
    fn owned(lba: u64, bytes: Vec<u8>) -> Self {
        Self {
            lba,
            bytes: Cow::Owned(bytes),
        }
    }

    /// The first block.
    pub const fn lba(&self) -> u64 {
        self.lba
    }

    /// The bytes, a whole number of blocks.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The runs of [`Disk::runs`].
#[derive(Debug)]
pub struct Runs<'a> {
    inner: alloc::vec::IntoIter<Run<'a>>,
}

impl<'a> Iterator for Runs<'a> {
    type Item = Run<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for Runs<'_> {}

impl FusedIterator for Runs<'_> {}

/// An iterator over the partitions of a table.
#[derive(Debug, Clone)]
pub struct Partitions<'a> {
    inner: Inner<'a>,
    next: usize,
}

#[derive(Debug, Clone)]
enum Inner<'a> {
    Mbr(&'a Mbr),
    Gpt(&'a Gpt),
}

impl<'a> Partitions<'a> {
    pub(crate) fn mbr(mbr: &'a Mbr) -> Self {
        Self {
            inner: Inner::Mbr(mbr),
            next: 0,
        }
    }

    pub(crate) fn gpt(gpt: &'a Gpt) -> Self {
        Self {
            inner: Inner::Gpt(gpt),
            next: 0,
        }
    }
}

impl Iterator for Partitions<'_> {
    type Item = Partition;

    fn next(&mut self) -> Option<Partition> {
        match self.inner {
            Inner::Mbr(mbr) => {
                let end = FIRST_LOGICAL + mbr.logical_count();
                while self.next < end {
                    let index = self.next;
                    self.next += 1;
                    let Some(partition) = mbr.entry(index) else {
                        continue;
                    };
                    let extended = index < FIRST_LOGICAL
                        && matches!(partition.kind(), crate::PartitionKind::Mbr(k) if k.is_extended());
                    if !extended {
                        return Some(partition);
                    }
                }
                None
            }
            Inner::Gpt(gpt) => {
                while self.next < gpt.entry_count() {
                    let index = self.next;
                    self.next += 1;
                    if let Some(partition) = gpt.entry(index) {
                        return Some(partition);
                    }
                }
                None
            }
        }
    }
}

impl FusedIterator for Partitions<'_> {}
