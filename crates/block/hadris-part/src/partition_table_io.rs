io_transform! {

use super::super::{Read, Seek, SeekFrom};
use super::mbr_io::MasterBootRecordReadExt;
use super::scheme_io::DiskPartitionSchemeReadExt;
use crate::{DiskPartitionScheme, MasterBootRecord, PartitionError, PartitionSchemeType, Result};

/// Cheaply detects the partition-table family and restores the source position.
#[cfg(all(feature = "alloc", feature = "read"))]
pub async fn detect<R>(source: &mut R) -> Result<PartitionSchemeType>
where
    R: Read + Seek<Error = <R as Read>::Error>,
{
    let original = source.stream_position().await.map_err(PartitionError::from)?;
    source.seek(SeekFrom::Start(0)).await.map_err(PartitionError::from)?;
    let detected = MasterBootRecord::read_from(source)
        .await
        .map(|mbr| crate::scheme::detect_scheme_from_mbr(&mbr));
    source.seek(SeekFrom::Start(original)).await.map_err(PartitionError::from)?;
    detected
}

/// Opens and validates an MBR, GPT, or hybrid partition table.
#[cfg(all(feature = "alloc", feature = "read"))]
pub async fn open<R>(source: &mut R, logical_block_size: u32) -> Result<DiskPartitionScheme>
where
    R: Read + Seek<Error = <R as Read>::Error>,
{
    DiskPartitionScheme::read_from(source, logical_block_size).await
}

} // io_transform!
