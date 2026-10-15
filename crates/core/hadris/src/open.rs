#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
use hadris_fs::{
    Capabilities, DirCursor, DirEntry, Extent, FsStats, Metadata, Name, NodeId, OpenMode,
    RenameMode, Resolve, SetAttr,
};
use hadris_fs::{ErrorKind, FsResult, MountError, MountOptions};
#[cfg(feature = "alloc")]
use hadris_part::Partition;
use hadris_storage::BlockIndex;

use crate::detect::{
    Detection, ImageFormat, Optical, Table, cpio_format, fat_kind, partition_table,
};
use hadris_fat_raw::io::BlockBuf;

use super::ApfsContainer;
#[cfg(feature = "alloc")]
use super::ApfsFs;
use super::{BlockDevice, IsoFs, UdfFs, exio, rawio};
#[cfg(feature = "alloc")]
use super::{ExFatFs, FatFs, FileSystem, part_open, part_read};
#[cfg(feature = "alloc")]
use crate::Select;

/// The largest device block `detect` reads.
const MAX_BLOCK: usize = 4096;

/// Bytes in an optical sector.
const OPTICAL_SECTOR: u64 = 2048;

/// Runs `$e` on whichever driver `$any` holds, bound to `$fs`.
#[cfg(feature = "alloc")]
macro_rules! each {
    ($any:expr, $fs:ident => $e:expr) => {
        match $any {
            AnyFs::Fat($fs) => $e,
            AnyFs::ExFat($fs) => $e,
            AnyFs::Iso($fs) => $e,
            AnyFs::Udf($fs) => $e,
            AnyFs::Apfs($fs) => $e,
        }
    };
}

io_transform! {

/// Reads `out.len()` bytes at byte `at`. `false` when the device ends
/// first.
async fn read_at<D: BlockDevice + ?Sized>(
    dev: &mut D,
    buf: &mut [u8; MAX_BLOCK],
    at: u64,
    out: &mut [u8],
) -> FsResult<bool, D::Error> {
    let block = dev.block_size().get() as usize;
    let end = at + out.len() as u64;
    if end > dev.block_count().saturating_mul(block as u64) {
        return Ok(false);
    }
    let mut pos = at;
    while pos < end {
        dev.read_blocks(BlockIndex::new(pos / block as u64), &mut buf[..block]).await?;
        let within = (pos % block as u64) as usize;
        let n = (block - within).min((end - pos) as usize);
        let done = (pos - at) as usize;
        out[done..done + n].copy_from_slice(&buf[within..within + n]);
        pos += n as u64;
    }
    Ok(true)
}

/// A structure `detect` recognized but its reader refused is damaged, not
/// foreign.
fn damaged<E, F>(err: &hadris_fs::Error<E>) -> hadris_fs::Error<F> {
    if err.kind() != ErrorKind::NotRecognized {
        return err.without_device();
    }
    let corrupt = hadris_fs::Error::new(ErrorKind::Corrupt, err.message());
    match err.detail() {
        Some(detail) => corrupt.with_detail(detail),
        None => corrupt,
    }
}

/// The damage of a candidate from its reader's result. Device errors end
/// detection.
fn damage<T, E>(result: FsResult<T, E>) -> FsResult<Option<hadris_fs::Error<core::convert::Infallible>>, E> {
    match result {
        Ok(_) => Ok(None),
        Err(err) if err.kind() == ErrorKind::Io => Err(err),
        Err(err) => Ok(Some(damaged(&err))),
    }
}

/// Finds what `dev` holds: the ISO 9660 and UDF recognition area from
/// byte 32768, then the first sector for NTFS, exFAT, FAT, a partition
/// table, an APFS container or a cpio archive. It reads a few blocks, never writes and does
/// not allocate.
///
/// A format whose signature is present is listed with the error its mount
/// would give when its first structures are damaged, with kind
/// [`ErrorKind::Corrupt`] where the reader would say `NotRecognized`, so a
/// damaged volume never reads as a foreign one. Devices whose blocks are larger than
/// 4096 bytes give an empty [`Detection`]. Fails only with the device's
/// errors.
pub async fn detect<D: BlockDevice + ?Sized>(dev: &mut D) -> FsResult<Detection, D::Error> {
    let mut found = Detection::new();
    let block = dev.block_size().get() as usize;
    if block > MAX_BLOCK {
        return Ok(found);
    }
    let mut buf = [0u8; MAX_BLOCK];

    let mut optical = Optical::default();
    for i in 0..Optical::SECTORS {
        let mut head = [0u8; 7];
        if !read_at(dev, &mut buf, (Optical::FIRST + i) * OPTICAL_SECTOR, &mut head).await? {
            break;
        }
        optical.inspect(&head);
    }
    let iso = if optical.iso {
        Some(damage(IsoFs::mount(&mut *dev, MountOptions::new()).await.map_err(MountError::into_error))?)
    } else {
        None
    };
    let udf = if optical.udf {
        Some(damage(UdfFs::mount(&mut *dev, MountOptions::new()).await.map_err(MountError::into_error))?)
    } else {
        None
    };
    if let (Some(_), Some(udf)) = (&iso, &udf) {
        found.push(ImageFormat::IsoUdfBridge, udf.clone());
    }
    if let Some(iso) = iso {
        found.push(ImageFormat::Iso, iso);
    }
    if let Some(udf) = udf {
        found.push(ImageFormat::Udf, udf);
    }

    let mut sector = [0u8; 512];
    if !read_at(dev, &mut buf, 0, &mut sector).await? {
        return Ok(found);
    }
    let signed = sector[510..512] == [0x55, 0xAA];
    let Some(mut raw) = BlockBuf::<[u8; MAX_BLOCK]>::new(block) else {
        return Ok(found);
    };
    if &sector[32..36] == b"NXSB" {
        let apfs_damage = damage(ApfsContainer::open(&mut *dev).await)?;
        found.push(ImageFormat::Apfs, apfs_damage);
    } else if signed && &sector[3..11] == b"NTFS    " {
        found.push(ImageFormat::Ntfs, None);
    } else if signed && &sector[3..11] == b"EXFAT   " {
        found.push(ImageFormat::ExFat, damage(exio::read_boot(&mut &mut *dev, &mut raw).await)?);
    } else if let Some(table) = partition_table(&sector) {
        let gpt = match table {
            Table::Mbr => false,
            _ => {
                let mut sig = [0u8; 8];
                let header = block.max(512) as u64;
                read_at(dev, &mut buf, header, &mut sig).await? && &sig == b"EFI PART"
            }
        };
        if gpt {
            found.push(ImageFormat::Gpt, None);
        }
        if table != Table::Gpt {
            found.push(ImageFormat::Mbr, None);
        }
    } else if let Some(kind) = fat_kind(&sector) {
        let geo = rawio::read_geometry(&mut &mut *dev, &mut raw).await;
        let kind = geo.as_ref().map_or(kind, |geo| geo.kind());
        found.push(ImageFormat::Fat(kind), damage(geo)?);
    } else if let Some(format) = cpio_format(&sector) {
        found.push(ImageFormat::Cpio(format), None);
    }
    Ok(found)
}

/// A mounted filesystem of any format [`open`] reaches. It implements
/// `FileSystem` by delegation; format extras are reached by `match`.
///
/// NTFS is added as a variant once it is stable.
#[cfg(feature = "alloc")]
#[derive(Debug)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum AnyFs<D> {
    /// A FAT12, FAT16 or FAT32 volume.
    Fat(FatFs<D>),
    /// An exFAT volume.
    ExFat(ExFatFs<D>),
    /// An ISO 9660 image.
    Iso(IsoFs<D>),
    /// A UDF volume, including the UDF side of an ISO 9660 and UDF bridge.
    Udf(UdfFs<D>),
    /// An explicitly selected APFS volume or a container's sole volume.
    /// Native APFS extensions are a preview and may change in 3.x minors.
    Apfs(ApfsFs<D>),
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice> AnyFs<D> {
    /// Syncs the volume and gives the device back.
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        each!(self, fs => fs.unmount().await)
    }

    /// Gives the device back without syncing.
    pub fn into_inner(self) -> D {
        each!(self, fs => fs.into_inner())
    }

    /// The format of the mounted filesystem. A bridge image mounted as UDF
    /// reports [`ImageFormat::Udf`].
    pub fn image_format(&self) -> ImageFormat {
        match self {
            Self::Fat(fs) => ImageFormat::Fat(fs.info().kind()),
            Self::ExFat(_) => ImageFormat::ExFat,
            Self::Iso(_) => ImageFormat::Iso,
            Self::Udf(_) => ImageFormat::Udf,
            Self::Apfs(_) => ImageFormat::Apfs,
        }
    }

    /// Maps `node` to the device with the format's own `extents`: fills
    /// `out` with the byte runs that hold its data from file offset `from`
    /// on, and returns how many it filled. Call again from the end of the
    /// last run for more; 0 means there are none. Fails with
    /// [`ErrorKind::Unsupported`] on APFS.
    pub async fn extents(&mut self, node: NodeId, from: u64, out: &mut [Extent]) -> FsResult<usize, D::Error> {
        match self {
            Self::Fat(fs) => fs.extents(node, from, out).await,
            Self::ExFat(fs) => fs.extents(node, from, out).await,
            Self::Iso(fs) => fs.extents(node, from, out).await,
            Self::Udf(fs) => fs.extents(node, from, out).await,
            Self::Apfs(_) => Err(ErrorKind::Unsupported.into()),
        }
    }

    /// The volume serial number: the boot sector's on FAT and exFAT, and on
    /// UDF the serial the volume set identifier starts with. `None` when the
    /// volume records none. Fails with [`ErrorKind::Unsupported`] on ISO
    /// 9660 and APFS, which have no serial number.
    pub fn volume_serial(&self) -> FsResult<Option<u64>, D::Error> {
        match self {
            Self::Fat(fs) => Ok(fs.volume_serial().map(u64::from)),
            Self::ExFat(fs) => Ok(Some(u64::from(fs.volume_serial()))),
            Self::Udf(fs) => Ok(fs.info().volume_serial()),
            Self::Iso(_) | Self::Apfs(_) => Err(ErrorKind::Unsupported.into()),
        }
    }

    /// Sets the volume label, or removes it with `None`, with the format's
    /// own `set_label`. Fails with the error of the format's label type
    /// when `label` does not fit it, with [`ErrorKind::ReadOnly`] on a
    /// read-only mount, and with [`ErrorKind::Unsupported`] on ISO 9660,
    /// UDF and APFS.
    pub async fn set_label(&mut self, label: Option<&str>) -> FsResult<(), D::Error> {
        match self {
            Self::Fat(fs) => {
                let label = label.map(hadris_fat::VolumeLabel::new).transpose()?;
                fs.set_label(label).await
            }
            Self::ExFat(fs) => {
                let label = label.map(hadris_fat::exfat::VolumeLabel::new).transpose()?;
                fs.set_label(label).await
            }
            Self::Iso(_) | Self::Udf(_) | Self::Apfs(_) => Err(ErrorKind::Unsupported.into()),
        }
    }
}

/// Detects what `dev` holds and mounts the first filesystem found with
/// `options`: a bridge image as UDF, falling back to ISO 9660.
///
/// Fails with the mount error of the first filesystem found when none
/// mounts (`Corrupt` where the driver says `NotRecognized`, as in
/// [`detect`]), and with [`ErrorKind::NotRecognized`] for a device holding
/// no filesystem: a partition table (mount its partitions with
/// [`open_partition`]), an archive, nothing recognized, or an NTFS volume
/// (message `"ntfs"`), which [`detect`] lists but `open` does not reach in
/// 3.0. APFS mounts a container's sole volume; containers with
/// multiple volumes require explicit selection with `open_apfs` instead.
/// The [`MountError`] gives `dev` back.
#[cfg(feature = "alloc")]
pub async fn open<D: BlockDevice>(mut dev: D, options: MountOptions) -> Result<AnyFs<D>, MountError<D, D::Error>> {
    let found = match detect(&mut dev).await {
        Ok(found) => found,
        Err(err) => return Err(MountError::new(err, dev)),
    };
    let mut failed = None;
    let mut other = None;
    for candidate in found.iter() {
        let mounted = match candidate.format() {
            ImageFormat::Fat(_) => FatFs::mount(dev, options).await.map(AnyFs::Fat),
            ImageFormat::ExFat => ExFatFs::mount(dev, options).await.map(AnyFs::ExFat),
            ImageFormat::Iso => IsoFs::mount(dev, options).await.map(AnyFs::Iso),
            ImageFormat::Udf | ImageFormat::IsoUdfBridge => UdfFs::mount(dev, options).await.map(AnyFs::Udf),
            ImageFormat::Apfs => ApfsFs::mount(dev, options).await.map(AnyFs::Apfs),
            ImageFormat::Ntfs => {
                other.get_or_insert("ntfs");
                continue;
            }
            ImageFormat::Mbr | ImageFormat::Gpt => {
                other.get_or_insert("partition table");
                continue;
            }
            ImageFormat::Cpio(_) => {
                other.get_or_insert("archive");
                continue;
            }
        };
        match mounted {
            Ok(fs) => return Ok(fs),
            Err(err) => {
                let (err, back) = err.into_parts();
                if err.kind() == ErrorKind::Io {
                    return Err(MountError::new(err, back));
                }
                failed.get_or_insert(damaged(&err));
                dev = back;
            }
        }
    }
    let err = failed.unwrap_or_else(|| {
        hadris_fs::Error::new(ErrorKind::NotRecognized, other.unwrap_or("no filesystem recognized"))
    });
    Err(MountError::new(err, dev))
}

/// Reads the partition table of `dev` and detects what each partition
/// holds, as [`detect`] does for a whole device, in the order the table
/// lists them. A partition that does not fit on `dev` has an empty
/// [`Detection`].
///
/// Fails as `hadris_part`'s `read` does: with [`ErrorKind::NotRecognized`]
/// when `dev` has no partition table, [`ErrorKind::Corrupt`] when the
/// table is invalid, and with the device's errors.
#[cfg(feature = "alloc")]
pub async fn detect_partitions<D: BlockDevice>(dev: &mut D) -> FsResult<Vec<(Partition, Detection)>, D::Error> {
    let disk = part_read(&mut *dev).await?;
    let mut found = Vec::new();
    for partition in disk.partitions() {
        let detection = match part_open(&mut *dev, &partition) {
            Ok(mut window) => detect(&mut window).await?,
            Err(_) => Detection::new(),
        };
        found.push((partition, detection));
    }
    Ok(found)
}

/// Reads the partition table of `dev` and mounts the filesystem of the
/// partition `select` names with `options`, as [`open`] mounts a whole
/// device. The filesystem's device is the partition's window of `dev`.
///
/// Fails as `hadris_part`'s `read` does when `dev` has no valid partition
/// table, with [`ErrorKind::NotFound`] when no partition has the index or
/// GUID asked for, and with the error of [`open`] for the selected
/// partition. [`Select::First`] skips partitions holding no filesystem and
/// partitions that do not fit on `dev`. It fails with the error of the
/// first partition holding a filesystem that does not mount; when none
/// holds one, with the [`ErrorKind::InvalidInput`] of the first partition
/// that does not fit, as on a truncated image, and otherwise with
/// [`ErrorKind::NotRecognized`]. The [`MountError`] gives `dev` back.
#[cfg(feature = "alloc")]
pub async fn open_partition<D: BlockDevice>(
    mut dev: D,
    select: Select,
    options: MountOptions,
) -> Result<AnyFs<hadris_storage::Partition<D>>, MountError<D, D::Error>> {
    let disk = match part_read(&mut dev).await {
        Ok(disk) => disk,
        Err(err) => return Err(MountError::new(err, dev)),
    };
    let first = select == Select::First;
    let mut failed = None;
    let mut unreachable = None;
    for partition in disk.partitions() {
        let chosen = match select {
            Select::Index(slot) => partition.slot() == slot,
            Select::Guid(guid) => partition.unique_guid() == Some(guid),
            Select::First => true,
        };
        if !chosen {
            continue;
        }
        let (offset, len) = match part_open(&mut dev, &partition) {
            Ok(window) => (window.offset(), window.len()),
            Err(err) if first => {
                unreachable.get_or_insert(err);
                continue;
            }
            Err(err) => return Err(MountError::new(err, dev)),
        };
        let window = hadris_storage::Partition::new(dev, offset, len);
        let err = match open(window, options).await {
            Ok(fs) => return Ok(fs),
            Err(err) => err,
        };
        let (err, back) = err.into_parts();
        dev = back.into_inner();
        if !first || err.kind() == ErrorKind::Io {
            return Err(MountError::new(err, dev));
        }
        if err.kind() != ErrorKind::NotRecognized {
            failed.get_or_insert(err);
        }
    }
    let err = failed.or(unreachable).unwrap_or_else(|| match select {
        Select::First => hadris_fs::Error::new(ErrorKind::NotRecognized, "no partition holds a filesystem"),
        _ => hadris_fs::Error::new(ErrorKind::NotFound, "no such partition"),
    });
    Err(MountError::new(err, dev))
}

/// Mounts an APFS volume selected by index, object id, UUID or name.
///
/// Use this for containers with multiple volumes; [`open`] never chooses
/// an arbitrary volume. Mount errors return ownership of the device.
#[cfg(all(feature = "alloc", feature = "unstable-apfs"))]
pub async fn open_apfs<D: BlockDevice>(
    dev: D,
    options: MountOptions,
    volume: crate::apfs::VolumeSelector<'_>,
) -> Result<AnyFs<D>, MountError<D, D::Error>> {
    ApfsFs::mount_volume(dev, options, volume).await.map(AnyFs::Apfs)
}

/// Writes an empty filesystem of the format `options` names to `dev`, so
/// that [`open`] mounts it: FAT and exFAT with `hadris_fat`'s `format`, and
/// UDF with its tree writer and an empty tree. Call the format's own
/// function for its result, such as the FAT geometry.
#[cfg(all(feature = "alloc", feature = "write"))]
pub async fn format<D: BlockDevice>(dev: &mut D, options: &crate::FormatOptions) -> Result<(), crate::PathError> {
    match options {
        crate::FormatOptions::Fat(options) => super::fat_format(dev, options).await.map(drop)?,
        crate::FormatOptions::ExFat(options) => super::exfat_format(dev, options).await.map(drop)?,
        crate::FormatOptions::Udf(options) => {
            super::udf_write(&mut *dev, &hadris_fs::Tree::new(), options).await.map(drop)?
        }
    }
    Ok(())
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice> FileSystem for AnyFs<D> {
    type DeviceError = D::Error;

    fn capabilities(&self) -> Capabilities {
        each!(self, fs => FileSystem::capabilities(fs))
    }
    fn root(&self) -> NodeId {
        each!(self, fs => FileSystem::root(fs))
    }
    async fn statfs(&mut self) -> FsResult<FsStats, Self::DeviceError> {
        each!(self, fs => FileSystem::statfs(fs).await)
    }
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, Self::DeviceError> {
        each!(self, fs => FileSystem::label(fs, buf).await)
    }
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => FileSystem::lookup(fs, dir, name).await)
    }
    fn forget(&mut self, node: NodeId, count: u64) {
        each!(self, fs => FileSystem::forget(fs, node, count))
    }
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => FileSystem::parent(fs, dir).await)
    }
    async fn resolve(&mut self, path: &[u8], how: Resolve) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => FileSystem::resolve(fs, path, how).await)
    }
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
        each!(self, fs => FileSystem::stat(fs, node).await)
    }
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, Self::DeviceError> {
        each!(self, fs => FileSystem::readdir(fs, dir, from).await)
    }
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], Self::DeviceError> {
        each!(self, fs => FileSystem::readlink(fs, node, buf).await)
    }
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::open(fs, node, mode).await)
    }
    async fn close(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::close(fs, node).await)
    }
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        each!(self, fs => FileSystem::read(fs, node, offset, buf).await)
    }
    async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::setattr(fs, node, changes).await)
    }
    async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
        each!(self, fs => FileSystem::write(fs, node, offset, buf).await)
    }
    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::truncate(fs, node, len).await)
    }
    async fn fsync(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::fsync(fs, node).await)
    }
    async fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => FileSystem::create(fs, dir, name, attrs).await)
    }
    async fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => FileSystem::mkdir(fs, dir, name, attrs).await)
    }
    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::unlink(fs, dir, name).await)
    }
    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::rmdir(fs, dir, name).await)
    }
    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::rename(fs, from_dir, from, to_dir, to, mode).await)
    }
    async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => FileSystem::sync(fs).await)
    }
}

send_filesystem! {
#[cfg(feature = "alloc")]
impl<D: hadris_storage::async_::SendBlockDevice> hadris_fs::async_::FileSystem for AnyFs<D> {
    type DeviceError = D::Error;

    fn capabilities(&self) -> Capabilities {
        each!(self, fs => hadris_fs::async_::FileSystem::capabilities(fs))
    }
    fn root(&self) -> NodeId {
        each!(self, fs => hadris_fs::async_::FileSystem::root(fs))
    }
    async fn statfs(&mut self) -> FsResult<FsStats, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::statfs(fs).await)
    }
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::label(fs, buf).await)
    }
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::lookup(fs, dir, name).await)
    }
    fn forget(&mut self, node: NodeId, count: u64) {
        each!(self, fs => hadris_fs::async_::FileSystem::forget(fs, node, count))
    }
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::parent(fs, dir).await)
    }
    async fn resolve(&mut self, path: &[u8], how: Resolve) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::resolve(fs, path, how).await)
    }
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::stat(fs, node).await)
    }
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::readdir(fs, dir, from).await)
    }
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::readlink(fs, node, buf).await)
    }
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::open(fs, node, mode).await)
    }
    async fn close(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::close(fs, node).await)
    }
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::read(fs, node, offset, buf).await)
    }
    async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::setattr(fs, node, changes).await)
    }
    async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::write(fs, node, offset, buf).await)
    }
    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::truncate(fs, node, len).await)
    }
    async fn fsync(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::fsync(fs, node).await)
    }
    async fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::create(fs, dir, name, attrs).await)
    }
    async fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::mkdir(fs, dir, name, attrs).await)
    }
    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::unlink(fs, dir, name).await)
    }
    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::rmdir(fs, dir, name).await)
    }
    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::rename(fs, from_dir, from, to_dir, to, mode).await)
    }
    async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
        each!(self, fs => hadris_fs::async_::FileSystem::sync(fs).await)
    }
}
}


}
