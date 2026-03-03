use super::directory::DirectoryRef;
use super::io::{self, IsoCursor, LogicalSector, Read, Seek};
use super::path::{PathTableInfo, PathTableRef};
use super::volume::VolumeDescriptorList;
use super::volume::{PrimaryVolumeDescriptor, VolumeDescriptor};
use crate::file::EntryType;
use crate::joliet::JolietLevel;
use hadris_common::types::endian::Endian;
#[cfg(not(feature = "alloc"))]
use hadris_common::types::no_alloc::ArrayVec;
sync_only! {
    use volume::VolumeDescriptorIter;
}

mod boot;
mod directory;
pub use boot::*;
pub use directory::{DirEntry, IsoDir};
sync_only! {
    pub use directory::{IsoDirIter, RawDirIter};
}
use spin::Mutex;

mod rrip;
pub(crate) use rrip::SuspInfo;
pub use rrip::*;

mod volume;

pub enum FilenameType {
    Builtin,
    Joliet,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct IsoImageInfo {
    block_size: usize,
    sector_size: usize,
    root_dirs: RootDirs,
    boot_catalog: Option<u32>,
    path_table: PathTableRef,
    pub(crate) susp_info: SuspInfo,
}

#[derive(Debug)]
pub struct RootDirs {
    #[cfg(not(feature = "alloc"))]
    dirs: ArrayVec<RootDir, 8>,
    #[cfg(feature = "alloc")]
    dirs: alloc::vec::Vec<RootDir>,
}

impl RootDirs {
    fn new() -> Self {
        Self {
            #[cfg(not(feature = "alloc"))]
            dirs: ArrayVec::new(),
            #[cfg(feature = "alloc")]
            dirs: alloc::vec::Vec::new(),
        }
    }

    pub fn best_choice(&self) -> RootDir {
        assert!(
            !self.dirs.is_empty(),
            "ISO image contains no directory trees!"
        );
        let mut best = (0, EntryType::default());
        for (idx, dir) in self.dirs.iter().enumerate() {
            if dir.ty > best.1 {
                best = (idx, dir.ty);
            }
        }
        self.dirs[best.0]
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RootDir {
    ty: EntryType,
    dir_ref: DirectoryRef,
}

impl RootDir {
    pub fn iter<'a, DATA: Read + Seek>(&self, iso: &'a IsoImage<DATA>) -> IsoDir<'a, DATA> {
        IsoDir {
            image: iso,
            directory: self.dir_ref,
        }
    }

    /// Returns the underlying `DirectoryRef` for this root directory.
    pub fn dir_ref(&self) -> DirectoryRef {
        self.dir_ref
    }
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

/// A struct representing an open ISO9660 Image
///
/// This struct is interior mutable.
#[derive(Debug)]
pub struct IsoImage<DATA: Seek> {
    pub(crate) data: Mutex<IsoCursor<DATA>>,
    pub(crate) info: IsoImageInfo,
}

io_transform! {
impl<DATA: Read + Seek> IsoImage<DATA> {
    /// Opens a ISO9660 Image
    pub async fn open(data: DATA) -> io::Result<Self> {
        let sector_size = 2048;
        let mut data = IsoCursor::new(data, sector_size);
        data.seek_sector(LogicalSector(16)).await?;
        let mut root_dirs = RootDirs::new();
        let volume_descriptors = VolumeDescriptorList::parse(&mut data).await?;
        let pvd = volume_descriptors.primary();
        let block_size = pvd.logical_block_size.read() as usize;
        let root_extent = LogicalSector(pvd.dir_record.header.extent.read() as usize);
        let root_size = pvd.dir_record.header.data_len.read() as usize;
        let root_dir = DirectoryRef {
            extent: root_extent,
            size: root_size,
        };

        // Detect SUSP/RRIP from root directory's "." entry
        let susp_info = rrip::detect_susp_rrip(&mut data, root_extent).await?;
        let supports_rrip = susp_info.rrip_detected;

        root_dirs.dirs.push(RootDir {
            ty: EntryType::Level1 {
                supports_lowercase: false,
                supports_rrip,
            },
            dir_ref: root_dir,
        });

        let path_table = PathTableRef {
            lpt: LogicalSector(pvd.type_l_path_table.get() as usize),
            mpt: LogicalSector(pvd.type_m_path_table.get() as usize),
            size: pvd.path_table_size.read() as u64,
        };

        let mut info = IsoImageInfo {
            block_size,
            sector_size,
            root_dirs,
            boot_catalog: None,
            path_table,
            susp_info,
        };

        for svd in volume_descriptors.supplementary() {
            if svd.header.version == 1 {
                // Joliet Check
                for &level in JolietLevel::all() {
                    if svd.escape_sequences == level.escape_sequence() {
                        info.root_dirs.dirs.push(RootDir {
                            ty: EntryType::Joliet {
                                level,
                                supports_rrip: false,
                            },
                            dir_ref: DirectoryRef {
                                extent: LogicalSector(svd.dir_record.header.extent.read() as usize),
                                size: svd.dir_record.header.data_len.read() as usize,
                            },
                        });
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
            data: Mutex::new(data),
            info,
        })
    }

    /// Read raw bytes from an absolute byte position in the image.
    pub async fn read_bytes_at(&self, byte_offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut data = self.data.lock();
        data.seek(super::io::SeekFrom::Start(byte_offset)).await?;
        data.read_exact(buf).await?;
        Ok(())
    }
}
} // io_transform!

impl<DATA: Read + Seek> IsoImage<DATA> {
    pub fn root_dir(&self) -> RootDir {
        self.root_dirs().best_choice()
    }

    pub fn root_dirs(&self) -> &RootDirs {
        &self.info.root_dirs
    }

    /// Returns whether RRIP (Rock Ridge) extensions were detected in the image.
    pub fn supports_rrip(&self) -> bool {
        self.info.susp_info.rrip_detected
    }

    /// Open a directory by its `DirectoryRef`, enabling navigation into subdirectories.
    pub fn open_dir(&self, dir_ref: DirectoryRef) -> IsoDir<'_, DATA> {
        IsoDir {
            image: self,
            directory: dir_ref,
        }
    }

    /// Returns the path table information for this image.
    pub fn path_table(&self) -> PathTableInfo {
        PathTableInfo {
            path_table: self.info.path_table,
        }
    }
}

sync_only! {
impl<DATA: Read + Seek> IsoImage<DATA> {
    pub fn read_volume_descriptors(&self) -> VolumeDescriptorIter<'_, DATA> {
        VolumeDescriptorIter {
            data: &self.data,
            current_sector: LogicalSector(16),
            done: false,
        }
    }

    pub fn read_pvd(&self) -> PrimaryVolumeDescriptor {
        self.read_volume_descriptors()
            .find_map(|vd| {
                if let Ok(VolumeDescriptor::Primary(vd)) = vd {
                    Some(vd)
                } else {
                    None
                }
            })
            .expect("could not find PVD!")
    }
}
} // sync_only!
