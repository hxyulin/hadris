use super::directory::DirectoryRef;
use super::io::{self, IsoCursor, LogicalSector, Read, Seek};
use super::path::{PathTableEntry, PathTableInfo, PathTableRef};
use super::volume::{PrimaryVolumeDescriptor, VolumeDescriptorList};
use crate::file::EntryType;
use crate::joliet::JolietLevel;
use hadris_common::types::endian::Endian;
#[cfg(not(feature = "alloc"))]
use hadris_common::types::no_alloc::ArrayVec;
use hadris_path::{Component, Separators, VPath};
pub use volume::VolumeDescriptorIter;

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

/// Identifies a FilenameType value.
pub enum FilenameType {
    /// The `Builtin` variant.
    Builtin,
    /// The `Joliet` variant.
    Joliet,
}

#[derive(Debug)]
#[allow(dead_code)]
/// Represents IsoImageInfo.
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
/// Represents RootDirs.
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

    /// Returns the number of directory-tree namespaces in the image.
    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    /// Returns whether the image contains no directory-tree namespaces.
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// Iterates over every directory-tree namespace in descriptor order.
    pub fn iter(&self) -> core::slice::Iter<'_, RootDir> {
        self.dirs.iter()
    }

    /// Finds the root whose entry type exactly matches `ty`.
    pub fn get(&self, ty: EntryType) -> Option<RootDir> {
        self.dirs.iter().copied().find(|root| root.ty == ty)
    }

    /// Selects the most useful directory-tree namespace, if one exists.
    pub fn try_best_choice(&self) -> Option<RootDir> {
        if self.dirs.is_empty() {
            return None;
        }
        let mut best = (0, EntryType::default());
        for (idx, dir) in self.dirs.iter().enumerate() {
            if dir.ty > best.1 {
                best = (idx, dir.ty);
            }
        }
        Some(self.dirs[best.0])
    }

    /// Selects the most useful directory-tree namespace.
    ///
    /// # Panics
    ///
    /// Panics if the ISO image contains no directory trees. Use
    /// [`Self::try_best_choice`] when handling a potentially empty collection.
    pub fn best_choice(&self) -> RootDir {
        self.try_best_choice()
            .expect("ISO image contains no directory trees!")
    }
}

impl<'a> IntoIterator for &'a RootDirs {
    type Item = &'a RootDir;
    type IntoIter = core::slice::Iter<'a, RootDir>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Default, Clone, Copy)]
/// Represents RootDir.
pub struct RootDir {
    ty: EntryType,
    dir_ref: DirectoryRef,
}

impl RootDir {
    /// Returns the filename namespace represented by this root.
    pub fn entry_type(&self) -> EntryType {
        self.ty
    }

    /// Performs the `iter` operation.
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
    /// Extension features supported by an ISO image reader.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SupportedFeatures: u64 {

    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Identifies a PathSeparator value.
pub enum PathSeparator {
    /// The `ForwardSlash` variant.
    ForwardSlash = b'/',
    /// The `Backslash` variant.
    Backslash = b'\\',
}

impl PathSeparator {
    /// Performs the `as_char` operation.
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

impl<DATA: Seek> IsoImage<DATA> {
    /// Consumes the image handle and returns its underlying data source.
    pub fn into_inner(self) -> DATA {
        self.data.into_inner().data
    }
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
            data.seek(super::io::SeekFrom::Start(start_byte))
                .await
                .map_err(super::io::Error::erase)?;
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
        data.seek(super::io::SeekFrom::Start(byte_offset))
            .await
            .map_err(super::io::Error::erase)?;
        data.read_exact(buf).await?;
        Ok(())
    }

    /// Finds an entry by a slash- or backslash-delimited path.
    ///
    /// Leading and repeated separators and `.` components are ignored. The
    /// root itself has no directory entry, so an empty or root-only path
    /// returns `None`. Parent (`..`) components are rejected.
    pub async fn find_path(&self, path: &str) -> io::Result<Option<DirEntry>> {
        let mut components = VPath::with_separators(path, Separators::SlashOrBackslash)
            .components()
            .filter_map(|component| match component {
                Component::Root | Component::Current => None,
                Component::Parent => Some(Err(io::Error::other(
                    "parent path components are not supported",
                ))),
                Component::Normal(component) => Some(Ok(component)),
            })
            .peekable();
        let mut directory = self.open_dir(self.root_dir().dir_ref());

        while let Some(component) = components.next() {
            let component = component?;
            let Some(entry) = directory.find(component).await? else {
                return Ok(None);
            };
            if components.peek().is_none() {
                return Ok(Some(entry));
            }
            if !entry.is_directory() {
                return Ok(None);
            }
            directory = self.open_dir(entry.as_dir_ref(self).await?);
        }
        Ok(None)
    }

    /// Read the complete contents of a file, handling multi-extent files.
    ///
    /// For single-extent files, this reads from the entry's extent.
    /// For multi-extent files (using `NOT_FINAL` flag), this reads and
    /// concatenates all extents in order.
    #[cfg(feature = "alloc")]
    pub async fn read_file(&self, entry: &directory::DirEntry) -> io::Result<alloc::vec::Vec<u8>> {
        let total = entry.total_size();
        // `total` comes from on-disk directory-record data-length fields (u32 each,
        // summed across extents) and is untrusted. Bound it against the actual
        // image size before allocating, otherwise a tiny image whose record claims
        // ~4 GiB would force that allocation up front — a DoS that aborts the
        // process on no-overcommit / embedded targets.
        let image_len = {
            let mut data = self.data.lock();
            data.seek(super::io::SeekFrom::End(0))
                .await
                .map_err(super::io::Error::erase)?
        };
        if total > image_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry claims more data than the image contains",
            ));
        }
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
    /// Performs the `root_dir` operation.
    pub fn root_dir(&self) -> RootDir {
        self.root_dirs().best_choice()
    }

    /// Performs the `root_dirs` operation.
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

    /// Creates a volume-descriptor cursor starting at logical sector 16.
    ///
    /// Async callers use [`VolumeDescriptorIter::next_descriptor`]. In sync
    /// builds the cursor also implements [`Iterator`].
    pub fn read_volume_descriptors(&self) -> VolumeDescriptorIter<'_, DATA> {
        VolumeDescriptorIter {
            data: &self.data,
            current_sector: LogicalSector(16),
            done: false,
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

io_transform! {
impl<DATA: Read + Seek> IsoImage<DATA> {
    /// Reads the primary volume descriptor.
    ///
    /// Returns an I/O error if the descriptor sequence is malformed, truncated,
    /// or contains no primary descriptor.
    pub async fn read_pvd(&self) -> io::Result<PrimaryVolumeDescriptor> {
        let mut descriptors = self.read_volume_descriptors();
        while let Some(descriptor) = descriptors.next_descriptor().await? {
            if let super::volume::VolumeDescriptor::Primary(pvd) = descriptor {
                return Ok(pvd);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "primary volume descriptor not found",
        ))
    }
}
} // io_transform!
