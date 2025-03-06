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

use hadris_core::{
    disk::{DiskError, DiskReader, DiskWriter},
    time::TimeProvider,
    FsCreationError,
};
use structures::{
    boot_sector::{BootSector, BootSectorInfo},
    fat::Fat32,
    fs_info::{FsInfo, FsInfoInfo},
};

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod structures;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat32,
    Fat16,
    Fat12,
}

impl core::fmt::Display for FatType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Fat32 => write!(f, "FAT32"),
            Self::Fat16 => write!(f, "FAT16"),
            Self::Fat12 => write!(f, "FAT12"),
        }
    }
}

/// A struct representing a FAT32 file system
/// Currently this only supports FAT32, but in the future it will support other FAT variants
/// This struct is not thread safe, and should only be used in a single thread
/// The struct is generic over the disk reader, which is used to read and write to the disk
/// If the disk reader also implements [`DiskWriter`], then functions for writing to the filesystem
/// will be available
pub struct FatFs<'a, D: DiskReader> {
    bs: BootSectorInfo,
    fs_info: FsInfoInfo,
    fat: Fat32,

    reader: &'a mut D,
    time_provider: &'a dyn TimeProvider,
}

impl<'a, T: DiskReader> FatFs<'a, T> {
    /// Creates a new FAT32 file system from the given reader.
    ///
    /// The time provider will be the [`hadris_core::time::default_time_provider`].
    /// If you want to specify a custom time provider, use the [`read_with_tp`] function.
    ///
    /// # Errors
    /// This function will return an error if the reader does not contain a valid FAT32 file system.
    ///
    /// # Examples
    /// ```
    /// use hadris_fat::FatFs;
    /// use hadris_core::disk::DiskReader;
    ///
    /// let mut disk = [0; 1024];
    /// let mut reader = &mut disk[..];
    /// let mut fat_fs = FatFs::read(&mut reader).unwrap();
    /// ```
    pub fn read(reader: &'a mut T) -> Result<Self, FsCreationError> {
        Self::read_with_tp(reader, hadris_core::time::default_time_provider())
    }

    /// Creates a new FAT32 file system from the given reader and time provider.
    ///
    /// If the `std` feature is enabled, the time provider will be the [`StdTimeProvider`].
    /// Otherwise, the time provider will be the [`NoTimeProvider`].
    /// If you want to read without manually providing a time provider, use the [`read`] function.
    pub fn read_with_tp(
        reader: &'a mut T,
        time_provider: &'a dyn TimeProvider,
    ) -> Result<Self, FsCreationError> {
        let mut bs_buffer = [0u8; 512];
        reader.read_sector(0, &mut bs_buffer)?;
        let bs = BootSector::from_bytes(&bs_buffer).info();
        reader.read_sector(bs.fs_info_sector() as u32, &mut bs_buffer)?;
        let fs_info = FsInfo::from_bytes(&bs_buffer).info();
        let fat = Fat32::new(
            // Start of FAT in bytes
            bs.reserved_sector_count() as usize * bs.bytes_per_sector() as usize,
            // Size of FAT in bytes
            bs.sectors_per_fat() as usize * bs.bytes_per_sector() as usize,
            bs.fat_count() as usize,
            bs.bytes_per_sector() as usize,
        );

        Ok(Self {
            reader,
            time_provider,
            bs,
            fs_info,
            fat,
        })
    }
}

impl<R: DiskReader> core::fmt::Debug for FatFs<'_, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FatFs")
            .field("bs", &self.bs)
            .field("fs_info", &self.fs_info)
            .finish()
    }
}

impl<'a, T: DiskReader + DiskWriter> FatFs<'a, T> {
    /// Flushes the FAT32 file system to the disk
    /// This ensures that all changes are written to the disk
    ///
    /// # Errors
    /// This function will return an error if there is an error while writing to the disk.
    pub fn flush(&mut self) -> Result<(), DiskError> {
        let mut buffer = [0u8; 512];
        // BPB shouldn't be modified, so we can just make sure it is still the same
        self.reader.read_sector(0, &mut buffer)?;
        let bpb = BootSector::from_bytes(&buffer).info();
        assert_eq!(self.bs, bpb);
        self.reader
            .read_sector(self.bs.fs_info_sector() as u32, &mut buffer)?;
        let fs_info = FsInfo::from_bytes_mut(&mut buffer);
        fs_info.set_free_clusters(self.fs_info.free_clusters);
        fs_info.set_next_free_cluster(self.fs_info.next_free_cluster);
        self.reader
            .write_sector(self.bs.fs_info_sector() as u32, &buffer)?;
        Ok(())
    }
}
