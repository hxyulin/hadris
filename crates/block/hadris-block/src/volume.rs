use super::{BlockDevice, FatFs, detect};
use hadris_fat::FatKind;

use crate::detect::{BlockFormat, FatVariant};
use crate::{Error, OpenError};

io_transform! {

/// An opened block filesystem with lossless access to its concrete driver.
#[non_exhaustive]
pub enum OpenVolume<D> {
    /// An opened FAT12, FAT16, or FAT32 filesystem.
    Fat(FatFs<D>),
}

impl<D: BlockDevice> OpenVolume<D> {
    /// Detects and opens the filesystem on `dev`.
    ///
    /// Partitioned disks must first be narrowed to a partition with the
    /// `partition` module of the same I/O mode. On failure the [`OpenError`]
    /// gives `dev` back.
    pub async fn open(mut dev: D) -> Result<Self, OpenError<D, D::Error>> {
        let detected = match detect(&mut dev).await {
            Ok(detected) => detected,
            Err(error) => return Err(OpenError::new(Error::Device(error), dev)),
        };
        match detected {
            Some(BlockFormat::Fat(format)) => Self::open_detected(dev, format).await,
            Some(BlockFormat::PartitionTable(kind)) => {
                Err(OpenError::new(Error::PartitionedDisk(kind), dev))
            }
            _ => Err(OpenError::new(Error::UnknownFormat, dev)),
        }
    }

    /// Opens `dev` as a previously detected FAT variant.
    ///
    /// The volume is mounted once on a borrow of `dev` to validate it, so a
    /// volume that fails validation gives `dev` back in the [`OpenError`].
    pub async fn open_detected(
        mut dev: D,
        detected: FatVariant,
    ) -> Result<Self, OpenError<D, D::Error>> {
        if let Err(error) = validate(&mut dev, detected).await {
            return Err(OpenError::new(error, dev));
        }
        match FatFs::open(dev).await {
            Ok(fat) => Ok(Self::Fat(fat)),
            Err(error) => Err(OpenError::without_device(error.into_error().into())),
        }
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

/// Mounts the volume on a borrow of `dev` and checks it is `detected`.
async fn validate<D: BlockDevice>(dev: &mut D, detected: FatVariant) -> Result<(), Error<D::Error>> {
    let unsupported = Error::UnsupportedFormat(BlockFormat::Fat(detected));
    if detected == FatVariant::ExFat {
        return Err(unsupported);
    }
    let kind = FatFs::open(dev).await.map_err(|error| error.into_error())?.kind();
    match fat_variant(kind) {
        Some(opened) if opened == detected => Ok(()),
        Some(opened) => Err(Error::DetectedFormatMismatch { detected, opened }),
        None => Err(unsupported),
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
