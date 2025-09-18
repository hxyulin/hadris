use core::ops::DerefMut;

use crate::{
    LockedCursor, LogicalSector,
    boot::{BaseBootCatalog, BootCatalog},
    directory::{DirectoryRecord, DirectoryRef},
    path::{PathTableInfo, PathTableRef},
    volume::VolumeDescriptorList,
};
use hadris_common::types::endian::Endian;
use hadris_io::{self as io, Read, Seek, SeekFrom};
use spin::Mutex;

mod boot;
mod directory;
pub use boot::*;
pub use directory::{IsoDir, IsoDirIter};

pub enum FilenameType {
    Builtin,
    Joliet,
}

#[derive(Debug)]
pub struct IsoImageInfo {
    block_size: usize,
    sector_size: usize,
    root_dirs: RootDirs,
    boot_catalog: Option<u32>,
    path_table: PathTableRef,
}

#[derive(Debug)]
struct RootDirs {
    builtin: DirectoryRef,
    joliet: Option<DirectoryRef>,
}

/// A struct representing an ISO9660 Image
#[derive(Debug)]
pub struct IsoImage<DATA: Seek> {
    pub(crate) data: LockedCursor<DATA>,
    pub(crate) info: IsoImageInfo,
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
            
            let path_table = PathTableRef {
                lpt: LogicalSector(pvd.type_l_path_table.get() as usize),
                mpt: LogicalSector(pvd.type_m_path_table.get() as usize),
                size: pvd.path_table_size.read() as u64,
            };

            IsoImageInfo {
                block_size,
                sector_size: sector_size as usize,
                root_dirs: RootDirs {
                    builtin: root_dir,
                    joliet: None,
                },
                boot_catalog: None,
                path_table,
            }
        };
        for svd in volume_descriptors.supplementary() {
            if svd.header.version == 1 {
                // Joliet Check
                let seq = &svd.escape_sequences[0..3];
                for escape_seq in crate::joliet::ESCAPE_SEQUNCES {
                    if seq == escape_seq {
                        info.root_dirs.joliet = Some(DirectoryRef {
                            extent: LogicalSector(svd.dir_record.header.extent.read() as usize),
                            size: svd.dir_record.header.data_len.read() as usize,
                        })
                    }
                }
            } else if svd.file_structure_version == 2 {
                // TODO: EVD
                continue;
            }
        }

        if let Some(boot_record) = volume_descriptors.boot_record() {
            info.boot_catalog = Some(boot_record.catalog_ptr.get());
        }

        Ok(Self {
            data: LockedCursor {
                data: Mutex::new(data),
                sector_size: sector_size as usize,
            },
            info,
        })
    }

    pub fn boot_info(&self) -> io::Result<Option<BootInfo>> {
        let catalog_ptr = match self.info.boot_catalog {
            None => return Ok(None),
            Some(ptr) => LogicalSector(ptr as usize),
        };
        self.data.seek_sector(catalog_ptr)?;
        let catalog = {
            let mut data = self.data.lock();
            BaseBootCatalog::parse(data.deref_mut())?
        };

        Ok(Some(BootInfo {
            catalog,
            catalog_ptr,
        }))
    }

    pub fn path_table(&self) -> PathTableInfo {
        PathTableInfo {
            path_table: self.info.path_table,
        }
    }

    pub fn root_dir(&self) -> IsoDir<'_, DATA> {
        self.read_dir(self.info.root_dirs.builtin)
    }

    pub fn root_dir_for(&self, filename_ty: FilenameType) -> Option<IsoDir<'_, DATA>> {
        match filename_ty {
            FilenameType::Builtin => Some(self.root_dir()),
            FilenameType::Joliet => self.info.root_dirs.joliet.map(|root| self.read_dir(root)),
        }
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
        let start = file.header().extent.read() as usize * self.info.sector_size;
        let end = file.header().data_len.read() as usize + start;
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
