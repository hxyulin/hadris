use hadris_io::SeekFrom;
use hadris_io::r#async::{Read, Seek, Write};
use hadris_io::legacy::r#async as legacy;

use crate::PartitionView;

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

impl<S: legacy::Read + legacy::Seek> legacy::Read for PartitionView<'_, S> {
    async fn read(&mut self, buffer: &mut [u8]) -> hadris_io::legacy::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source.seek(SeekFrom::Start(absolute)).await?;
        let read = self.source.read(&mut buffer[..length]).await?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<S: legacy::Read + legacy::Seek> legacy::Seek for PartitionView<'_, S> {
    async fn seek(&mut self, from: SeekFrom) -> hadris_io::legacy::Result<u64> {
        self.position = self.seek_position(from)?;
        Ok(self.position)
    }
}

impl<S: legacy::Read + legacy::Write + legacy::Seek> legacy::Write for PartitionView<'_, S> {
    async fn write(&mut self, buffer: &[u8]) -> hadris_io::legacy::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source.seek(SeekFrom::Start(absolute)).await?;
        let written = self.source.write(&buffer[..length]).await?;
        self.position += written as u64;
        Ok(written)
    }

    async fn flush(&mut self) -> hadris_io::legacy::Result<()> {
        self.source.flush().await
    }
}
