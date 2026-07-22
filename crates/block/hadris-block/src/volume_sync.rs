use crate::detect::{BlockFormat, FatVariant};
use crate::{Error, Result};
use hadris_io::SeekFrom;
use hadris_io::sync::{Borrowed, Read, Seek};

/// An opened block filesystem with lossless access to its concrete handle.
#[non_exhaustive]
pub enum OpenVolume<'a, S>
where
    S: Seek,
{
    /// An opened FAT12, FAT16, or FAT32 filesystem.
    Fat(hadris_fat::sync::FatVolume<Borrowed<'a, S>>),
}

impl<'a, S> OpenVolume<'a, S>
where
    S: Read + Seek<Error = <S as Read>::Error>,
{
    /// Detects and opens a filesystem at the beginning of `source`.
    ///
    /// Partitioned disks must first be narrowed to a partition view.
    pub fn open(source: &'a mut S, logical_block_size: u32) -> Result<Self> {
        match crate::detect::sync::detect(source, logical_block_size)? {
            Some(BlockFormat::Fat(format)) => Self::open_detected(source, format),
            Some(BlockFormat::PartitionTable(kind)) => Err(Error::PartitionedDisk(kind)),
            None => Err(Error::UnknownFormat),
        }
    }

    /// Opens a filesystem using a previously detected FAT variant.
    pub fn open_detected(source: &'a mut S, detected: FatVariant) -> Result<Self> {
        if detected == FatVariant::ExFat {
            return Err(Error::UnsupportedFormat(BlockFormat::Fat(detected)));
        }
        source
            .seek(SeekFrom::Start(0))
            .map_err(hadris_io::Error::erase)?;
        let fat = hadris_fat::sync::FatVolume::open(Borrowed::new(source))?;
        let opened = fat_variant(fat.fat_type());
        if opened != detected {
            return Err(Error::DetectedFormatMismatch { detected, opened });
        }
        Ok(Self::Fat(fat))
    }

    /// Returns the concrete FAT variant of the opened filesystem.
    pub fn format(&self) -> FatVariant {
        match self {
            Self::Fat(fat) => fat_variant(fat.fat_type()),
        }
    }

    /// Borrows the opened FAT filesystem.
    pub fn as_fat(&self) -> Option<&hadris_fat::sync::FatVolume<Borrowed<'a, S>>> {
        match self {
            Self::Fat(fat) => Some(fat),
        }
    }

    /// Mutably borrows the opened FAT filesystem.
    pub fn as_fat_mut(&mut self) -> Option<&mut hadris_fat::sync::FatVolume<Borrowed<'a, S>>> {
        match self {
            Self::Fat(fat) => Some(fat),
        }
    }

    #[allow(clippy::result_large_err)]
    /// Extracts the FAT filesystem, returning `self` if its format differs.
    pub fn into_fat(
        self,
    ) -> core::result::Result<hadris_fat::sync::FatVolume<Borrowed<'a, S>>, Self> {
        match self {
            Self::Fat(fat) => Ok(fat),
        }
    }

    /// Closes the filesystem and returns the borrowed source.
    pub fn into_inner(self) -> &'a mut S {
        match self {
            Self::Fat(fat) => fat.into_inner().0,
        }
    }
}

fn fat_variant(format: hadris_fat::sync::FatType) -> FatVariant {
    match format {
        hadris_fat::sync::FatType::Fat12 => FatVariant::Fat12,
        hadris_fat::sync::FatType::Fat16 => FatVariant::Fat16,
        hadris_fat::sync::FatType::Fat32 => FatVariant::Fat32,
    }
}
