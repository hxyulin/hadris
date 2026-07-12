use crate::detect::{BlockFormat, FatVariant};
use crate::{Error, Result};
use hadris_fat::r#async::fat_table::FatType;
use hadris_fat::r#async::fs::FatFs;
use hadris_io::SeekFrom;
use hadris_io::r#async::{Borrowed, Read, Seek};

/// An asynchronously opened block filesystem with concrete-format access.
#[non_exhaustive]
pub enum OpenVolume<'a, S>
where
    S: Seek,
{
    Fat(FatFs<Borrowed<'a, S>>),
}

impl<'a, S> OpenVolume<'a, S>
where
    S: Read + Seek<Error = <S as Read>::Error>,
{
    pub async fn open(source: &'a mut S, logical_block_size: u32) -> Result<Self> {
        match crate::detect::r#async::detect(source, logical_block_size).await? {
            Some(BlockFormat::Fat(format)) => Self::open_detected(source, format).await,
            Some(BlockFormat::PartitionTable(kind)) => Err(Error::PartitionedDisk(kind)),
            None => Err(Error::UnknownFormat),
        }
    }

    pub async fn open_detected(source: &'a mut S, detected: FatVariant) -> Result<Self> {
        if detected == FatVariant::ExFat {
            return Err(Error::UnsupportedFormat(BlockFormat::Fat(detected)));
        }
        source
            .seek(SeekFrom::Start(0))
            .await
            .map_err(hadris_io::Error::erase)?;
        let fat = FatFs::open(Borrowed::new(source)).await?;
        let opened = fat_variant(fat.fat_type());
        if opened != detected {
            return Err(Error::DetectedFormatMismatch { detected, opened });
        }
        Ok(Self::Fat(fat))
    }

    pub fn format(&self) -> FatVariant {
        match self {
            Self::Fat(fat) => fat_variant(fat.fat_type()),
        }
    }

    pub fn as_fat(&self) -> Option<&FatFs<Borrowed<'a, S>>> {
        match self {
            Self::Fat(fat) => Some(fat),
        }
    }

    pub fn as_fat_mut(&mut self) -> Option<&mut FatFs<Borrowed<'a, S>>> {
        match self {
            Self::Fat(fat) => Some(fat),
        }
    }

    #[allow(clippy::result_large_err)]
    pub fn into_fat(self) -> core::result::Result<FatFs<Borrowed<'a, S>>, Self> {
        match self {
            Self::Fat(fat) => Ok(fat),
        }
    }

    pub fn into_inner(self) -> &'a mut S {
        match self {
            Self::Fat(fat) => fat.into_inner().0,
        }
    }
}

fn fat_variant(format: FatType) -> FatVariant {
    match format {
        FatType::Fat12 => FatVariant::Fat12,
        FatType::Fat16 => FatVariant::Fat16,
        FatType::Fat32 => FatVariant::Fat32,
    }
}
