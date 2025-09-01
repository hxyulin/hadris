//! Hadris ISO
//! Terminology and spec are followed by the specifications described in
//! the [non official ISO9660 specification included](https://github.com/hxyulin/hadris/tree/main/crates/hadris-iso/spec)

extern crate alloc;

pub mod directory;
pub mod types;
pub mod volume;

use core::fmt;

use hadris_io::{self as io, Read, Seek, SeekFrom, Write};
use spin::Mutex;

use crate::{
    directory::{DirectoryRecord, DirectoryRef, IsoDir},
    volume::VolumeDescriptorList,
};

/// A Logical Sector, size has to be 2^n and > 2048
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LogicalSector(usize);

/// A Logical Sector, size has to be 2^n and > 512
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LogicalBlock(usize);

struct IsoCursor<DATA: Seek> {
    data: Mutex<DATA>,
}

impl<DATA: Seek> fmt::Debug for IsoCursor<DATA> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IsoCursor").finish()
    }
}

#[derive(Debug)]
pub struct IsoImageInfo {
    block_size: usize,
    sector_size: usize,
    root_dir: DirectoryRef,
}

/// A struct representing an ISO9660 Image
#[derive(Debug)]
pub struct IsoImage<DATA: Seek> {
    data: IsoCursor<DATA>,
    info: IsoImageInfo,
}

impl<DATA: Read + Seek> IsoImage<DATA> {
    pub fn parse(mut data: DATA) -> io::Result<Self> {
        let sector_size = 2048;
        let pvd_start = 16 * sector_size;
        data.seek(SeekFrom::Start(pvd_start))?;
        let volume_descriptors = VolumeDescriptorList::parse(&mut data)?;
        let info = {
            let pvd = volume_descriptors.primary();
            let block_size = pvd.logical_block_size.read() as usize;
            let root_dir = DirectoryRef {
                offset: pvd.dir_record.header.extent.read() as usize,
                size: pvd.dir_record.header.data_len.read() as usize,
            };

            IsoImageInfo {
                block_size,
                sector_size: sector_size as usize,
                root_dir,
            }
        };

        Ok(Self {
            data: IsoCursor {
                data: Mutex::new(data),
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
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
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
