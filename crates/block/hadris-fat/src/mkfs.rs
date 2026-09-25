use hadris_fs::{ErrorKind, FsResult};

use super::block_io::new_block;
use super::rawio;
use super::storage::BlockDevice;
use hadris_fat_raw::Geometry;
use hadris_fat_raw::layout::{self, BootFields, Layout, LayoutError, Request};

use crate::FatOptions;
use crate::options::serial;

fn layout_error(err: LayoutError) -> ErrorKind {
    match err {
        LayoutError::TooSmall => ErrorKind::NoSpace,
        LayoutError::TooLarge => ErrorKind::LimitExceeded,
        LayoutError::Invalid(_) => ErrorKind::InvalidInput,
        _ => ErrorKind::InvalidInput,
    }
}

/// The bytes of the volume `options` asks for on a device of `block_count`
/// blocks of `block_size` bytes that can grow to `max_block_count`.
pub(crate) fn volume_bytes(
    size: Option<u64>,
    block_size: u64,
    block_count: u64,
    max_block_count: u64,
) -> Result<u64, ErrorKind> {
    let device = block_count.saturating_mul(block_size);
    match size {
        None => Ok(device),
        Some(size) if size <= device || size <= max_block_count.saturating_mul(block_size) => {
            Ok(size)
        }
        Some(_) => Err(ErrorKind::NoSpace),
    }
}

/// The sectors before the volume on its disk: `offset` bytes, or the
/// device's `disk_offset`.
pub(crate) fn offset_sectors(offset: u64, sector_size: u64) -> Result<u64, ErrorKind> {
    if offset % sector_size != 0 {
        return Err(ErrorKind::InvalidInput);
    }
    Ok(offset / sector_size)
}

/// Plans the layout, adding reserved sectors until the data region starts
/// on a multiple of `options.alignment`.
fn plan(request: Request, sector_size: u32, options: &FatOptions) -> Result<Layout, ErrorKind> {
    let mut layout = layout::plan(&request).map_err(layout_error)?;
    let Some(align) = options.alignment else {
        return Ok(layout);
    };
    if !align.is_power_of_two() || align < sector_size {
        return Err(ErrorKind::InvalidInput);
    }
    let align = align as u64;
    for _ in 0..8 {
        let short = (align - layout.data_start() % align) % align;
        if short == 0 {
            return Ok(layout);
        }
        let reserved = layout.reserved_sectors() as u64 + short / sector_size as u64;
        let reserved = u16::try_from(reserved).map_err(|_| ErrorKind::LimitExceeded)?;
        layout = layout::plan(&request.with_reserved_sectors(reserved)).map_err(layout_error)?;
    }
    Err(ErrorKind::LimitExceeded)
}

io_transform! {

/// Formats `dev` as a FAT12, FAT16 or FAT32 volume and returns its
/// geometry. Needs no allocator; mount the volume with
/// [`FatFs::mount`](super::FatFs::mount) and the options of your choice.
///
/// The volume fills the device unless [`FatOptions::with_size`] asks for
/// another size; pass a `hadris_storage` `Partition` to format a partition.
/// Everything before the data region is written: the boot sector, on FAT32
/// the FSInfo sector and their backups, the FATs, the root directory and
/// its label entry. The data region is left as it is, except that a
/// growable device is grown to the volume's end. The device is flushed.
///
/// Fails with [`ErrorKind::InvalidInput`] when an option is out of range,
/// with [`ErrorKind::NoSpace`] when the device is too small for the size,
/// any layout or the variant asked for, with [`ErrorKind::LimitExceeded`]
/// when the volume is too large for that variant or has more than
/// `u32::MAX` sectors, with [`ErrorKind::Unsupported`] when the device's
/// blocks are larger than 4096 bytes, and with [`ErrorKind::ReadOnly`] when
/// it refuses writes. Nothing is written unless the options are valid; a
/// format that fails later, or is interrupted, leaves a device that does
/// not mount.
pub async fn format<D: BlockDevice>(dev: &mut D, options: &FatOptions) -> FsResult<Geometry, D::Error> {
    Ok(format_volume(dev, options).await?.0)
}

/// [`format`], also returning the volume's length in bytes.
pub(crate) async fn format_volume<D: BlockDevice>(
    dev: &mut D,
    options: &FatOptions,
) -> FsResult<(Geometry, u64), D::Error> {
    let block_size = dev.block_size().get() as usize;
    let mut block = new_block(block_size)?;
    if !matches!(options.media, 0xF0 | 0xF8..=0xFF) {
        return Err(ErrorKind::InvalidInput.into());
    }
    let sector_size = options.sector_size.unwrap_or(match block_size {
        512 | 1024 | 2048 | 4096 => block_size as u32,
        _ => 512,
    });
    let bytes = volume_bytes(options.size, block_size as u64, dev.block_count(), dev.max_block_count())?;
    let mut request = Request::new(sector_size, bytes / sector_size.max(1) as u64)
        .with_fat_count(options.fat_count)
        .with_root_entries(options.root_entries);
    if let Some(kind) = options.kind {
        request = request.with_kind(kind);
    }
    if let Some(bytes) = options.cluster_size {
        request = request.with_cluster_size(bytes);
    }
    if let Some(sectors) = options.reserved_sectors {
        request = request.with_reserved_sectors(sectors);
    }
    let layout = plan(request, sector_size, options)?;
    let hidden = offset_sectors(options.partition_offset.unwrap_or(dev.disk_offset()), sector_size as u64)?;
    let hidden = u32::try_from(hidden).map_err(|_| ErrorKind::LimitExceeded)?;
    let mut fields = BootFields::new()
        .with_oem_name(options.oem_name)
        .with_media(options.media)
        .with_hidden_sectors(hidden)
        .with_volume_id(options.serial.unwrap_or_else(|| serial(options.time, options.seed)));
    if let Some(label) = options.label {
        fields = fields.with_label(*label.as_bytes());
    }
    let volume = layout.total_sectors() as u64 * sector_size as u64;
    let geometry = rawio::mkfs(dev, &mut block, &layout, &fields, options.time).await?;
    grow(dev, &mut block, volume).await?;
    dev.flush().await?;
    Ok((geometry, volume))
}

/// Grows a device shorter than `volume` bytes by writing its last block.
pub(crate) async fn grow<D: BlockDevice>(
    dev: &mut D,
    block: &mut hadris_fat_raw::io::BlockBuf,
    volume: u64,
) -> FsResult<(), D::Error> {
    let size = dev.block_size().get() as u64;
    if dev.block_count().saturating_mul(size) < volume {
        let last = volume.div_ceil(size) * size - size;
        rawio::write_zeros(dev, block, last, size as usize).await?;
    }
    Ok(())
}

}

#[cfg(feature = "alloc")]
mod tree {
    use alloc::collections::BTreeMap;
    use alloc::vec;
    use alloc::vec::Vec;

    use hadris_fs::{DateTime, FileType, MountOptions, Node, PathError, Report, SetAttr, Tree};

    use super::super::FatFs;
    use super::super::fsapi::{ContentReader, FileSystem, copy_tree};
    use super::super::storage::BlockDevice;
    use super::format_volume;
    use crate::FatOptions;

    /// The seed a volume written from `tree` derives its serial from: the
    /// options' seed, or their time, mixed with the tree's fingerprint.
    pub(crate) fn tree_seed(time: DateTime, seed: Option<u64>, tree: &Tree) -> u64 {
        let base =
            seed.unwrap_or((time.unix_seconds() as u64) ^ (u64::from(time.nanoseconds()) << 32));
        base.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ tree.fingerprint()
    }

    /// `tree` with `time` for each time its nodes leave unset, after
    /// checking that this mode reads every file.
    pub(crate) fn stamped(tree: &Tree, time: DateTime) -> Result<Tree, PathError> {
        let stamp = |attrs: &SetAttr| {
            let mut attrs = *attrs;
            if attrs.created().is_none() {
                attrs = attrs.with_created(time);
            }
            if attrs.modified().is_none() {
                attrs = attrs.with_modified(time);
            }
            if attrs.accessed().is_none() {
                attrs = attrs.with_accessed(time);
            }
            attrs
        };
        let mut out = Tree::new();
        out.replace(
            "",
            Node::dir().with_attrs(stamp(tree.root().node().attrs())),
        )?;
        let mut first: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        let mut pending = vec![(tree.root(), Vec::new())];
        while let Some((dir, prefix)) = pending.pop() {
            for (name, child) in dir.children() {
                let mut path = prefix.clone();
                if !path.is_empty() {
                    path.push(b'/');
                }
                path.extend_from_slice(name.as_bytes());
                if let Some(target) = first.get(&child.id()) {
                    out.link(target, &path)?;
                    continue;
                }
                let node = child.node();
                if let Some(content) = node.content() {
                    ContentReader::check(content).map_err(|err| err.with_path(&path))?;
                }
                out.insert(&path, node.clone().with_attrs(stamp(node.attrs())))?;
                if node.file_type() == FileType::Dir {
                    pending.push((child, path.clone()));
                }
                if child.links() > 1 {
                    first.insert(child.id(), path);
                }
            }
        }
        Ok(out)
    }

    io_transform! {

    /// Formats `out` as [`format`](super::format) does and copies `tree`
    /// into the new volume with `copy_tree`, then unmounts it.
    ///
    /// Nodes without times get the options' time, so the same tree and
    /// options give the same bytes. The serial derives from the tree's
    /// paths, sizes and times together with the seed, or the time without
    /// one, unless [`FatOptions::with_serial`] sets it. A growable device
    /// such as `Vec<u8>` needs [`FatOptions::with_size`], since it starts empty. Every file's
    /// content is checked to be readable in this mode before anything is
    /// written. The report has the volume's size and the warnings of
    /// `copy_tree`: symlinks, special files and extra hard-link names are
    /// skipped, and fields FAT does not store are dropped. Fails as
    /// `format` and `copy_tree` do, with the tree path of the node that
    /// failed.
    pub async fn write<D: BlockDevice>(mut out: D, tree: &Tree, options: &FatOptions) -> Result<Report, PathError> {
        let options = options.with_seed(tree_seed(options.time, options.seed, tree));
        let tree = stamped(tree, options.time)?;
        let (_, volume) = format_volume(&mut out, &options).await?;
        let mut fs = FatFs::mount(out, MountOptions::new()).await?;
        let root = fs.root();
        let mut report = copy_tree(&tree, &mut fs, root).await?;
        fs.unmount().await?;
        report.set_size(volume);
        Ok(report)
    }

    }
}

#[cfg(feature = "alloc")]
pub use tree::write;
#[cfg(feature = "alloc")]
pub(crate) use tree::{stamped, tree_seed};
