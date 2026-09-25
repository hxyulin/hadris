#[cfg(feature = "alloc")]
use hadris_fs::{
    Capabilities, DirCursor, DirEntry, FsStats, Metadata, Name, NodeId, OpenMode, RenameMode,
    Resolve, SetAttr,
};
use hadris_fs::{ErrorKind, FsResult, MountError, MountOptions};
use hadris_storage::BlockIndex;

use crate::detect::{
    Detection, ImageFormat, Optical, Table, cpio_format, fat_kind, partition_table,
};
use hadris_fat_raw::io::BlockBuf;

use super::{BlockDevice, IsoFs, UdfFs, exio, rawio};
#[cfg(feature = "alloc")]
use super::{ExFatFs, FatFs, FileSystem};

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
/// table or a cpio archive. It reads a few blocks, never writes and does
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
    if let (Some(_), Some(udf)) = (iso, udf) {
        found.push(ImageFormat::IsoUdfBridge, udf);
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
    if signed && &sector[3..11] == b"NTFS    " {
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
}

/// Detects what `dev` holds and mounts the first filesystem found with
/// `options`: a bridge image as UDF, falling back to ISO 9660.
///
/// Fails with the mount error of the first filesystem found when none
/// mounts (`Corrupt` where the driver says `NotRecognized`, as in
/// [`detect`]), and with [`ErrorKind::NotRecognized`] for a device holding
/// no filesystem: a partition table, an archive, nothing recognized, or
/// an NTFS volume (message `"ntfs"`), which [`detect`] lists but `open`
/// does not reach in 3.0. The [`MountError`] gives `dev` back.
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

}
