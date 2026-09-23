//! Partition entries as block-device slices, in logical blocks of the disk.
//!
//! The disk device's block size must be the logical block size the
//! partition table was written with.

fn mbr_range(entry: &hadris_part::MbrPartition) -> (u64, u64) {
    (
        entry.start_lba.to_ne() as u64,
        entry.sector_count.to_ne() as u64,
    )
}

fn gpt_range(entry: &hadris_part::GptPartitionEntry) -> (u64, u64) {
    (entry.first_lba.to_ne(), entry.size_sectors())
}

macro_rules! partition_mode {
    ($mode:ident) => {
        use hadris_storage::BlockIndex;
        use hadris_storage::$mode::{BlockDevice, Slice};

        /// Restricts `disk` to an MBR partition. Fails, returning `disk`,
        /// when the partition does not fit on it.
        pub fn mbr_partition<D: BlockDevice>(
            disk: D,
            entry: &hadris_part::MbrPartition,
        ) -> Result<Slice<D>, D> {
            let (first, count) = super::mbr_range(entry);
            Slice::new(disk, BlockIndex(first), count)
        }

        /// Restricts `disk` to a GPT partition. Fails, returning `disk`,
        /// when the partition does not fit on it.
        pub fn gpt_partition<D: BlockDevice>(
            disk: D,
            entry: &hadris_part::GptPartitionEntry,
        ) -> Result<Slice<D>, D> {
            let (first, count) = super::gpt_range(entry);
            Slice::new(disk, BlockIndex(first), count)
        }
    };
}

/// Synchronous partition slices.
#[cfg(feature = "sync")]
pub mod sync {
    partition_mode!(sync);
}

/// Asynchronous partition slices.
#[cfg(feature = "async")]
pub mod r#async {
    partition_mode!(r#async);
}

/// Asynchronous partition slices over the `Send` devices of
/// `hadris_storage::async_send`.
#[cfg(feature = "async-send")]
pub mod async_send {
    partition_mode!(async_send);
}
