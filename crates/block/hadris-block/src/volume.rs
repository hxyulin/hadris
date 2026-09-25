use hadris_fat::FatKind;
use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FsResult, FsStats, Metadata, MountError,
    MountOptions, Name, NodeId, OpenMode, RenameMode, Resolve, SetAttr,
};

use super::{BlockDevice, ExFatFs, FatFs, FileSystem, NtfsFs, detect};
use crate::Detail;
use crate::detect::{BlockFormat, FatVariant};
use crate::error::Error;

fn fat_variant(kind: FatKind) -> Option<FatVariant> {
    match kind {
        FatKind::Fat12 => Some(FatVariant::Fat12),
        FatKind::Fat16 => Some(FatVariant::Fat16),
        FatKind::Fat32 => Some(FatVariant::Fat32),
        _ => None,
    }
}

/// Runs `$e` on whichever driver the volume opened, bound to `$fs`.
macro_rules! each {
    ($inner:expr, $fs:ident => $e:expr) => {
        match $inner {
            Inner::Fat($fs) => $e,
            Inner::ExFat($fs) => $e,
            Inner::Ntfs($fs) => $e,
        }
    };
}

io_transform! {

// Boxing the larger driver would need an allocator.
#[allow(clippy::large_enum_variant)]
enum Inner<D> {
    Fat(FatFs<D>),
    ExFat(ExFatFs<D>),
    Ntfs(NtfsFs<D>),
}

/// A block filesystem opened by detection: FAT12, FAT16, FAT32, exFAT or
/// NTFS.
///
/// It implements `hadris_fs` `FileSystem` by delegating to the driver it
/// opened, so generic code lists and reads any of them the same way. NTFS
/// is read-only, so its write methods fail with [`ErrorKind::ReadOnly`].
/// [`as_fat`](Self::as_fat), [`into_fat`](Self::into_fat),
/// [`as_exfat`](Self::as_exfat) and [`into_exfat`](Self::into_exfat) reach
/// the FAT and exFAT drivers' native APIs; the NTFS driver is a preview and is reached the
/// same way only with the `unstable-ntfs` feature.
///
/// ```rust,ignore
/// let vol = Volume::new(OpenVolume::open(dev)?);
/// for entry in vol.read_dir("/")? {
///     println!("{:?}", entry?.name());
/// }
/// ```
pub struct OpenVolume<D> {
    inner: Inner<D>,
}

impl<D: BlockDevice> core::fmt::Debug for OpenVolume<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenVolume")
            .field("format", &self.format())
            .finish_non_exhaustive()
    }
}

impl<D: BlockDevice> OpenVolume<D> {
    /// Detects and opens the filesystem on `dev`.
    ///
    /// A device with no known format fails with
    /// [`ErrorKind::NotRecognized`]. A partitioned disk fails with
    /// [`Detail::PartitionedDisk`]; open a partition of it, from
    /// `hadris_part`, instead. On failure the [`MountError`] gives `dev`
    /// back.
    pub async fn open(mut dev: D) -> Result<Self, MountError<D, D::Error>> {
        match detect(&mut dev).await {
            Ok(Some(format)) => Self::open_detected(dev, format).await,
            Ok(None) => Err(MountError::new(
                Error::new(ErrorKind::NotRecognized, "unknown block volume format"),
                dev,
            )),
            Err(err) => Err(MountError::new(err, dev)),
        }
    }

    /// Opens `dev` as a previously detected format.
    ///
    /// The volume is mounted once. On failure, including a FAT variant
    /// other than `detected`, the [`MountError`] gives `dev` back. A driver
    /// that refuses the volume returns its own error.
    pub async fn open_detected(dev: D, detected: BlockFormat) -> Result<Self, MountError<D, D::Error>> {
        let unsupported = Detail::UnsupportedFormat.error(ErrorKind::Unsupported);
        match detected {
            BlockFormat::Fat(FatVariant::ExFat) => {
                ExFatFs::mount(dev, MountOptions::new()).await.map(|exfat| Self { inner: Inner::ExFat(exfat) })
            }
            BlockFormat::Fat(variant) => {
                let fat = FatFs::mount(dev, MountOptions::new()).await?;
                match fat_variant(fat.kind()) {
                    Some(opened) if opened == variant => Ok(Self { inner: Inner::Fat(fat) }),
                    Some(_) => Err(MountError::new(
                        Detail::FormatMismatch.error(ErrorKind::Corrupt),
                        fat.into_inner(),
                    )),
                    None => Err(MountError::new(unsupported, fat.into_inner())),
                }
            }
            BlockFormat::Ntfs => {
                NtfsFs::mount(dev, MountOptions::new()).await.map(|ntfs| Self { inner: Inner::Ntfs(ntfs) })
            }
            BlockFormat::PartitionTable(_) => Err(MountError::new(
                Detail::PartitionedDisk.error(ErrorKind::InvalidInput),
                dev,
            )),
        }
    }

    /// The format of the opened filesystem.
    pub fn format(&self) -> BlockFormat {
        match &self.inner {
            Inner::Fat(fat) => BlockFormat::Fat(fat_variant(fat.kind()).unwrap_or(FatVariant::Fat32)),
            Inner::ExFat(_) => BlockFormat::Fat(FatVariant::ExFat),
            Inner::Ntfs(_) => BlockFormat::Ntfs,
        }
    }

    /// Borrows the FAT driver, if the volume is FAT.
    pub fn as_fat(&self) -> Option<&FatFs<D>> {
        match &self.inner {
            Inner::Fat(fat) => Some(fat),
            _ => None,
        }
    }

    /// Mutably borrows the FAT driver, if the volume is FAT.
    pub fn as_fat_mut(&mut self) -> Option<&mut FatFs<D>> {
        match &mut self.inner {
            Inner::Fat(fat) => Some(fat),
            _ => None,
        }
    }

    /// Takes the FAT driver, or gives `self` back if the volume is not FAT.
    #[allow(clippy::result_large_err)]
    pub fn into_fat(self) -> Result<FatFs<D>, Self> {
        match self.inner {
            Inner::Fat(fat) => Ok(fat),
            inner => Err(Self { inner }),
        }
    }

    /// Borrows the exFAT driver, if the volume is exFAT.
    pub fn as_exfat(&self) -> Option<&ExFatFs<D>> {
        match &self.inner {
            Inner::ExFat(exfat) => Some(exfat),
            _ => None,
        }
    }

    /// Mutably borrows the exFAT driver, if the volume is exFAT.
    pub fn as_exfat_mut(&mut self) -> Option<&mut ExFatFs<D>> {
        match &mut self.inner {
            Inner::ExFat(exfat) => Some(exfat),
            _ => None,
        }
    }

    /// Takes the exFAT driver, or gives `self` back if the volume is not
    /// exFAT.
    #[allow(clippy::result_large_err)]
    pub fn into_exfat(self) -> Result<ExFatFs<D>, Self> {
        match self.inner {
            Inner::ExFat(exfat) => Ok(exfat),
            inner => Err(Self { inner }),
        }
    }

    /// Borrows the NTFS driver, if the volume is NTFS.
    #[cfg(feature = "unstable-ntfs")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable-ntfs")))]
    pub fn as_ntfs(&self) -> Option<&NtfsFs<D>> {
        match &self.inner {
            Inner::Ntfs(ntfs) => Some(ntfs),
            _ => None,
        }
    }

    /// Mutably borrows the NTFS driver, if the volume is NTFS.
    #[cfg(feature = "unstable-ntfs")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable-ntfs")))]
    pub fn as_ntfs_mut(&mut self) -> Option<&mut NtfsFs<D>> {
        match &mut self.inner {
            Inner::Ntfs(ntfs) => Some(ntfs),
            _ => None,
        }
    }

    /// Takes the NTFS driver, or gives `self` back if the volume is not
    /// NTFS.
    #[cfg(feature = "unstable-ntfs")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable-ntfs")))]
    #[allow(clippy::result_large_err)]
    pub fn into_ntfs(self) -> Result<NtfsFs<D>, Self> {
        match self.inner {
            Inner::Ntfs(ntfs) => Ok(ntfs),
            inner => Err(Self { inner }),
        }
    }

    /// Closes the filesystem and returns the device. What `FatFs` or
    /// `ExFatFs` has not written yet is lost; call `sync` first.
    pub fn into_inner(self) -> D {
        match self.inner {
            Inner::Fat(fat) => fat.into_inner(),
            Inner::ExFat(fat) => fat.into_inner(),
            Inner::Ntfs(ntfs) => ntfs.into_inner(),
        }
    }
}

impl<D: BlockDevice> FileSystem for OpenVolume<D> {
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
