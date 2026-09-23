use hadris_fs::ErrorKind;

use crate::error::{Detail, TableError};
use crate::{MbrType, PartitionFlags};

/// One GPT partition to mirror in a hybrid MBR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mirror {
    pub(crate) gpt_index: usize,
    pub(crate) kind: MbrType,
    pub(crate) flags: PartitionFlags,
}

/// How to build a hybrid MBR: which slot holds the protective entry and
/// which GPT partitions (at most three) the other slots mirror.
///
/// Hybrid MBRs are outside the UEFI specification. They let BIOS systems see
/// selected GPT partitions, as hybrid ISO images need, but tools that trust
/// the MBR may corrupt the GPT. The configuration holds no allocation, so it
/// behaves the same with and without `alloc`.
///
/// ```rust
/// use hadris_part::{HybridMbr, MbrType, PartitionFlags};
///
/// let mut config = HybridMbr::new();
/// config.add_mirrored(0, MbrType::EFI_SYSTEM, PartitionFlags::BOOTABLE).unwrap();
/// assert_eq!(config.mirrored_count(), 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HybridMbr {
    protective_slot: usize,
    mirrors: [Option<Mirror>; 3],
}

impl HybridMbr {
    /// A configuration with the protective entry in slot 0 and no mirrors.
    pub const fn new() -> Self {
        Self {
            protective_slot: 0,
            mirrors: [None; 3],
        }
    }

    /// Puts the protective entry in `slot` (0 to 3); mirrors fill the other
    /// slots in order.
    pub const fn with_protective_slot(mut self, slot: usize) -> Self {
        self.protective_slot = slot;
        self
    }

    /// Mirrors GPT partition `gpt_index` with MBR type `kind`. Only
    /// [`PartitionFlags::BOOTABLE`] can be stored.
    ///
    /// Fails with [`ErrorKind::LimitExceeded`] after three mirrors.
    pub fn add_mirrored(
        &mut self,
        gpt_index: usize,
        kind: MbrType,
        flags: PartitionFlags,
    ) -> Result<(), TableError> {
        let slot = self
            .mirrors
            .iter_mut()
            .find(|m| m.is_none())
            .ok_or(TableError::new(ErrorKind::LimitExceeded, Detail::TableFull))?;
        *slot = Some(Mirror {
            gpt_index,
            kind,
            flags,
        });
        Ok(())
    }

    /// The slot of the protective entry.
    pub const fn protective_slot(&self) -> usize {
        self.protective_slot
    }

    /// The number of mirrored partitions.
    pub fn mirrored_count(&self) -> usize {
        self.mirrors.iter().flatten().count()
    }

    /// The MBR slot of mirror `k`: the `k`-th slot that is not protective.
    pub(crate) fn mirror_slots(&self) -> impl Iterator<Item = (usize, Mirror)> + '_ {
        (0..4)
            .filter(|&slot| slot != self.protective_slot)
            .zip(self.mirrors.iter().flatten().copied())
    }
}

#[cfg(feature = "alloc")]
mod table {
    use hadris_fs::ErrorKind;

    use super::HybridMbr;
    use crate::disk::Partitions;
    use crate::error::{Detail, TableError};
    use crate::raw::{Chs, RawMbrEntry};
    use crate::{Gpt, MbrType, Partition};

    /// A GPT together with a hybrid MBR that mirrors some of its partitions.
    ///
    /// The GPT is authoritative: [`partitions`](Self::partitions) lists its
    /// entries, and [`mbr_partitions`](Self::mbr_partitions) the mirrors.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Hybrid {
        gpt: Gpt,
        entries: [RawMbrEntry; 4],
    }

    impl Hybrid {
        /// Builds the hybrid MBR for `gpt` from `config`.
        ///
        /// The protective entry covers the blocks from 1 up to the first
        /// mirrored partition, or the whole disk without mirrors. Fails with
        /// [`ErrorKind::InvalidInput`] when the protective slot is past 3, a
        /// mirror names an unused GPT slot or an empty, protective or extended
        /// type, or stores a flag other than `BOOTABLE`, and with
        /// [`ErrorKind::LimitExceeded`] when a mirrored partition ends past
        /// block `u32::MAX`.
        pub fn new(gpt: Gpt, config: &HybridMbr) -> Result<Self, TableError> {
            let protective = config.protective_slot();
            if protective > 3 {
                return Err(TableError::invalid(Detail::Mirror));
            }
            let mut entries = [RawMbrEntry::default(); 4];
            let mut first_mirror = u64::MAX;
            for (slot, mirror) in config.mirror_slots() {
                let kind = mirror.kind;
                if kind.is_empty() || kind.is_protective() || kind.is_extended() {
                    return Err(TableError::invalid(Detail::Mirror));
                }
                if !mirror.flags.fits_mbr() {
                    return Err(TableError::invalid(Detail::Flags));
                }
                let partition = gpt
                    .entry(mirror.gpt_index)
                    .ok_or(TableError::invalid(Detail::Mirror))?;
                if partition.is_empty() {
                    return Err(TableError::invalid(Detail::Mirror));
                }
                let start = u32::try_from(partition.start());
                let last = u32::try_from(partition.end() - 1);
                let (Ok(start), Ok(_)) = (start, last) else {
                    return Err(TableError::new(
                        ErrorKind::LimitExceeded,
                        Detail::FieldOverflow,
                    ));
                };
                let mut entry = RawMbrEntry::new(kind.code(), start, partition.len() as u32);
                entry.boot_indicator = mirror.flags.to_mbr();
                entries[slot] = entry;
                if partition.start() > 1 {
                    first_mirror = first_mirror.min(partition.start());
                }
            }
            let last_block = gpt.block_count().saturating_sub(1).min(u64::from(u32::MAX)) as u32;
            let end = if first_mirror == u64::MAX {
                last_block
            } else {
                (first_mirror - 1).min(u64::from(u32::MAX)) as u32
            }
            .max(1);
            entries[protective] = RawMbrEntry {
                boot_indicator: 0,
                start_chs: Chs::from_lba(1),
                kind: MbrType::GPT_PROTECTIVE.code(),
                end_chs: Chs::from_lba(end),
                start_lba: 1u32.to_le_bytes(),
                sector_count: end.to_le_bytes(),
            };
            Ok(Self { gpt, entries })
        }

        pub(crate) fn from_disk(gpt: Gpt, entries: [RawMbrEntry; 4]) -> Self {
            Self { gpt, entries }
        }

        /// The GPT.
        pub const fn gpt(&self) -> &Gpt {
            &self.gpt
        }

        /// The GPT, dropping the hybrid MBR.
        pub fn into_gpt(self) -> Gpt {
            self.gpt
        }

        /// The GPT partitions.
        pub fn partitions(&self) -> Partitions<'_> {
            self.gpt.partitions()
        }

        /// The MBR entries other than the protective one, indexed by slot.
        pub fn mbr_partitions(&self) -> impl Iterator<Item = Partition> + '_ {
            self.entries.iter().enumerate().filter_map(|(slot, entry)| {
                let kind = MbrType::new(entry.kind);
                (!kind.is_empty() && !kind.is_protective()).then(|| {
                    Partition::from_mbr(
                        slot,
                        u64::from(entry.start_lba()),
                        entry,
                        self.gpt.block_size(),
                    )
                })
            })
        }

        /// The raw MBR entry of slot `slot` (0 to 3), as it will be written.
        pub fn raw_entry(&self, slot: usize) -> Option<&RawMbrEntry> {
            self.entries.get(slot)
        }

        pub(crate) fn entries(&self) -> &[RawMbrEntry; 4] {
            &self.entries
        }
    }
}

#[cfg(feature = "alloc")]
pub use table::Hybrid;
