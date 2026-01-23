use core::{fmt, ops::{Add, AddAssign}};

pub use hadris_io::{Error, Parsable, Read, ReadExt, Result, Seek, SeekFrom, Writable, Write, try_io_result_option};

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

pub struct IsoCursor<DATA: Seek> {
    pub data: DATA,
    pub sector_size: usize,
}

impl<DATA: Read + Seek> Read for IsoCursor<DATA> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.data.read(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.data.read_exact(buf)
    }
}

impl<DATA: Seek> Seek for IsoCursor<DATA> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.data.seek(pos)
    }

    fn stream_position(&mut self) -> Result<u64> {
        self.data.stream_position()
    }

    fn seek_relative(&mut self, offset: i64) -> Result<()> {
        self.data.seek_relative(offset)
    }
}

impl<DATA: Seek> IsoCursor<DATA> {
    pub fn new(data: DATA, sector_size: usize) -> Self {
        Self { data, sector_size }
    }

    pub fn pad_align_sector(&mut self) -> Result<LogicalSector> {
        let stream_pos = self.stream_position()?;
        let sector_size_minus_one = self.sector_size as u64 - 1;
        let aligned_pos = (stream_pos + sector_size_minus_one) & !sector_size_minus_one;
        if aligned_pos != stream_pos {
            self.seek(SeekFrom::Start(aligned_pos))?;
        }
        Ok(LogicalSector(
            (aligned_pos / self.sector_size as u64) as usize,
        ))
    }

    pub fn seek_sector(&mut self, sector: LogicalSector) -> Result<u64> {
        self.seek(SeekFrom::Start(sector.0 as u64 * self.sector_size as u64))
    }
}

impl<DATA: Write + Seek> Write for IsoCursor<DATA> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.data.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.data.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.data.write_all(buf)
    }
}

impl<DATA: Seek> fmt::Debug for IsoCursor<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cursor").finish()
    }
}
