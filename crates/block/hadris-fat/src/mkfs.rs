use hadris_fs::{DateTime, ErrorKind, FsResult, MountError, MountOptions};

use super::block_io::new_block;
use super::fatfs::FatFs;
use super::rawio;
use super::storage::BlockDevice;
use hadris_fat_raw::layout::{self, BootFields, LayoutError, Request};

use crate::FormatOptions;

fn layout_error(err: LayoutError) -> ErrorKind {
    match err {
        LayoutError::TooSmall => ErrorKind::NoSpace,
        LayoutError::TooLarge => ErrorKind::LimitExceeded,
        LayoutError::Invalid(_) => ErrorKind::InvalidInput,
        _ => ErrorKind::InvalidInput,
    }
}

fn volume_id(now: DateTime) -> u32 {
    let seconds = now.unix_seconds() as u64;
    (seconds as u32) ^ ((seconds >> 32) as u32) ^ now.nanoseconds().rotate_left(16)
}

io_transform! {

/// Formats `dev` as a FAT12, FAT16 or FAT32 volume that fills it, and mounts
/// it with the default [`MountOptions`] and the options' clock.
///
/// The volume uses every whole sector of the device; pass a
/// `hadris_storage` `Partition` to format a partition. Everything before the
/// data region is written: the boot sector, on FAT32 the FSInfo sector and
/// their backups, the FATs, the root directory and its label entry. The
/// data region is left as it is. The device is flushed before the volume is
/// mounted; use `into_inner` to mount it with other [`MountOptions`].
///
/// Fails with [`ErrorKind::InvalidInput`] when an option is out of range,
/// with [`ErrorKind::NoSpace`] when the device is too small for any layout
/// or the variant asked for, with [`ErrorKind::LimitExceeded`] when it is
/// too large for that variant or has more than `u32::MAX` sectors, with
/// [`ErrorKind::Unsupported`] when its blocks are larger than 4096 bytes,
/// and with [`ErrorKind::ReadOnly`] when it refuses writes. Nothing is
/// written unless the options are valid; a format that fails later, or is
/// interrupted, leaves a device that does not mount. On any failure,
/// including the final mount, the [`MountError`] gives `dev` back.
pub async fn format<D: BlockDevice>(
    mut dev: D,
    options: FormatOptions,
) -> Result<FatFs<D>, MountError<D, D::Error>> {
    if let Err(error) = write_volume(&mut dev, &options).await {
        return Err(MountError::new(error, dev));
    }
    FatFs::mount(dev, MountOptions::new().with_clock(options.clock)).await
}

async fn write_volume<D: BlockDevice>(
    dev: &mut D,
    options: &FormatOptions,
) -> FsResult<(), D::Error> {
    let block_size = dev.block_size().get() as usize;
    let mut block = new_block(block_size)?;
    if !matches!(options.media, 0xF0 | 0xF8..=0xFF) {
        return Err(ErrorKind::InvalidInput.into());
    }
    let sector_size = options.sector_size.unwrap_or(match block_size {
        512 | 1024 | 2048 | 4096 => block_size as u32,
        _ => 512,
    });
    let device_bytes = dev.block_count().saturating_mul(block_size as u64);
    let mut request = Request::new(sector_size, device_bytes / sector_size.max(1) as u64)
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
    let layout = layout::plan(&request).map_err(layout_error)?;
    let now = options.clock.now();
    let mut fields = BootFields::new()
        .with_oem_name(options.oem_name)
        .with_media(options.media)
        .with_hidden_sectors(options.hidden_sectors)
        .with_volume_id(options.volume_id.unwrap_or_else(|| volume_id(now)));
    if let Some(label) = options.label {
        fields = fields.with_label(*label.as_bytes());
    }
    rawio::mkfs(dev, &mut block, &layout, &fields, now).await?;
    dev.flush().await?;
    Ok(())
}

}
