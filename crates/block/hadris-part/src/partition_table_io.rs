io_transform! {

use super::super::{Read, Seek, SeekFrom};
use super::mbr_io::MasterBootRecordReadExt;
use super::scheme_io::PartitionTableReadExt;
use crate::{PartitionTable, MasterBootRecord, Error, PartitionSchemeType, Result};

/// Cheaply detects the partition-table family and restores the source position.
#[cfg(all(feature = "alloc", feature = "read"))]
pub async fn detect<R>(source: &mut R) -> Result<PartitionSchemeType>
where
    R: Read + Seek<Error = <R as Read>::Error>,
{
    let original = source.stream_position().await.map_err(Error::from)?;
    source.seek(SeekFrom::Start(0)).await.map_err(Error::from)?;
    let detected = MasterBootRecord::read_from(source)
        .await
        .map(|mbr| crate::scheme::detect_scheme_from_mbr(&mbr));
    source.seek(SeekFrom::Start(original)).await.map_err(Error::from)?;
    detected
}

/// Opens and validates an MBR, GPT, or hybrid partition table.
#[cfg(all(feature = "alloc", feature = "read"))]
pub async fn open<R>(source: &mut R, logical_block_size: u32) -> Result<PartitionTable>
where
    R: Read + Seek<Error = <R as Read>::Error>,
{
    PartitionTable::read_from(source, logical_block_size).await
}

} // io_transform!
