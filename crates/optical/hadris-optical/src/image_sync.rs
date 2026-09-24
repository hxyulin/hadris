use crate::detect::OpticalFormats;
use crate::{Error, OpenPolicy, OpticalFormat, Result};
use hadris_io::SeekFrom;
use hadris_io::legacy::sync::{Read, Seek};

/// One opened filesystem selected from an optical image.
#[non_exhaustive]
pub enum OpenOpticalImage<'a, S>
where
    S: Read + Seek,
{
    /// An opened ISO 9660 filesystem.
    Iso9660(hadris_iso::sync::IsoImage<StreamBlocks<&'a mut S>>),
    /// An opened UDF filesystem.
    Udf(hadris_udf::sync::UdfVolume<&'a mut S>),
}

impl<'a, S> OpenOpticalImage<'a, S>
where
    S: Read + Seek,
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
        source.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
        match selected {
            OpticalFormat::Iso9660 => {
                let blocks = StreamBlocks::new(source).map_err(Error::Io)?;
                hadris_iso::sync::IsoImage::open(blocks)
                    .map(Self::Iso9660)
                    .map_err(|err| Error::Iso(err.into_error().into()))
            }
            OpticalFormat::Udf => hadris_udf::sync::UdfVolume::open(source)
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
    pub fn as_iso9660(&self) -> Option<&hadris_iso::sync::IsoImage<StreamBlocks<&'a mut S>>> {
        match self {
            Self::Iso9660(image) => Some(image),
            Self::Udf(_) => None,
        }
    }

    /// Borrows the UDF handle when that format was selected.
    pub fn as_udf(&self) -> Option<&hadris_udf::sync::UdfVolume<&'a mut S>> {
        match self {
            Self::Udf(image) => Some(image),
            Self::Iso9660(_) => None,
        }
    }

    /// Mutably borrows the ISO 9660 handle when that format was selected.
    pub fn as_iso9660_mut(
        &mut self,
    ) -> Option<&mut hadris_iso::sync::IsoImage<StreamBlocks<&'a mut S>>> {
        match self {
            Self::Iso9660(image) => Some(image),
            Self::Udf(_) => None,
        }
    }

    /// Mutably borrows the UDF handle when that format was selected.
    pub fn as_udf_mut(&mut self) -> Option<&mut hadris_udf::sync::UdfVolume<&'a mut S>> {
        match self {
            Self::Udf(image) => Some(image),
            Self::Iso9660(_) => None,
        }
    }

    /// Closes the selected filesystem and returns the borrowed source.
    pub fn into_inner(self) -> &'a mut S {
        match self {
            Self::Iso9660(image) => image.into_inner().into_inner(),
            Self::Udf(image) => image.into_inner(),
        }
    }
}

/// A legacy byte stream as a block device of 2048-byte blocks, so the
/// ISO 9660 reader can open it. Transitional until the optical facade moves
/// to block devices.
#[derive(Debug)]
pub struct StreamBlocks<S> {
    inner: S,
    blocks: u64,
}

impl<S: Seek> StreamBlocks<S> {
    /// Wraps `inner`, measuring its length.
    pub fn new(mut inner: S) -> core::result::Result<Self, hadris_io::legacy::Error> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self {
            inner,
            blocks: len / 2048,
        })
    }

    /// Returns the stream.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S> hadris_io::ErrorType for StreamBlocks<S> {
    type Error = hadris_io::legacy::Error;
}

impl<S: Read + Seek> hadris_storage::sync::BlockDevice for StreamBlocks<S> {
    fn block_size(&self) -> hadris_storage::BlockSize {
        const { hadris_storage::BlockSize::new(2048).unwrap() }
    }

    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> core::result::Result<(), Self::Error> {
        self.inner.seek(SeekFrom::Start(first.get() * 2048))?;
        self.inner.read_exact(buf)
    }
}
