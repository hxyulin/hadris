pub use hadris_io::{Result, Error, Read, Seek, SeekFrom, Write, ReadExt};

/// A Type Representing a FAT Sector
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Sector<T = usize>(pub T);

pub trait SectorLike {
    fn to_bytes(self, bytes_per_sector: usize) -> usize;
}

macro_rules! sector_impl {
    ($ty:ty) => {
        impl SectorLike for Sector<$ty> {
            fn to_bytes(self, bytes_per_sector: usize) -> usize {
                (self.0 as usize) * bytes_per_sector
            }
        }
    };
}
sector_impl!(u8);
sector_impl!(u16);
sector_impl!(u32);
sector_impl!(u64);
sector_impl!(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Cluster<T = usize>(pub T);

pub(crate) trait ClusterLike {
    fn to_bytes(self, data_start: usize, bytes_per_cluster: usize) -> usize;
}

macro_rules! cluster_impl {
    ($ty:ty) => {
        impl ClusterLike for Cluster<$ty> {
            fn to_bytes(self, data_start: usize, bytes_per_cluster: usize) -> usize {
                data_start + (self.0 as usize - 2) * bytes_per_cluster
            }
        }
    };
}
cluster_impl!(u8);
cluster_impl!(u16);
cluster_impl!(u32);
cluster_impl!(u64);
cluster_impl!(usize);

pub struct SectorCursor<DATA: Seek> {
    pub(crate) data: DATA,
    pub(crate) sector_size: usize,
}

impl<DATA: Seek> SectorCursor<DATA> {
    pub const fn new(data: DATA, sector_size: usize) -> Self {
        Self { data, sector_size }
    }

    pub fn seek_sector(&mut self, sector: impl SectorLike) -> Result<u64> {
        self.seek(SeekFrom::Start(sector.to_bytes(self.sector_size) as u64))
    }
}

impl<T> Seek for SectorCursor<T>
where
    T: Seek,
{
    fn seek(&mut self, pos: hadris_io::SeekFrom) -> hadris_io::Result<u64> {
        self.data.seek(pos)
    }

    fn rewind(&mut self) -> hadris_io::Result<()> {
        self.data.rewind()
    }

    fn seek_relative(&mut self, offset: i64) -> hadris_io::Result<()> {
        self.data.seek_relative(offset)
    }

    fn stream_position(&mut self) -> hadris_io::Result<u64> {
        self.data.stream_position()
    }
}

impl<T> Read for SectorCursor<T>
where
    T: Read + Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> hadris_io::Result<usize> {
        self.data.read(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> hadris_io::Result<()> {
        self.data.read_exact(buf)
    }
}

