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
#[must_use]
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
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] after three mirrors.
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
    use crate::{Gpt, MbrType, Partition, PartitionFlags};

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
        /// mirrored partition, or the whole disk without mirrors.
        ///
        /// # Errors
        ///
        /// Returns [`ErrorKind::InvalidInput`] for an invalid protective slot,
        /// missing partition, invalid mirror type, or unsupported flags.
        /// Returns [`ErrorKind::LimitExceeded`] when a mirror exceeds MBR
        /// addressing limits. The consumed GPT is dropped on failure.
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
                let repeated = config
                    .mirror_slots()
                    .any(|(other, m)| other < slot && m.gpt_index == mirror.gpt_index);
                if repeated {
                    return Err(TableError::invalid(Detail::Mirror));
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

        /// The GPT, for edits.
        ///
        /// The MBR entries are kept as read or built. After an edit that
        /// moves a mirrored partition or changes the disk size, rebuild them
        /// with `Hybrid::new(hybrid.gpt().clone(), &hybrid.mirror())`.
        pub fn gpt_mut(&mut self) -> &mut Gpt {
            &mut self.gpt
        }

        /// The configuration that rebuilds this hybrid MBR with
        /// [`new`](Self::new): the slot of the protective entry, and each
        /// other entry in slot order as a mirror of the GPT partition that
        /// starts where it does, preferring one of the same length.
        ///
        /// Entries that start no GPT partition, that start one an earlier
        /// entry already mirrors, and extended entries are left out. A hybrid MBR built by [`new`](Self::new), or one whose
        /// mirrors directly follow each other around the protective entry
        /// as `gdisk` writes them, round-trips.
        pub fn mirror(&self) -> HybridMbr {
            let protective = self
                .entries
                .iter()
                .position(|e| MbrType::new(e.kind).is_protective())
                .unwrap_or(0);
            let mut config = HybridMbr::new().with_protective_slot(protective);
            for entry in &self.entries {
                let kind = MbrType::new(entry.kind);
                if kind.is_empty() || kind.is_protective() || kind.is_extended() {
                    continue;
                }
                let start = u64::from(entry.start_lba());
                let len = u64::from(entry.sector_count());
                let mirrored =
                    |slot: usize| config.mirror_slots().any(|(_, m)| m.gpt_index == slot);
                let mut starting = self
                    .gpt
                    .partitions()
                    .filter(|p| p.start() == start && !mirrored(p.slot()));
                let exact = starting.clone().find(|p| p.len() == len);
                let Some(partition) = exact.or_else(|| starting.next()) else {
                    continue;
                };
                let flags = PartitionFlags::from_mbr(entry.boot_indicator);
                if config.add_mirrored(partition.slot(), kind, flags).is_err() {
                    break;
                }
            }
            config
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
