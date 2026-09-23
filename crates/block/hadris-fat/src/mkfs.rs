use hadris_fs::{Clock, DateTime, ErrorKind, FixedTable, FsResult};

use super::fatfs::{BlockBuf, FatFs, MAX_BLOCK_SIZE, write_bytes};
use super::storage::BlockDevice;
use crate::codec::date;
use crate::codec::dirent::{ATTR_VOLUME_ID, ShortEntry};
use crate::codec::entry::FatKind;
use crate::codec::layout::{
    self, BACKUP_BOOT_SECTOR, BootFields, FS_INFO_SECTOR, Layout, LayoutError, ROOT_CLUSTER,
    Request,
};
use crate::{FormatOptions, MountOptions};

fn layout_error(err: LayoutError) -> ErrorKind {
    match err {
        LayoutError::TooSmall => ErrorKind::NoSpace,
        LayoutError::TooLarge => ErrorKind::LimitExceeded,
        LayoutError::Invalid(_) => ErrorKind::InvalidInput,
    }
}

fn volume_id(now: DateTime) -> u32 {
    let seconds = now.unix_seconds() as u64;
    (seconds as u32) ^ ((seconds >> 32) as u32) ^ now.nanoseconds().rotate_left(16)
}

io_transform! {

/// Formats `dev` as a FAT12, FAT16 or FAT32 volume that fills it, and mounts
/// it with the default node table and code page and the options' clock.
///
/// The volume uses every whole sector of the device; pass a
/// `hadris_storage` `Slice` to format a partition. Everything before the
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
/// interrupted, leaves a device that does not mount.
pub async fn format<D: BlockDevice, C: Clock>(
    mut dev: D,
    options: FormatOptions<C>,
) -> FsResult<FatFs<D, FixedTable<64>, C>, D::Error> {
    let block_size = dev.block_size().get() as usize;
    if block_size > MAX_BLOCK_SIZE {
        return Err(ErrorKind::Unsupported.into());
    }
    if !matches!(options.media, 0xF0 | 0xF8..=0xFF) {
        return Err(ErrorKind::InvalidInput.into());
    }
    let sector_size = options.sector_size.unwrap_or(match block_size {
        512 | 1024 | 2048 | 4096 => block_size as u32,
        _ => 512,
    });
    let device_bytes = dev.block_count().saturating_mul(block_size as u64);
    let layout = layout::plan(&Request {
        kind: options.kind,
        sector_size,
        total_sectors: device_bytes / sector_size.max(1) as u64,
        cluster_size: options.cluster_size,
        reserved_sectors: options.reserved_sectors,
        fat_count: options.fat_count,
        root_entries: options.root_entries,
    })
    .map_err(layout_error)?;
    let now = options.clock.now();
    let fields = BootFields {
        oem_name: options.oem_name,
        media: options.media,
        hidden_sectors: options.hidden_sectors,
        volume_id: options.volume_id.unwrap_or_else(|| volume_id(now)),
        label: options.label.map(|label| *label.as_bytes()),
    };
    let mut block = BlockBuf::new(block_size);
    write_layout(&mut dev, &mut block, &layout, &fields, now).await?;
    dev.flush().await?;
    Ok(FatFs::open_with(dev, MountOptions::new().with_clock(options.clock)).await?)
}

async fn write_layout<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    layout: &Layout,
    fields: &BootFields,
    now: DateTime,
) -> FsResult<(), D::Error> {
    let sector = layout.sector_size as u64;
    let data_start = layout.data_start();
    let root = if layout.kind == FatKind::Fat32 {
        data_start
    } else {
        layout.root_start()
    };
    let root_end = if layout.kind == FatKind::Fat32 {
        data_start + layout.cluster_size() as u64
    } else {
        data_start
    };
    write_bytes(dev, block, 0, None, root_end as usize).await?;

    let (entries, len) = layout::reserved_fat_entries(layout.kind, fields.media);
    for copy in 0..layout.fat_count as u64 {
        let at = layout.fat_start() + copy * layout.fat_sectors as u64 * sector;
        write_bytes(dev, block, at, Some(&entries[..len]), len).await?;
    }
    if let Some(label) = fields.label {
        let mut entry = ShortEntry::new(label, ATTR_VOLUME_ID);
        let (date, time, tenths) = date::encode(now);
        (entry.created_date, entry.created_time, entry.created_tenths) = (date, time, tenths);
        (entry.modified_date, entry.modified_time) = (date, time);
        entry.accessed_date = date;
        write_bytes(dev, block, root, Some(&entry.encode()), 32).await?;
    }

    let boot = layout::encode_boot_sector(layout, fields);
    if layout.kind == FatKind::Fat32 {
        let free = layout.clusters - 1;
        let info = layout::encode_fs_info(free, ROOT_CLUSTER + 1);
        let backup = BACKUP_BOOT_SECTOR as u64 * sector;
        write_bytes(dev, block, backup, Some(&boot), boot.len()).await?;
        write_bytes(dev, block, backup + sector, Some(&info), info.len()).await?;
        write_bytes(dev, block, FS_INFO_SECTOR as u64 * sector, Some(&info), info.len()).await?;
    }
    write_bytes(dev, block, 0, Some(&boot), boot.len()).await
}

}
