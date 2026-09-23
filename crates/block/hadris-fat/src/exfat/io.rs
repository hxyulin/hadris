//! The legacy synchronous I/O the exFAT preview runs on.

pub(crate) use hadris_io::legacy::sync::{Read, ReadExt, Seek, Write};
pub(crate) use hadris_io::legacy::{Error, ErrorKind, SeekFrom};

pub(crate) type IoResult<T> = hadris_io::legacy::Result<T>;

pub(crate) fn error_from_kind(kind: ErrorKind) -> Error {
    Error::from_kind(kind)
}

/// A seekable volume with its sector and cluster sizes.
pub(crate) struct SectorCursor<DATA> {
    pub(crate) data: DATA,
    #[allow(dead_code)]
    pub(crate) sector_size: usize,
    #[allow(dead_code)]
    pub(crate) cluster_size: usize,
}

impl<DATA> SectorCursor<DATA> {
    pub(crate) const fn new(data: DATA, sector_size: usize, cluster_size: usize) -> Self {
        Self {
            data,
            sector_size,
            cluster_size,
        }
    }
}

impl<T: Seek> Seek for SectorCursor<T> {
    fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
        self.data.seek(pos)
    }
}

impl<T: Read> Read for SectorCursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        self.data.read(buf)
    }
}

impl<T: Write> Write for SectorCursor<T> {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.data.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.data.flush()
    }
}
