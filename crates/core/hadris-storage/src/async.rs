use hadris_io::SeekFrom;
use hadris_io::r#async::{Read, Seek, Write};

use crate::PartitionView;

impl<S: Read + Seek> Read for PartitionView<'_, S> {
    async fn read(&mut self, buffer: &mut [u8]) -> hadris_io::Result<usize> {
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

impl<S: Read + Seek> Seek for PartitionView<'_, S> {
    async fn seek(&mut self, from: SeekFrom) -> hadris_io::Result<u64> {
        self.position = self.seek_position(from)?;
        Ok(self.position)
    }
}

impl<S: Read + Write + Seek> Write for PartitionView<'_, S> {
    async fn write(&mut self, buffer: &[u8]) -> hadris_io::Result<usize> {
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

    async fn flush(&mut self) -> hadris_io::Result<()> {
        self.source.flush().await
    }
}
