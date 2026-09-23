io_transform! {

// Re-export I/O traits from the parent sync/async module.
// Other I/O files use `super::io::Read` etc. which resolves through these re-exports.
pub use super::super::{Read, Write, Seek, ReadExt, Error, ErrorKind, SeekFrom, Parsable, Writable};
pub use super::super::IoResult;

/// Create an I/O error from an ErrorKind.
pub fn error_from_kind(kind: ErrorKind) -> Error {
    Error::from_kind(kind)
}

/// A Type Representing a FAT Sector
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Sector<T = usize>(pub T);

/// Converts a typed sector number into a byte offset.
pub trait SectorLike {
    /// Converts this sector number using `bytes_per_sector`.
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
    /// Combines the high and low words stored in a directory entry.
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

/// Seekable data source with FAT sector and cluster geometry.
pub struct SectorCursor<DATA: Seek> {
    pub(crate) data: DATA,
    pub(crate) sector_size: usize,
    pub(crate) cluster_size: usize,
}

impl<DATA: Seek> SectorCursor<DATA> {
    /// Creates a cursor with the supplied sector and cluster sizes.
    pub const fn new(data: DATA, sector_size: usize, cluster_size: usize) -> Self {
        Self {
            data,
            sector_size,
            cluster_size,
        }
    }

    /// Seeks to the beginning of a sector.
    pub async fn seek_sector(&mut self, sector: impl SectorLike) -> hadris_io::legacy::Result<u64> {
        self.seek(SeekFrom::Start(sector.to_bytes(self.sector_size) as u64))
            .await

    }
}

impl<T: Seek> Seek for SectorCursor<T> {
    async fn seek(&mut self, pos: hadris_io::SeekFrom) -> hadris_io::legacy::Result<u64> {
        self.data.seek(pos).await
    }
}

impl<T: Read + Seek> Read for SectorCursor<T> {
    async fn read(&mut self, buf: &mut [u8]) -> hadris_io::legacy::Result<usize> {
        self.data.read(buf).await
    }
}

#[cfg(feature = "write")]
impl<T: Write + Seek> Write for SectorCursor<T> {
    async fn write(&mut self, buf: &[u8]) -> hadris_io::legacy::Result<usize> {
        self.data.write(buf).await
    }

    async fn flush(&mut self) -> hadris_io::legacy::Result<()> {
        self.data.flush().await
    }
}

} // end io_transform!
