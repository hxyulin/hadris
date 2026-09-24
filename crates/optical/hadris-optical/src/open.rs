use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FsResult, FsStats, Metadata, MountError, Name,
    NameBuf, NodeId,
};
use hadris_iso::Namespace;

use super::{BlockDevice, IsoImage, IsoView, UdfFs, detect};
use crate::detect::OpticalFormats;
use crate::error::Error;
use crate::{Detail, OpenPolicy, OpticalFormat};

fn unknown<E>() -> Error<E> {
    Error::new(ErrorKind::NotRecognized, "no ISO 9660 or UDF volume")
}

io_transform! {

// Boxing the larger driver would need an allocator.
#[allow(clippy::large_enum_variant)]
enum Inner<D> {
    Iso(IsoView<D>),
    Udf(UdfFs<D>),
}

/// An optical image opened by detection: an ISO 9660 view or a UDF volume.
///
/// It implements `hadris_fs::FsDriver` read-only by delegating to the
/// driver it opened, so generic code lists and reads either the same way.
/// ISO 9660 opens with [`Namespace::Preferred`]; open `hadris_iso`
/// directly to pick another tree. [`as_iso`](Self::as_iso) and
/// [`as_udf`](Self::as_udf) reach the drivers' native API.
///
/// ```rust,ignore
/// let mut image = OpenOpticalImage::open(dev, OpenPolicy::PreferUdf)?;
/// let data = hadris_fs::sync::DriverExt::read_to_vec(&mut image, "/README.TXT")?;
/// ```
pub struct OpenOpticalImage<D> {
    inner: Inner<D>,
}

impl<D: BlockDevice> core::fmt::Debug for OpenOpticalImage<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenOpticalImage")
            .field("format", &self.format())
            .finish_non_exhaustive()
    }
}

impl<D: BlockDevice> OpenOpticalImage<D> {
    /// Detects the filesystems of `dev` and opens the one `policy`
    /// selects. An image with neither fails with
    /// [`ErrorKind::NotRecognized`]. On failure the [`MountError`] gives
    /// `dev` back.
    pub async fn open(mut dev: D, policy: OpenPolicy) -> Result<Self, MountError<D, D::Error>> {
        match detect(&mut dev).await {
            Ok(Some(formats)) => Self::open_detected(dev, formats, policy).await,
            Ok(None) => Err(MountError::new(unknown(), dev)),
            Err(err) => Err(MountError::new(err, dev)),
        }
    }

    /// Opens the filesystem `policy` selects from previously detected
    /// `formats`. The volume is mounted once. On failure the
    /// [`MountError`] gives `dev` back; a driver that refuses the volume
    /// returns its own error.
    pub async fn open_detected(
        dev: D,
        formats: OpticalFormats,
        policy: OpenPolicy,
    ) -> Result<Self, MountError<D, D::Error>> {
        let Some(selected) = policy.select(formats) else {
            let err = match policy.required() {
                Some(_) => Detail::FormatUnavailable.error(ErrorKind::Unsupported),
                None => unknown(),
            };
            return Err(MountError::new(err, dev));
        };
        match selected {
            OpticalFormat::Udf => {
                UdfFs::open(dev).await.map(|udf| Self { inner: Inner::Udf(udf) })
            }
            OpticalFormat::Iso9660 => {
                let image = IsoImage::open(dev).await?;
                let view = image.into_view(Namespace::Preferred)?;
                Ok(Self { inner: Inner::Iso(view) })
            }
        }
    }

    /// The filesystem that was opened.
    pub fn format(&self) -> OpticalFormat {
        match &self.inner {
            Inner::Iso(_) => OpticalFormat::Iso9660,
            Inner::Udf(_) => OpticalFormat::Udf,
        }
    }

    /// Borrows the ISO 9660 view, if ISO 9660 was opened.
    pub fn as_iso(&self) -> Option<&IsoView<D>> {
        match &self.inner {
            Inner::Iso(view) => Some(view),
            Inner::Udf(_) => None,
        }
    }

    /// Mutably borrows the ISO 9660 view, if ISO 9660 was opened.
    pub fn as_iso_mut(&mut self) -> Option<&mut IsoView<D>> {
        match &mut self.inner {
            Inner::Iso(view) => Some(view),
            Inner::Udf(_) => None,
        }
    }

    /// Takes the ISO 9660 view, or gives `self` back.
    #[allow(clippy::result_large_err)]
    pub fn into_iso(self) -> Result<IsoView<D>, Self> {
        match self.inner {
            Inner::Iso(view) => Ok(view),
            inner => Err(Self { inner }),
        }
    }

    /// Borrows the UDF volume, if UDF was opened.
    pub fn as_udf(&self) -> Option<&UdfFs<D>> {
        match &self.inner {
            Inner::Udf(udf) => Some(udf),
            Inner::Iso(_) => None,
        }
    }

    /// Mutably borrows the UDF volume, if UDF was opened.
    pub fn as_udf_mut(&mut self) -> Option<&mut UdfFs<D>> {
        match &mut self.inner {
            Inner::Udf(udf) => Some(udf),
            Inner::Iso(_) => None,
        }
    }

    /// Takes the UDF volume, or gives `self` back.
    #[allow(clippy::result_large_err)]
    pub fn into_udf(self) -> Result<UdfFs<D>, Self> {
        match self.inner {
            Inner::Udf(udf) => Ok(udf),
            inner => Err(Self { inner }),
        }
    }

    /// Closes the filesystem and returns the device.
    pub fn into_inner(self) -> D {
        match self.inner {
            Inner::Iso(view) => view.into_inner(),
            Inner::Udf(udf) => udf.into_inner(),
        }
    }

    /// The opened driver's capabilities; both are read-only.
    pub fn capabilities(&self) -> Capabilities {
        match &self.inner {
            Inner::Iso(view) => view.capabilities(),
            Inner::Udf(udf) => udf.capabilities(),
        }
    }

    /// The root directory.
    pub fn root(&self) -> NodeId {
        match &self.inner {
            Inner::Iso(view) => view.root(),
            Inner::Udf(udf) => udf.root(),
        }
    }

    /// Finds `name` in `dir`.
    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.lookup(dir, name).await,
            Inner::Udf(udf) => udf.lookup(dir, name).await,
        }
    }

    /// Metadata of a node.
    pub async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.node_metadata(node).await,
            Inner::Udf(udf) => udf.node_metadata(node).await,
        }
    }

    /// Writes the entry after `cursor` into `name` and advances `cursor`.
    pub async fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.read_dir_entry(dir, cursor, name).await,
            Inner::Udf(udf) => udf.read_dir_entry(dir, cursor, name).await,
        }
    }

    /// Reads from a file at `offset`.
    pub async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.read_at(node, offset, buf).await,
            Inner::Udf(udf) => udf.read_at(node, offset, buf).await,
        }
    }

    /// Size and free space of the volume.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.stats().await,
            Inner::Udf(udf) => udf.stats().await,
        }
    }

    /// Does nothing: ISO 9660 and UDF node ids are stable.
    pub fn forget(&mut self, node: NodeId) {
        match &mut self.inner {
            Inner::Iso(view) => view.forget(node),
            Inner::Udf(udf) => udf.forget(node),
        }
    }

    /// The directory containing `dir`.
    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.parent(dir).await,
            Inner::Udf(udf) => udf.parent(dir).await,
        }
    }

    /// Writes a symlink's target into `buf`.
    pub async fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        match &mut self.inner {
            Inner::Iso(view) => view.read_link(link, buf).await,
            Inner::Udf(udf) => udf.read_link(link, buf).await,
        }
    }
}

}

impl_optical_driver!(impl[D: BlockDevice] OpenOpticalImage<D>, error = D::Error, read_only; also = [parent, read_link]);
