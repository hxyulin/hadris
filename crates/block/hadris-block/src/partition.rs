use hadris_storage::{BlockError, PartitionView};

fn view<'a, S>(
    source: &'a mut S,
    start: u64,
    count: u64,
    block_size: u32,
) -> hadris_storage::Result<PartitionView<'a, S>, hadris_io::ErrorKind> {
    let offset = start
        .checked_mul(block_size as u64)
        .ok_or(BlockError::AddressOverflow)?;
    let length = count
        .checked_mul(block_size as u64)
        .ok_or(BlockError::AddressOverflow)?;
    PartitionView::new(source, offset, length)
}

/// Creates a bounded stream for an MBR partition entry.
pub fn mbr_partition_view<'a, S>(
    source: &'a mut S,
    entry: &hadris_part::MbrPartition,
    block_size: u32,
) -> hadris_storage::Result<PartitionView<'a, S>, hadris_io::ErrorKind> {
    view(
        source,
        entry.start_lba.to_ne() as u64,
        entry.sector_count.to_ne() as u64,
        block_size,
    )
}

/// Creates a bounded stream for a GPT partition entry.
pub fn gpt_partition_view<'a, S>(
    source: &'a mut S,
    entry: &hadris_part::GptPartitionEntry,
    block_size: u32,
) -> hadris_storage::Result<PartitionView<'a, S>, hadris_io::ErrorKind> {
    view(
        source,
        entry.first_lba.to_ne(),
        entry.size_sectors(),
        block_size,
    )
}
