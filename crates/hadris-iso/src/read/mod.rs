use super::directory::DirectoryRef;
use super::io::{self, IsoCursor, LogicalSector, Read, Seek};
use super::path::{PathTableEntry, PathTableInfo, PathTableRef};
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
pub use directory::{DirEntry, Extent, IsoDir};
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
    /// True if an Enhanced Volume Descriptor (EVD) was found, indicating
    /// this may be a UDF bridge ISO (combined ISO 9660 + UDF).
    has_evd: bool,
    /// Cached path table entries, parsed once during open() to avoid
    /// repeated seeks for directory hierarchy traversal.
    #[cfg(feature = "alloc")]
    path_table_cache: alloc::vec::Vec<PathTableEntry>,
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

        // Parse and cache path table entries
        #[cfg(feature = "alloc")]
        let path_table_cache = {
            use crate::types::EndianType;
            let pt_start = if cfg!(target_endian = "little") {
                path_table.lpt
            } else {
                path_table.mpt
            };
            let start_byte = pt_start.0 as u64 * sector_size as u64;
            let end_byte = start_byte + path_table.size;
            data.seek(super::io::SeekFrom::Start(start_byte)).await?;
            let mut entries = alloc::vec::Vec::new();
            let mut pos = start_byte;
            while pos < end_byte {
                let entry = PathTableEntry::parse(&mut data, EndianType::NativeEndian).await?;
                pos += entry.size() as u64;
                entries.push(entry);
            }
            entries
        };

        let mut info = IsoImageInfo {
            block_size,
            sector_size,
            root_dirs,
            boot_catalog: None,
            path_table,
            susp_info,
            has_evd: false,
            #[cfg(feature = "alloc")]
            path_table_cache,
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
                // Enhanced Volume Descriptor (EVD) — indicates UDF bridge ISO.
                // The ISO 9660 spec defines this as a supplementary VD with
                // file_structure_version == 2. Its presence signals that the
                // image likely also contains a UDF filesystem that can be
                // accessed via hadris-udf for richer metadata.
                info.has_evd = true;
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

    /// Read the complete contents of a file, handling multi-extent files.
    ///
    /// For single-extent files, this reads from the entry's extent.
    /// For multi-extent files (using `NOT_FINAL` flag), this reads and
    /// concatenates all extents in order.
    #[cfg(feature = "alloc")]
    pub async fn read_file(&self, entry: &directory::DirEntry) -> io::Result<alloc::vec::Vec<u8>> {
        let total = entry.total_size();
        let mut buf = alloc::vec![0u8; total as usize];

        if entry.is_multi_extent() {
            let mut offset = 0usize;
            for extent in entry.extents() {
                let len = extent.length as usize;
                let byte_offset = extent.sector.0 as u64 * 2048;
                self.read_bytes_at(byte_offset, &mut buf[offset..offset + len]).await?;
                offset += len;
            }
        } else {
            let header = entry.header();
            let byte_offset = header.extent.read() as u64 * 2048;
            let len = header.data_len.read() as usize;
            self.read_bytes_at(byte_offset, &mut buf[..len]).await?;
        }

        Ok(buf)
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

    /// Returns whether an Enhanced Volume Descriptor (EVD) was found.
    ///
    /// An EVD indicates this is likely a UDF bridge ISO containing both
    /// ISO 9660 and UDF filesystems. Use `hadris-udf` for UDF access.
    pub fn has_evd(&self) -> bool {
        self.info.has_evd
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

    /// Returns the cached path table entries, parsed once during `open()`.
    ///
    /// Each entry contains a directory name, its LBA, and its parent index,
    /// enabling fast directory hierarchy traversal without repeated seeks.
    #[cfg(feature = "alloc")]
    pub fn path_table_entries(&self) -> &[PathTableEntry] {
        &self.info.path_table_cache
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
