//! A library for working with FAT32 file systems
//! Supports reading and writing to FAT32 file systems,
//! with no-std support
//!
//! When used with no features, the crate act as a place for providing the structures used in the
//! FAT32 file system.
//!
//! ## Cargo Features
//!
//! - **alloc**: Enables the 'alloc' feature, which allows for dynamic allocation of memory
//! - **std**: Enables the 'std' feature, which requires an 'std' environment
//! - **read**: Enables the 'read' feature, which allows for reading from FAT32 file systems
//! - **write**: Enables the 'write' feature, which allows for writing to FAT32 file systems
//! - **lfn**: Enables the 'lfn' feature, which allows for reading and writing long file names,
//! which is an optional extension to the FAT32 specification

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(feature = "alloc")]
extern crate alloc;

// TODO: Add support for big endian, because we currently just reinterpret the bytes as little endian

#[cfg(not(target_endian = "little"))]
compile_error!("This crate only supports little endian systems");

pub mod structures;

#[cfg(feature = "write")]
use structures::Fat32Ops;

use structures::{
    boot_sector::{BootSector, BootSectorConversionError, BootSectorInfo},
    directory::{Directory, FileAttributes, FileEntry},
    fat::Fat32,
    fs_info::FsInfo,
    raw::{self, directory::RawDirectoryEntry},
    FatStr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat32,
    Fat16,
    Fat12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileDescriptor {
    cluster: u32,
    entry_offset: usize,
}

/// A FAT filesystem
///
/// The data stored follows the following layout according to the FAT specification:
/// ```text
/// Reserved
/// FAT(s)
/// Data
/// ```
///
/// The reserved area should contain the boot sector and the backup boot sector, and FsInfo on FAT32.
/// The FAT(s) should contain the FAT table(s).
/// The data area should contain the data of the filesystem,
/// for FAT12 and FAT16, the first cluster is used as the root directory.
///
/// TODO: This currently only supports FAT32.
pub struct FileSystem<'ctx> {
    pub(crate) reserved: &'ctx mut [u8],
    pub(crate) fat: &'ctx mut [u8],
    pub(crate) data: &'ctx mut [u8],

    // Store some metadata about the filesystem
    pub(crate) bs: BootSectorInfo,
    // To make this no-std compliant, we just have a list of clusters
    pub(crate) descriptors: [Option<FileDescriptor>; MAX_OPEN],
}

impl core::fmt::Debug for FileSystem<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileSystem")
            .field("bs", &self.bs)
            .field("descriptors", &self.descriptors)
            .finish()
    }
}

const MAX_OPEN: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemError {
    InvalidBootSector(BootSectorConversionError),
    FileTooSmall,
}

impl<'ctx> FileSystem<'ctx> {
    pub fn read_from_bytes(bytes: &'ctx mut [u8]) -> Result<Self, FilesystemError> {
        if bytes.len() < 512 {
            return Err(FilesystemError::FileTooSmall);
        }
        let bs = {
            let bs = BootSectorInfo::try_from(raw::boot_sector::RawBootSector::from_bytes(
                &bytes[0..512].try_into().unwrap(),
            ))
            .map_err(|e| FilesystemError::InvalidBootSector(e))?;
            if bytes.len() < bs.bytes_per_sector() as usize * bs.total_sectors() as usize {
                return Err(FilesystemError::FileTooSmall);
            }
            bs.clone()
        };

        let (reserved, rest) = bytes
            .split_at_mut(bs.bytes_per_sector() as usize * bs.reserved_sector_count() as usize);
        let (fat, data) =
            rest.split_at_mut(bs.sectors_per_fat() as usize * bs.bytes_per_sector() as usize);

        Ok(Self {
            reserved,
            fat,
            data,

            bs,
            descriptors: [None; MAX_OPEN],
        })
    }

    fn create_file_descriptor(
        &mut self,
        path: &str,
        mode: hadris_core::OpenMode,
    ) -> Result<u32, ()> {
        // TODO: This is very hacky, we should probably use a better implementation
        let cluster_size =
            self.bs.bytes_per_sector() as usize * self.bs.sectors_per_cluster() as usize;
        let root_cluster = self.bs.root_cluster();
        let cluster_start = (root_cluster as usize - 2) * cluster_size;
        // root directory
        let index = {
            let directory =
                Directory::from_bytes(&self.data[cluster_start..cluster_start + cluster_size]);
            let path = path.split('.').collect::<Vec<_>>();
            directory.find_by_name(
                &FatStr::new_truncate(path[0]),
                &FatStr::new_truncate(path[1]),
            )
        };
        let (entry, offset) = match index {
            Some(index) => {
                let directory =
                    Directory::from_bytes(&self.data[cluster_start..cluster_start + cluster_size]);
                let offset = cluster_start + size_of::<RawDirectoryEntry>() * index;
                (directory.entries[index], offset)
            }
            None => {
                if mode != hadris_core::OpenMode::Write {
                    return Err(());
                }
                let cluster_free = self.allocate_clusters(1);
                let path = path.split('.').collect::<Vec<_>>();
                let file =
                    FileEntry::new(path[0], path[1], FileAttributes::ARCHIVE, 0, cluster_free);
                let directory = Directory::from_bytes_mut(
                    &mut self.data[cluster_start..cluster_start + cluster_size],
                );
                let index = directory.write_entry(file).unwrap();
                let offset = cluster_start + size_of::<RawDirectoryEntry>() * index;
                (directory.entries[index], offset)
            }
        };
        let descriptor = self
            .descriptors
            .iter_mut()
            .enumerate()
            .find(|(_, d)| d.is_none())
            .ok_or(())?;
        descriptor.1.replace(FileDescriptor {
            cluster: entry.cluster(),
            entry_offset: offset,
        });
        Ok(descriptor.0 as u32)
    }

    fn to_clusters_rounded_up(&self, size: usize) -> usize {
        let bytes_per_cluster =
            self.bs.bytes_per_sector() as usize * self.bs.sectors_per_cluster() as usize;
        (size + bytes_per_cluster - 1) / bytes_per_cluster
    }
}

impl hadris_core::FileSystem for FileSystem<'_> {
    fn open(&mut self, path: &str, mode: hadris_core::OpenMode) -> Result<hadris_core::File, ()> {
        let descriptor = self.create_file_descriptor(path, mode)?;
        Ok(unsafe { hadris_core::File::with_descriptor(descriptor) })
    }
}

impl hadris_core::internal::FileSystemRead for FileSystem<'_> {
    fn read(&self, file: &hadris_core::File, buffer: &mut [u8]) -> Result<usize, ()> {
        let cluster_size =
            self.bs.bytes_per_sector() as usize * self.bs.sectors_per_cluster() as usize;
        let fd = self.descriptors[file.descriptor() as usize].unwrap();
        Ok(Fat32::from_bytes(self.fat).read_data(
            self.data,
            cluster_size,
            fd.cluster,
            file.seek() as usize,
            buffer,
        ))
    }
}

impl hadris_core::internal::FileSystemWrite for FileSystem<'_> {
    fn write(&mut self, file: &hadris_core::File, buffer: &[u8]) -> Result<usize, ()> {
        // TODO: This is a super hacky implementation, we should probably use a better implementation
        // FIXME: This also doesn't even work
        let cluster_size =
            self.bs.bytes_per_sector() as usize * self.bs.sectors_per_cluster() as usize;
        let fd = self.descriptors[file.descriptor() as usize].unwrap();
        let written = Fat32::from_bytes(self.fat).write_data(
            self.data,
            cluster_size,
            fd.cluster,
            file.seek() as usize,
            buffer,
        );

        // Round down the entry offset to the start of the directory entry
        let cluster_start = (fd.entry_offset / cluster_size) * cluster_size;
        let directory =
            Directory::from_bytes_mut(&mut self.data[cluster_start..cluster_start + cluster_size]);
        let offset = (fd.entry_offset % cluster_size) / size_of::<RawDirectoryEntry>();
        let entry = &mut directory.entries[offset];
        entry.write_size(entry.size() + written as u32);

        Ok(written)
    }
}

#[cfg(feature = "write")]
impl<'ctx> FileSystem<'ctx> {
    pub fn new_f32(mut ops: Fat32Ops, data: &'ctx mut [u8]) -> Self {
        let bytes_per_sector = ops.bytes_per_sector as usize;
        let usable_sectors = ops.total_sectors_32 as usize - ops.reserved_sector_count as usize;
        let fat_size_sectors = usable_sectors / Fat32::entries_per_sector(bytes_per_sector) + 1;
        assert!(
            fat_size_sectors == ops.sectors_per_fat_32 as usize,
            "Specified fat size in sectors does not match provided {} vs {}",
            ops.sectors_per_cluster,
            fat_size_sectors
        );
        ops.sectors_per_fat_32 = fat_size_sectors as u32;

        let boot_sector = BootSector::create_fat32(
            ops.bytes_per_sector,
            ops.sectors_per_cluster,
            ops.reserved_sector_count,
            ops.fat_count,
            ops.media_type,
            ops.hidden_sector_count,
            ops.total_sectors_32,
            ops.sectors_per_fat_32,
            ops.root_cluster,
            ops.fs_info_sector,
            ops.boot_sector,
            ops.drive_number,
            ops.volume_id,
            ops.volume_label.as_ref().map(String::as_str),
        );

        // Used sectors include:
        // 1. Reserved sectors
        // 2. FAT sectors (and second FAT sectors if there are 2 fats)
        let used_sectors = ops.reserved_sector_count as u32 + fat_size_sectors as u32;
        let fat_start = ops.reserved_sector_count as usize * bytes_per_sector;
        let fat_end = fat_start + fat_size_sectors * bytes_per_sector;

        let used_clusters =
            (used_sectors + ops.sectors_per_cluster as u32 - 1) / ops.sectors_per_cluster as u32;
        let mut fs_info = FsInfo::with_ops(&ops, used_clusters);

        let fat = Fat32::from_bytes_mut(&mut data[fat_start..fat_end]);
        fat.init();
        // Mark root cluster as EOF (allocated)
        fat.mark_cluster_as(ops.root_cluster as usize, 0xFFFF_FFFF);
        // Foor cluster allocated
        fs_info.free_count -= 1;
        if ops.root_cluster == fs_info.next_free {
            fs_info.next_free += 1;
        }

        // We dont care about sector size, it is always 512 bytes
        const BOOT_SECTOR_SIZE: usize = 512;
        boot_sector.copy_to_bytes((&mut data[0..BOOT_SECTOR_SIZE]).try_into().unwrap());
        let fs_info_start = ops.fs_info_sector as usize * bytes_per_sector;
        fs_info.write(&mut data[fs_info_start..fs_info_start + bytes_per_sector]);
        if ops.boot_sector != 0 {
            let start = ops.boot_sector as usize * bytes_per_sector;
            boot_sector.copy_to_bytes(
                (&mut data[start..start + BOOT_SECTOR_SIZE])
                    .try_into()
                    .unwrap(),
            );
            let start = start + bytes_per_sector;
            fs_info.write(&mut data[start..start + bytes_per_sector]);
        }

        let (reserved, rest) =
            data.split_at_mut(ops.reserved_sector_count as usize * bytes_per_sector);
        let (fat, data) = rest.split_at_mut(fat_size_sectors * bytes_per_sector);

        Self {
            reserved,
            fat,
            data,

            bs: boot_sector.info(),
            descriptors: [None; MAX_OPEN],
        }
    }

    fn fs_info_range<'a>(&'a mut self) -> core::ops::Range<usize> {
        let bytes_per_sector = self.bs.bytes_per_sector() as usize;
        let fs_info_start = self.bs.fs_info_sector() as usize * bytes_per_sector;
        fs_info_start..fs_info_start + bytes_per_sector
    }

    fn allocate_clusters(&mut self, count: u32) -> u32 {
        let range = self.fs_info_range();
        let (mut next_free, mut free_count) = {
            let fs_info = FsInfo::from_bytes(&self.reserved[range.clone()]);
            (fs_info.next_free, fs_info.free_count)
        };
        let cluster = Fat32::from_bytes_mut(&mut self.fat).allocate_clusters(
            &mut free_count,
            &mut next_free,
            count,
        );
        let fs_info = FsInfo::from_bytes_mut(&mut self.reserved[range]);
        fs_info.next_free = next_free;
        fs_info.free_count = free_count;
        cluster
    }

    pub fn create_file(&mut self, path: &str, data: &[u8]) {
        use structures::directory::Directory;

        assert!(path.len() > 0);
        assert!(data.len() < u32::MAX as usize);
        let path = path.split('.').collect::<Vec<_>>();
        let cluster_free = self.allocate_clusters(self.to_clusters_rounded_up(data.len()) as u32);
        let file = FileEntry::new(
            path[0],
            path[1],
            FileAttributes::ARCHIVE,
            data.len() as u32,
            cluster_free,
        );
        let cluster_size =
            self.bs.bytes_per_sector() as usize * self.bs.sectors_per_cluster() as usize;
        let root_cluster = self.bs.root_cluster();
        let cluster_start = (root_cluster as usize - 2) * cluster_size;
        let directory =
            Directory::from_bytes_mut(&mut self.data[cluster_start..cluster_start + cluster_size]);
        let index = directory.write_entry(file);
        assert!(
            index.is_some(),
            "Directory is full, allocating more space is not implemented"
        );

        Fat32::from_bytes_mut(&mut self.fat).write_data(
            &mut self.data,
            cluster_size,
            cluster_free,
            0,
            data,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
