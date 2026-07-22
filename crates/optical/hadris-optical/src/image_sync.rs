use crate::detect::OpticalFormats;
use crate::{Error, OpenPolicy, OpticalFormat, Result};
use hadris_io::SeekFrom;
use hadris_io::sync::{Borrowed, Read, Seek};

/// One opened filesystem selected from an optical image.
#[non_exhaustive]
pub enum OpenOpticalImage<'a, S>
where
    S: Read + Seek<Error = <S as Read>::Error>,
{
    /// An opened ISO 9660 filesystem.
    Iso9660(hadris_iso::sync::IsoImage<Borrowed<'a, S>>),
    /// An opened UDF filesystem.
    Udf(hadris_udf::sync::UdfVolume<Borrowed<'a, S>>),
}

impl<'a, S> OpenOpticalImage<'a, S>
where
    S: Read + Seek<Error = <S as Read>::Error>,
{
    /// Detects the image and opens the filesystem selected by `policy`.
    pub fn open(source: &'a mut S, policy: OpenPolicy) -> Result<Self> {
        let formats = crate::detect::sync::detect(source)?.ok_or(Error::UnknownFormat)?;
        Self::open_detected(source, formats, policy)
    }

    /// Opens an image using a previously obtained detection result.
    pub fn open_detected(
        source: &'a mut S,
        formats: OpticalFormats,
        policy: OpenPolicy,
    ) -> Result<Self> {
        let selected = policy.select(formats).ok_or_else(|| {
            Error::RequestedFormatUnavailable(
                policy
                    .required()
                    .expect("preference policies always select a detected format"),
            )
        })?;
        source
            .seek(SeekFrom::Start(0))
            .map_err(|error| Error::Io(hadris_io::Error::erase(error)))?;
        match selected {
            OpticalFormat::Iso9660 => hadris_iso::sync::IsoImage::open(Borrowed::new(source))
                .map(Self::Iso9660)
                .map_err(Error::Iso),
            OpticalFormat::Udf => hadris_udf::sync::UdfVolume::open(Borrowed::new(source))
                .map(Self::Udf)
                .map_err(Error::Udf),
        }
    }

    /// Returns the concrete format selected from the image.
    pub const fn format(&self) -> OpticalFormat {
        match self {
            Self::Iso9660(_) => OpticalFormat::Iso9660,
            Self::Udf(_) => OpticalFormat::Udf,
        }
    }

    /// Borrows the ISO 9660 handle when that format was selected.
    pub fn as_iso9660(&self) -> Option<&hadris_iso::sync::IsoImage<Borrowed<'a, S>>> {
        match self {
            Self::Iso9660(image) => Some(image),
            Self::Udf(_) => None,
        }
    }

    /// Borrows the UDF handle when that format was selected.
    pub fn as_udf(&self) -> Option<&hadris_udf::sync::UdfVolume<Borrowed<'a, S>>> {
        match self {
            Self::Udf(image) => Some(image),
            Self::Iso9660(_) => None,
        }
    }

    /// Mutably borrows the ISO 9660 handle when that format was selected.
    pub fn as_iso9660_mut(&mut self) -> Option<&mut hadris_iso::sync::IsoImage<Borrowed<'a, S>>> {
        match self {
            Self::Iso9660(image) => Some(image),
            Self::Udf(_) => None,
        }
    }

    /// Mutably borrows the UDF handle when that format was selected.
    pub fn as_udf_mut(&mut self) -> Option<&mut hadris_udf::sync::UdfVolume<Borrowed<'a, S>>> {
        match self {
            Self::Udf(image) => Some(image),
            Self::Iso9660(_) => None,
        }
    }

    /// Closes the selected filesystem and returns the borrowed source.
    pub fn into_inner(self) -> &'a mut S {
        match self {
            Self::Iso9660(image) => image.into_inner().0,
            Self::Udf(image) => image.into_inner().0,
        }
    }
}
