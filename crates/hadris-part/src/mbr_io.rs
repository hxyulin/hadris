io_transform! {

use super::super::{Read, Write};
use crate::mbr::MasterBootRecord;

// I/O operations

/// Extension trait for reading [`MasterBootRecord`] from I/O sources.
#[cfg(feature = "read")]
pub trait MasterBootRecordReadExt: Sized {
    /// Reads an MBR from the beginning of a reader.
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or if the MBR signature is invalid.
    async fn read_from<R: Read>(reader: &mut R) -> crate::error::Result<Self>;
}

#[cfg(feature = "read")]
impl MasterBootRecordReadExt for MasterBootRecord {
    async fn read_from<R: Read>(reader: &mut R) -> crate::error::Result<Self> {
        let mut buf = [0u8; 512];
        reader
            .read_exact(&mut buf)
            .await
            .map_err(|_| crate::error::PartitionError::Io)?;
        let mbr: Self = bytemuck::cast(buf);
        if !mbr.has_valid_signature() {
            return Err(crate::error::PartitionError::InvalidMbrSignature {
                found: mbr.signature,
            });
        }
        Ok(mbr)
    }
}

/// Extension trait for writing [`MasterBootRecord`] to I/O sinks.
#[cfg(feature = "write")]
pub trait MasterBootRecordWriteExt {
    /// Writes this MBR to a writer.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    async fn write_to<W: Write>(&self, writer: &mut W) -> crate::error::Result<()>;
}

#[cfg(feature = "write")]
impl MasterBootRecordWriteExt for MasterBootRecord {
    async fn write_to<W: Write>(&self, writer: &mut W) -> crate::error::Result<()> {
        writer
            .write_all(bytemuck::bytes_of(self))
            .await
            .map_err(|_| crate::error::PartitionError::Io)
    }
}

} // io_transform!
