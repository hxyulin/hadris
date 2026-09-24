use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FsResult, FsStats, Metadata, MountError, Name,
    NodeId, OpenMode, RenameMode, Resolve, SetAttr,
};
use hadris_iso::Namespace;

use super::{BlockDevice, FileSystem, IsoImage, IsoView, UdfFs, detect};
use crate::detect::OpticalFormats;
use crate::error::Error;
use crate::{Detail, OpenPolicy, OpticalFormat};

/// Runs `$e` on whichever driver the image opened, bound to `$fs`.
macro_rules! each {
    ($inner:expr, $fs:ident => $e:expr) => {
        match $inner {
            Inner::Iso($fs) => $e,
            Inner::Udf($fs) => $e,
        }
    };
}

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
/// It implements `hadris_fs` `FileSystem` read-only by delegating to the
/// driver it opened, so generic code lists and reads either the same way.
/// ISO 9660 opens with [`Namespace::Preferred`]; open `hadris_iso`
/// directly to pick another tree. [`as_iso`](Self::as_iso) and
/// [`as_udf`](Self::as_udf) reach the drivers' native API.
///
/// ```rust,ignore
/// let vol = Volume::new(OpenOpticalImage::open(dev, OpenPolicy::PreferUdf)?);
/// let mut readme = vol.open("/README.TXT", OpenOptions::new().read())?;
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
}

impl<D: BlockDevice> FileSystem for OpenOpticalImage<D> {
    type DeviceError = D::Error;

    fn capabilities(&self) -> Capabilities {
        each!(&self.inner, fs => fs.capabilities())
    }

    fn root(&self) -> NodeId {
        each!(&self.inner, fs => fs.root())
    }

    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        each!(&mut self.inner, fs => fs.statfs().await)
    }

    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        each!(&mut self.inner, fs => fs.label(buf).await)
    }

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        each!(&mut self.inner, fs => fs.lookup(dir, name).await)
    }

    fn forget(&mut self, node: NodeId, count: u64) {
        each!(&mut self.inner, fs => fs.forget(node, count))
    }

    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        each!(&mut self.inner, fs => fs.parent(dir).await)
    }

    async fn resolve(&mut self, path: &[u8], how: Resolve) -> FsResult<NodeId, D::Error> {
        each!(&mut self.inner, fs => fs.resolve(path, how).await)
    }

    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        each!(&mut self.inner, fs => fs.stat(node).await)
    }

    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        each!(&mut self.inner, fs => fs.readdir(dir, from).await)
    }

    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        each!(&mut self.inner, fs => fs.readlink(node, buf).await)
    }

    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.open(node, mode).await)
    }

    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.close(node).await)
    }

    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        each!(&mut self.inner, fs => fs.read(node, offset, buf).await)
    }

    async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.setattr(node, changes).await)
    }

    async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, D::Error> {
        each!(&mut self.inner, fs => fs.write(node, offset, buf).await)
    }

    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.truncate(node, len).await)
    }

    async fn fsync(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.fsync(node).await)
    }

    async fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, D::Error> {
        each!(&mut self.inner, fs => fs.create(dir, name, attrs).await)
    }

    async fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, D::Error> {
        each!(&mut self.inner, fs => fs.mkdir(dir, name, attrs).await)
    }

    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.unlink(dir, name).await)
    }

    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.rmdir(dir, name).await)
    }

    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.rename(from_dir, from, to_dir, to, mode).await)
    }

    async fn sync(&mut self) -> FsResult<(), D::Error> {
        each!(&mut self.inner, fs => fs.sync().await)
    }
}

}
