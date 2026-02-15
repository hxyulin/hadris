pub use hadris_io::{Error, ErrorKind, Read, ReadExt, Result, Seek, SeekFrom, Write};

/// Create an I/O error from an ErrorKind.
///
/// This helper works in both std and no-std modes.
#[cfg(feature = "std")]
pub fn error_from_kind(kind: ErrorKind) -> Error {
    Error::new(kind, "")
}

#[cfg(not(feature = "std"))]
pub fn error_from_kind(kind: ErrorKind) -> Error {
    Error::from_kind(kind)
}

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

/// Represents a cluster number in a FAT filesystem.
/// Clusters are the allocation units for file data, starting at cluster 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cluster<T = usize>(pub T);

impl Cluster<usize> {
    pub fn from_parts(high: u16, low: u16) -> Self {
        Self((high as usize) << 16 | (low as usize))
    }
}

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
    pub(crate) cluster_size: usize,
}

impl<DATA: Seek> SectorCursor<DATA> {
    pub const fn new(data: DATA, sector_size: usize, cluster_size: usize) -> Self {
        Self {
            data,
            sector_size,
            cluster_size,
        }
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

    fn stream_position(&mut self) -> hadris_io::Result<u64> {
        self.data.stream_position()
    }

    fn seek_relative(&mut self, offset: i64) -> hadris_io::Result<()> {
        self.data.seek_relative(offset)
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

#[cfg(feature = "write")]
impl<T> Write for SectorCursor<T>
where
    T: Write + Seek,
{
    fn write(&mut self, buf: &[u8]) -> hadris_io::Result<usize> {
        self.data.write(buf)
    }

    fn flush(&mut self) -> hadris_io::Result<()> {
        self.data.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> hadris_io::Result<()> {
        self.data.write_all(buf)
    }
}
