use hadris_fat::FatKind;
use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FsResult, FsStats, Metadata, Name, NameBuf,
    NewNode, NodeId, RemoveKind, RenameFlags, SetMetadata,
};

use super::{BlockDevice, ExFatFs, FatFs, NtfsFs, detect};
use crate::detect::{BlockFormat, FatVariant};
use crate::{Detail, Error, OpenError};

fn fat_variant(kind: FatKind) -> Option<FatVariant> {
    match kind {
        FatKind::Fat12 => Some(FatVariant::Fat12),
        FatKind::Fat16 => Some(FatVariant::Fat16),
        FatKind::Fat32 => Some(FatVariant::Fat32),
        _ => None,
    }
}

fn read_only<T, E>() -> FsResult<T, E> {
    Err(ErrorKind::ReadOnly.into())
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
/// It implements `hadris_fs::FsDriver` by delegating to the driver it
/// opened, so generic code lists and reads any of them the same way. NTFS
/// is read-only, so its write methods fail with [`ErrorKind::ReadOnly`].
/// [`as_fat`](Self::as_fat), [`into_fat`](Self::into_fat),
/// [`as_exfat`](Self::as_exfat) and [`into_exfat`](Self::into_exfat) reach
/// the FAT and exFAT drivers' native APIs; the NTFS driver is a preview and is reached the
/// same way only with the `unstable-ntfs` feature.
///
/// ```rust,ignore
/// let mut volume = OpenVolume::open(dev)?;
/// for entry in hadris_fs::sync::DriverExt::read_dir(&mut volume, "/")? {
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
    /// A partitioned disk fails with [`Detail::PartitionedDisk`]; open a
    /// partition of it, from `hadris_part`, instead. On failure the
    /// [`OpenError`] gives `dev` back.
    pub async fn open(mut dev: D) -> Result<Self, OpenError<D, D::Error>> {
        match detect(&mut dev).await {
            Ok(Some(format)) => Self::open_detected(dev, format).await,
            Ok(None) => Err(OpenError::new(
                Error::new(ErrorKind::Unsupported, Detail::UnknownFormat),
                dev,
            )),
            Err(err) => Err(OpenError::new(err.into(), dev)),
        }
    }

    /// Opens `dev` as a previously detected format.
    ///
    /// The volume is mounted once. On failure, including a FAT variant
    /// other than `detected`, the [`OpenError`] gives `dev` back.
    pub async fn open_detected(dev: D, detected: BlockFormat) -> Result<Self, OpenError<D, D::Error>> {
        let unsupported = Error::new(ErrorKind::Unsupported, Detail::UnsupportedFormat(detected));
        match detected {
            BlockFormat::Fat(FatVariant::ExFat) => match ExFatFs::open(dev).await {
                Ok(exfat) => Ok(Self { inner: Inner::ExFat(exfat) }),
                Err(err) => {
                    let (err, dev) = err.into_parts();
                    Err(OpenError::new(Error::mount(err, detected), dev))
                }
            },
            BlockFormat::Fat(variant) => {
                let fat = match FatFs::open(dev).await {
                    Ok(fat) => fat,
                    Err(err) => {
                        let (err, dev) = err.into_parts();
                        return Err(OpenError::new(Error::mount(err, detected), dev));
                    }
                };
                match fat_variant(fat.kind()) {
                    Some(opened) if opened == variant => Ok(Self { inner: Inner::Fat(fat) }),
                    Some(opened) => Err(OpenError::new(
                        Error::new(
                            ErrorKind::Corrupt,
                            Detail::FormatMismatch {
                                detected: variant,
                                opened,
                            },
                        ),
                        fat.into_inner(),
                    )),
                    None => Err(OpenError::new(unsupported, fat.into_inner())),
                }
            }
            BlockFormat::Ntfs => match NtfsFs::open(dev).await {
                Ok(ntfs) => Ok(Self { inner: Inner::Ntfs(ntfs) }),
                Err(err) => {
                    let (err, dev) = err.into_parts();
                    Err(OpenError::new(Error::mount(err, detected), dev))
                }
            },
            BlockFormat::PartitionTable(kind) => Err(OpenError::new(
                Error::new(ErrorKind::InvalidInput, Detail::PartitionedDisk(kind)),
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
    /// `ExFatFs` has not written yet is lost; call [`sync`](Self::sync) first.
    pub fn into_inner(self) -> D {
        match self.inner {
            Inner::Fat(fat) => fat.into_inner(),
            Inner::ExFat(fat) => fat.into_inner(),
            Inner::Ntfs(ntfs) => ntfs.into_inner(),
        }
    }

    /// The opened driver's capabilities.
    pub fn capabilities(&self) -> Capabilities {
        match &self.inner {
            Inner::Fat(fat) => fat.capabilities(),
            Inner::ExFat(fat) => fat.capabilities(),
            Inner::Ntfs(ntfs) => ntfs.capabilities(),
        }
    }

    /// The root directory.
    pub fn root(&self) -> NodeId {
        match &self.inner {
            Inner::Fat(fat) => fat.root(),
            Inner::ExFat(fat) => fat.root(),
            Inner::Ntfs(ntfs) => ntfs.root(),
        }
    }

    /// Finds `name` in `dir` and pins the result.
    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.lookup(dir, name).await,
            Inner::ExFat(fat) => fat.lookup(dir, name).await,
            Inner::Ntfs(ntfs) => ntfs.lookup(dir, name).await,
        }
    }

    /// Metadata of a node.
    pub async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.node_metadata(node).await,
            Inner::ExFat(fat) => fat.node_metadata(node).await,
            Inner::Ntfs(ntfs) => ntfs.node_metadata(node).await,
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
            Inner::Fat(fat) => fat.read_dir_entry(dir, cursor, name).await,
            Inner::ExFat(fat) => fat.read_dir_entry(dir, cursor, name).await,
            Inner::Ntfs(ntfs) => ntfs.read_dir_entry(dir, cursor, name).await,
        }
    }

    /// Reads from a file at `offset`.
    pub async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.read_at(node, offset, buf).await,
            Inner::ExFat(fat) => fat.read_at(node, offset, buf).await,
            Inner::Ntfs(ntfs) => ntfs.read_at(node, offset, buf).await,
        }
    }

    /// Size and free space of the volume.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.stats().await,
            Inner::ExFat(fat) => fat.stats().await,
            Inner::Ntfs(ntfs) => ntfs.stats().await,
        }
    }

    /// Unpins a node.
    pub fn forget(&mut self, node: NodeId) {
        match &mut self.inner {
            Inner::Fat(fat) => fat.forget(node),
            Inner::ExFat(fat) => fat.forget(node),
            Inner::Ntfs(ntfs) => ntfs.forget(node),
        }
    }

    /// The directory containing `dir`.
    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.parent(dir).await,
            Inner::ExFat(fat) => fat.parent(dir).await,
            Inner::Ntfs(ntfs) => ntfs.parent(dir).await,
        }
    }

    /// Marks a pinned node open.
    pub async fn open_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.open_node(node).await,
            Inner::ExFat(fat) => fat.open_node(node).await,
            Inner::Ntfs(_) => Ok(()),
        }
    }

    /// Ends an [`open_node`](Self::open_node).
    pub fn close_node(&mut self, node: NodeId) {
        match &mut self.inner {
            Inner::Fat(fat) => fat.close_node(node),
            Inner::ExFat(fat) => fat.close_node(node),
            Inner::Ntfs(_) => {}
        }
    }

    /// Writes a node's pending metadata without flushing the device.
    pub async fn publish_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.publish_node(node).await,
            Inner::ExFat(fat) => fat.publish_node(node).await,
            Inner::Ntfs(_) => Ok(()),
        }
    }

    /// Creates `name` in `dir` and pins it.
    pub async fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.create(dir, name, kind, meta).await,
            Inner::ExFat(fat) => fat.create(dir, name, kind, meta).await,
            Inner::Ntfs(_) => read_only(),
        }
    }

    /// Removes `name` from `dir`.
    pub async fn remove(&mut self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.remove(dir, name, kind).await,
            Inner::ExFat(fat) => fat.remove(dir, name, kind).await,
            Inner::Ntfs(_) => read_only(),
        }
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`.
    pub async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.rename(from_dir, from, to_dir, to, flags).await,
            Inner::ExFat(fat) => fat.rename(from_dir, from, to_dir, to, flags).await,
            Inner::Ntfs(_) => read_only(),
        }
    }

    /// Writes to a file at `offset`.
    pub async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.write_at(node, offset, buf).await,
            Inner::ExFat(fat) => fat.write_at(node, offset, buf).await,
            Inner::Ntfs(_) => read_only(),
        }
    }

    /// Sets a file's length.
    pub async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.set_len(node, len).await,
            Inner::ExFat(fat) => fat.set_len(node, len).await,
            Inner::Ntfs(_) => read_only(),
        }
    }

    /// Changes a node's metadata.
    pub async fn set_metadata(&mut self, node: NodeId, changes: &SetMetadata) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.set_metadata(node, changes).await,
            Inner::ExFat(fat) => fat.set_metadata(node, changes).await,
            Inner::Ntfs(_) => read_only(),
        }
    }

    /// Makes one node durable.
    pub async fn sync_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.sync_node(node).await,
            Inner::ExFat(fat) => fat.sync_node(node).await,
            Inner::Ntfs(_) => Ok(()),
        }
    }

    /// Writes every piece of cached metadata and flushes the device.
    pub async fn sync(&mut self) -> FsResult<(), D::Error> {
        match &mut self.inner {
            Inner::Fat(fat) => fat.sync().await,
            Inner::ExFat(fat) => fat.sync().await,
            Inner::Ntfs(_) => Ok(()),
        }
    }
}

}

impl_block_driver!(impl[D: BlockDevice] OpenVolume<D>, error = D::Error; also = [parent, open_node, close_node, publish_node]);
