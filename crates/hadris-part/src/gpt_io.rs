io_transform! {

use super::super::{Read, Write, Seek, SeekFrom};
use crate::gpt::{GptHeader, GptHeaderRaw};

// I/O operations for GptHeader

/// Extension trait for reading/writing [`GptHeader`] from/to I/O sources.
#[cfg(feature = "read")]
pub trait GptHeaderReadExt: Sized {
    /// Reads a GPT header from a reader.
    ///
    /// The reader should be positioned at the start of the header (typically LBA 1).
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or if the signature is invalid.
    async fn read_from<R: Read>(reader: &mut R) -> crate::error::Result<Self>;

    /// Reads a GPT header from a specific LBA.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking/reading fails or if the signature is invalid.
    async fn read_from_lba<R: Read + Seek>(
        reader: &mut R,
        lba: u64,
        block_size: u32,
    ) -> crate::error::Result<Self>;
}

#[cfg(feature = "read")]
impl GptHeaderReadExt for GptHeader {
    async fn read_from<R: Read>(reader: &mut R) -> crate::error::Result<Self> {
        let mut buf = [0u8; GptHeaderRaw::SIZE];
        reader
            .read_exact(&mut buf)
            .await
            .map_err(|_| crate::error::PartitionError::Io)?;
        let raw: GptHeaderRaw = bytemuck::cast(buf);
        let header = Self::from_raw(&raw);

        if !header.has_valid_signature() {
            return Err(crate::error::PartitionError::InvalidGptSignature {
                found: header.signature,
            });
        }

        Ok(header)
    }

    async fn read_from_lba<R: Read + Seek>(
        reader: &mut R,
        lba: u64,
        block_size: u32,
    ) -> crate::error::Result<Self> {
        reader
            .seek(SeekFrom::Start(lba * block_size as u64))
            .await
            .map_err(|_| crate::error::PartitionError::Io)?;
        Self::read_from(reader).await
    }
}

/// Extension trait for writing [`GptHeader`] to I/O sinks.
#[cfg(feature = "write")]
pub trait GptHeaderWriteExt {
    /// Writes this GPT header to a writer.
    ///
    /// Only writes the 92-byte header, not padding to sector size.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    async fn write_to<W: Write>(&self, writer: &mut W) -> crate::error::Result<()>;

    /// Writes this GPT header to a specific LBA, padded to block size.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking/writing fails.
    async fn write_to_lba<W: Write + Seek>(
        &self,
        writer: &mut W,
        lba: u64,
        block_size: u32,
    ) -> crate::error::Result<()>;
}

#[cfg(feature = "write")]
impl GptHeaderWriteExt for GptHeader {
    async fn write_to<W: Write>(&self, writer: &mut W) -> crate::error::Result<()> {
        let raw = self.to_raw();
        writer
            .write_all(bytemuck::bytes_of(&raw))
            .await
            .map_err(|_| crate::error::PartitionError::Io)
    }

    async fn write_to_lba<W: Write + Seek>(
        &self,
        writer: &mut W,
        lba: u64,
        block_size: u32,
    ) -> crate::error::Result<()> {
        writer
            .seek(SeekFrom::Start(lba * block_size as u64))
            .await
            .map_err(|_| crate::error::PartitionError::Io)?;

        let raw = self.to_raw();
        writer
            .write_all(bytemuck::bytes_of(&raw))
            .await
            .map_err(|_| crate::error::PartitionError::Io)?;

        // Pad to block size
        let padding_size = block_size as usize - GptHeaderRaw::SIZE;
        if padding_size > 0 {
            let padding = [0u8; 512]; // Use 512 as max typical block size
            writer
                .write_all(&padding[..padding_size.min(512)])
                .await
                .map_err(|_| crate::error::PartitionError::Io)?;
        }

        Ok(())
    }
}

} // io_transform!
