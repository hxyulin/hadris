use hadris_fat::FatKind;
use hadris_fat::r#async::FatFs;
use hadris_storage::r#async::BlockDevice;

use crate::detect::{BlockFormat, FatVariant};
use crate::{Error, Result};

/// An asynchronously opened block filesystem with lossless access to its
/// concrete driver.
#[non_exhaustive]
pub enum OpenVolume<D> {
    /// An opened FAT12, FAT16, or FAT32 filesystem.
    Fat(FatFs<D>),
}

impl<D: BlockDevice> OpenVolume<D> {
    /// Detects and opens the filesystem on `dev`.
    ///
    /// Partitioned disks must first be narrowed to a partition, for example
    /// with [`partition::r#async`](crate::partition::r#async).
    pub async fn open(mut dev: D) -> Result<Self, D::Error> {
        match crate::detect::r#async::detect(&mut dev)
            .await
            .map_err(Error::Device)?
        {
            Some(BlockFormat::Fat(format)) => Self::open_detected(dev, format).await,
            Some(BlockFormat::PartitionTable(kind)) => Err(Error::PartitionedDisk(kind)),
            _ => Err(Error::UnknownFormat),
        }
    }

    /// Opens `dev` as a previously detected FAT variant.
    pub async fn open_detected(dev: D, detected: FatVariant) -> Result<Self, D::Error> {
        if detected == FatVariant::ExFat {
            return Err(Error::UnsupportedFormat(BlockFormat::Fat(detected)));
        }
        let fat = FatFs::open(dev).await?;
        let Some(opened) = fat_variant(fat.kind()) else {
            return Err(Error::UnsupportedFormat(BlockFormat::Fat(detected)));
        };
        if opened != detected {
            return Err(Error::DetectedFormatMismatch { detected, opened });
        }
        Ok(Self::Fat(fat))
    }

    /// Returns the concrete FAT variant of the opened filesystem.
    pub fn format(&self) -> FatVariant {
        match self {
            Self::Fat(fat) => fat_variant(fat.kind()).unwrap_or(FatVariant::ExFat),
        }
    }

    /// Borrows the opened FAT filesystem.
    pub fn as_fat(&self) -> Option<&FatFs<D>> {
        match self {
            Self::Fat(fat) => Some(fat),
        }
    }

    /// Mutably borrows the opened FAT filesystem.
    pub fn as_fat_mut(&mut self) -> Option<&mut FatFs<D>> {
        match self {
            Self::Fat(fat) => Some(fat),
        }
    }

    #[allow(clippy::result_large_err)]
    /// Extracts the FAT filesystem, returning `self` if its format differs.
    pub fn into_fat(self) -> core::result::Result<FatFs<D>, Self> {
        match self {
            Self::Fat(fat) => Ok(fat),
        }
    }

    /// Closes the filesystem and returns the device. Sizes and the free
    /// count `FatFs` has not written are lost; call its `sync` first.
    pub fn into_inner(self) -> D {
        match self {
            Self::Fat(fat) => fat.into_inner(),
        }
    }
}

fn fat_variant(kind: FatKind) -> Option<FatVariant> {
    match kind {
        FatKind::Fat12 => Some(FatVariant::Fat12),
        FatKind::Fat16 => Some(FatVariant::Fat16),
        FatKind::Fat32 => Some(FatVariant::Fat32),
        _ => None,
    }
}
