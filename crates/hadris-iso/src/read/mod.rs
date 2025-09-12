use crate::{
    LockedCursor, LogicalSector,
    directory::{DirectoryRecord, DirectoryRef},
    volume::VolumeDescriptorList,
};
use hadris_io::{self as io, Read, Seek, SeekFrom};
use spin::Mutex;

mod directory;
pub use directory::{IsoDir, IsoDirIter};

#[derive(Debug)]
pub struct IsoImageInfo {
    block_size: usize,
    sector_size: usize,
    root_dir: DirectoryRef,
}

/// A struct representing an ISO9660 Image
#[derive(Debug)]
pub struct IsoImage<DATA: Seek> {
    data: LockedCursor<DATA>,
    info: IsoImageInfo,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SupportedFeatures: u64 {

    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSeparator {
    ForwardSlash = b'/',
    Backslash = b'\\',
}

impl PathSeparator {
    pub fn as_char(self) -> char {
        self as u8 as char
    }
}

impl<DATA: Read + Seek> IsoImage<DATA> {
    pub fn parse(mut data: DATA) -> io::Result<Self> {
        let sector_size = 2048;
        let pvd_start = 16 * sector_size;
        data.seek(SeekFrom::Start(pvd_start))?;
        let volume_descriptors = VolumeDescriptorList::parse(&mut data)?;
        let mut info = {
            let pvd = volume_descriptors.primary();
            let block_size = pvd.logical_block_size.read() as usize;
            let root_dir = DirectoryRef {
                extent: LogicalSector(pvd.dir_record.header.extent.read() as usize),
                size: pvd.dir_record.header.data_len.read() as usize,
            };

            IsoImageInfo {
                block_size,
                sector_size: sector_size as usize,
                root_dir,
            }
        };
        for svd in volume_descriptors.supplementary() {
            /*
            info.root_dir = DirectoryRef {
                extent: LogicalSector(svd.dir_record.header.extent.read() as usize),
                size: svd.dir_record.header.data_len.read() as usize,
            };
            // UNIMPLEMENTED
            */
        }

        Ok(Self {
            data: LockedCursor {
                data: Mutex::new(data),
                sector_size: sector_size as usize,
            },
            info,
        })
    }

    pub fn root_dir(&self) -> IsoDir<'_, DATA> {
        self.read_dir(self.info.root_dir)
    }

    pub fn read_dir(&self, directory: DirectoryRef) -> IsoDir<'_, DATA> {
        IsoDir {
            reader: &self.data.data,
            directory,
        }
    }

    pub fn read_file(&self, file: DirectoryRecord) -> FileReader<'_, DATA> {
        // TODO: Support interleaved files
        assert!(file.is_file(), "tried to read non-file!");
        let start = file.header.extent.read() as usize * self.info.sector_size;
        let end = file.header.data_len.read() as usize + start;
        FileReader {
            data: &self.data.data,
            current: start,
            end,
        }
    }
}
pub struct FileReader<'a, T: Read + Seek> {
    data: &'a Mutex<T>,
    current: usize,
    end: usize,
}

impl<T: Read + Seek> Read for FileReader<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut reader = self.data.lock();
        let read_max = (self.end - self.current).min(buf.len());
        reader.seek(SeekFrom::Start(self.current as u64))?;
        let read_bytes = if read_max > buf.len() {
            reader.read(buf)?
        } else {
            reader.read(&mut buf[0..read_max])?
        };
        self.current += read_bytes;
        Ok(read_bytes)
    }
}
