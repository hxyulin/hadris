use core::{
    fmt,
    ops::{Add, AddAssign},
};

pub use super::super::{Parsable, Read, ReadExt, Seek, Writable, Write};
pub use hadris_io::{Error, ErrorKind, Result, SeekFrom, try_io_result_option};

/// A Logical Sector, size has to be 2^n and > 2048
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalSector(pub usize);

impl Add<usize> for LogicalSector {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign<usize> for LogicalSector {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

/// A Logical Sector, size has to be 2^n and > 512
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct _LogicalBlock(pub usize);

/// Represents IsoCursor.
pub struct IsoCursor<DATA: Seek> {
    /// The `data` field.
    pub data: DATA,
    /// The `sector_size` field.
    pub sector_size: usize,
}

io_transform! {

impl<DATA: Read + Seek> Read for IsoCursor<DATA> {
    type Error = <DATA as Read>::Error;

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.data.read(buf).await
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.data.read_exact(buf).await
    }
}

impl<DATA: Seek> Seek for IsoCursor<DATA> {
    type Error = DATA::Error;

    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.data.seek(pos).await
    }

    async fn stream_position(&mut self) -> Result<u64, Self::Error> {
        self.data.stream_position().await
    }

    async fn seek_relative(&mut self, offset: i64) -> Result<(), Self::Error> {
        self.data.seek_relative(offset).await
    }
}

impl<DATA: Seek> IsoCursor<DATA> {
    /// Performs the `new` operation.
    pub fn new(data: DATA, sector_size: usize) -> Self {
        Self { data, sector_size }
    }

    /// Consumes the cursor and returns its underlying data source.
    pub fn into_inner(self) -> DATA {
        self.data
    }

    /// Performs the `seek_sector` operation.
    pub async fn seek_sector(&mut self, sector: LogicalSector) -> Result<u64> {
        self.seek(SeekFrom::Start(sector.0 as u64 * self.sector_size as u64))
            .await
            .map_err(Error::erase)
    }
}

impl<DATA: Write + Seek> IsoCursor<DATA> {
    /// Advance to the next sector boundary, zero-filling the gap.
    ///
    /// The gap is written rather than skipped with a seek: `Write + Seek`
    /// targets are not guaranteed to read unwritten regions back as zeros
    /// (reused buffers, block devices with stale data), and readers scan
    /// some padding (e.g. the zero record-length terminator at the end of
    /// a directory's sector span). Gaps are always smaller than one sector.
    pub async fn pad_align_sector(&mut self) -> Result<LogicalSector> {
        const ZEROES: [u8; 512] = [0u8; 512];
        let stream_pos = self.stream_position().await.map_err(Error::erase)?;
        let sector_size_minus_one = self.sector_size as u64 - 1;
        let aligned_pos = (stream_pos + sector_size_minus_one) & !sector_size_minus_one;
        let mut remaining = (aligned_pos - stream_pos) as usize;
        while remaining > 0 {
            let n = remaining.min(ZEROES.len());
            self.write_all(&ZEROES[..n]).await.map_err(Error::erase)?;
            remaining -= n;
        }
        Ok(LogicalSector(
            (aligned_pos / self.sector_size as u64) as usize,
        ))
    }
}

impl<DATA: Write + Seek> Write for IsoCursor<DATA> {
    type Error = <DATA as Write>::Error;

    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.data.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.data.flush().await
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.data.write_all(buf).await
    }
}

} // io_transform!

impl<DATA: Seek> fmt::Debug for IsoCursor<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cursor").finish()
    }
}
