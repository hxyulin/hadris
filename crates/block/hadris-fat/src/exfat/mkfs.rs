use hadris_common::types::endian::LittleEndian;
use hadris_common::types::number::{U16, U32, U64};
use hadris_fs::{Clock, DateTime, ErrorKind, FixedTable, FsResult, MountError};

use super::block_io::{new_block, write_bytes};
use super::fs::ExFatFs;
use super::storage::BlockDevice;
use hadris_fat_raw::exfat::{self as raw, BootSector, ENTRY_SIZE, RawEntry};

use crate::exfat::{FormatOptions, MountOptions};

/// The smallest volume `format` lays out.
const MIN_VOLUME: u64 = 1 << 20;
/// The largest cluster size.
const MAX_CLUSTER: u32 = 32 << 20;
/// Bytes written at once when filling the bitmap.
const CHUNK: usize = 512;

/// Where `format` puts the volume's structures, in sectors and clusters.
#[derive(Debug, Clone, Copy)]
struct Layout {
    sector_shift: u8,
    cluster_shift: u8,
    volume_sectors: u64,
    fat_offset: u32,
    fat_length: u32,
    heap_offset: u32,
    cluster_count: u32,
    fats: u8,
    bitmap_clusters: u32,
    upcase_clusters: u32,
}

impl Layout {
    fn sector(&self) -> u64 {
        1 << self.sector_shift
    }

    fn cluster(&self) -> u64 {
        1 << self.cluster_shift
    }

    fn cluster_at(&self, cluster: u32) -> u64 {
        ((self.heap_offset as u64) << self.sector_shift)
            + (((cluster - raw::FIRST_CLUSTER) as u64) << self.cluster_shift)
    }

    fn bitmap_len(&self) -> u64 {
        (self.cluster_count as u64).div_ceil(8)
    }

    fn bitmap_first(&self, index: u8) -> u32 {
        raw::FIRST_CLUSTER + self.bitmap_clusters * index as u32
    }

    fn upcase_first(&self) -> u32 {
        self.bitmap_first(self.fats)
    }

    fn root(&self) -> u32 {
        self.upcase_first() + self.upcase_clusters
    }

    /// Clusters the empty volume uses.
    fn used(&self) -> u32 {
        self.bitmap_clusters * self.fats as u32 + self.upcase_clusters + 1
    }
}

fn shift_of(value: u64) -> Option<u8> {
    value
        .is_power_of_two()
        .then(|| value.trailing_zeros() as u8)
}

/// Plans a volume of `device_bytes`, or says why it cannot.
fn plan(
    device_bytes: u64,
    block_size: u32,
    options: &FormatOptions<impl Clock>,
) -> Result<Layout, ErrorKind> {
    let sector = options.sector_size.unwrap_or(match block_size {
        512 | 1024 | 2048 | 4096 => block_size,
        _ => 512,
    }) as u64;
    let fats = options.fat_count;
    if !matches!(sector, 512 | 1024 | 2048 | 4096) || !matches!(fats, 1 | 2) {
        return Err(ErrorKind::InvalidInput);
    }
    let sector_shift = shift_of(sector).ok_or(ErrorKind::InvalidInput)?;
    let volume_sectors = device_bytes >> sector_shift;
    let volume = volume_sectors << sector_shift;
    if volume < MIN_VOLUME {
        return Err(ErrorKind::NoSpace);
    }
    let cluster = options
        .cluster_size
        .map(u64::from)
        .unwrap_or(if volume < 256 << 20 {
            4 << 10
        } else if volume < 32 << 30 {
            32 << 10
        } else {
            128 << 10
        });
    let cluster_shift = shift_of(cluster).ok_or(ErrorKind::InvalidInput)?;
    if cluster < sector || cluster > MAX_CLUSTER as u64 {
        return Err(ErrorKind::InvalidInput);
    }
    let align = options
        .alignment
        .map(u64::from)
        .unwrap_or(if volume >= 64 << 20 { 1 << 20 } else { cluster });
    if !align.is_power_of_two() || align < sector {
        return Err(ErrorKind::InvalidInput);
    }
    let align_sectors = align >> sector_shift;
    let per_cluster = cluster_shift - sector_shift;
    let fat_offset = 24u64.next_multiple_of(align_sectors);
    let mut count = (volume_sectors >> per_cluster).min(raw::MAX_CLUSTER_COUNT as u64);
    let (fat_length, heap_offset) = loop {
        let fat_length = ((count + 2) * 4).div_ceil(sector);
        let heap_offset = (fat_offset + fat_length * fats as u64).next_multiple_of(align_sectors);
        let fits = volume_sectors.saturating_sub(heap_offset) >> per_cluster;
        if fits >= count {
            break (fat_length, heap_offset);
        }
        count = fits;
    };
    let fat_offset = u32::try_from(fat_offset).map_err(|_| ErrorKind::LimitExceeded)?;
    let fat_length = u32::try_from(fat_length).map_err(|_| ErrorKind::LimitExceeded)?;
    let heap_offset = u32::try_from(heap_offset).map_err(|_| ErrorKind::LimitExceeded)?;
    let count = count as u32;
    let bitmap_clusters = (count as u64).div_ceil(8).div_ceil(cluster) as u32;
    let upcase_clusters = (raw::RECOMMENDED_UPCASE_TABLE.len() as u64).div_ceil(cluster) as u32;
    if count < bitmap_clusters * fats as u32 + upcase_clusters + 2 {
        return Err(ErrorKind::NoSpace);
    }
    Ok(Layout {
        sector_shift,
        cluster_shift,
        volume_sectors,
        fat_offset,
        fat_length,
        heap_offset,
        cluster_count: count,
        fats,
        bitmap_clusters,
        upcase_clusters,
    })
}

fn volume_id(now: DateTime) -> u32 {
    let seconds = now.unix_seconds() as u64;
    (seconds as u32) ^ ((seconds >> 32) as u32) ^ now.nanoseconds().rotate_left(16)
}

fn boot_sector(layout: &Layout, options: &FormatOptions<impl Clock>, serial: u32) -> BootSector {
    let mut boot: BootSector = bytemuck::Zeroable::zeroed();
    boot.jump_boot = raw::JUMP_BOOT;
    boot.file_system_name = raw::FILE_SYSTEM_NAME;
    boot.partition_offset = U64::<LittleEndian>::new(options.partition_offset);
    boot.volume_length = U64::<LittleEndian>::new(layout.volume_sectors);
    boot.fat_offset = U32::<LittleEndian>::new(layout.fat_offset);
    boot.fat_length = U32::<LittleEndian>::new(layout.fat_length);
    boot.cluster_heap_offset = U32::<LittleEndian>::new(layout.heap_offset);
    boot.cluster_count = U32::<LittleEndian>::new(layout.cluster_count);
    boot.first_cluster_of_root_directory = U32::<LittleEndian>::new(layout.root());
    boot.volume_serial_number = U32::<LittleEndian>::new(serial);
    boot.file_system_revision = U16::<LittleEndian>::new(0x0100);
    boot.bytes_per_sector_shift = layout.sector_shift;
    boot.sectors_per_cluster_shift = layout.cluster_shift - layout.sector_shift;
    boot.number_of_fats = layout.fats;
    boot.drive_select = 0x80;
    boot.percent_in_use = (layout.used() as u64 * 100 / layout.cluster_count as u64) as u8;
    boot.boot_code = [0xF4; 390];
    boot.boot_signature = U16::<LittleEndian>::new(raw::BOOT_SIGNATURE);
    boot
}

/// The root directory's first entries: the label, when there is one, the
/// Allocation Bitmaps and the Up-case Table.
fn root_entries(layout: &Layout, options: &FormatOptions<impl Clock>) -> ([RawEntry; 4], usize) {
    let mut entries = [[0u8; ENTRY_SIZE]; 4];
    let mut count = 0;
    if let Some(label) = &options.label {
        let entry = &mut entries[count];
        entry[0] = raw::ENTRY_LABEL;
        entry[1] = label.as_utf16().len() as u8;
        for (index, unit) in label.as_utf16().iter().enumerate() {
            entry[2 + index * 2..4 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        count += 1;
    }
    for index in 0..layout.fats {
        let bitmap = &mut entries[count];
        bitmap[0] = raw::ENTRY_BITMAP;
        bitmap[1] = index;
        bitmap[20..24].copy_from_slice(&layout.bitmap_first(index).to_le_bytes());
        bitmap[24..32].copy_from_slice(&layout.bitmap_len().to_le_bytes());
        count += 1;
    }
    let upcase = &mut entries[count];
    upcase[0] = raw::ENTRY_UPCASE;
    upcase[4..8].copy_from_slice(&raw::RECOMMENDED_UPCASE_CHECKSUM.to_le_bytes());
    upcase[20..24].copy_from_slice(&layout.upcase_first().to_le_bytes());
    upcase[24..32].copy_from_slice(&(raw::RECOMMENDED_UPCASE_TABLE.len() as u64).to_le_bytes());
    count += 1;
    (entries, count)
}

io_transform! {

/// Formats `dev` as an exFAT volume that fills it, and mounts it with the
/// default node table and the options' clock.
///
/// The volume uses every whole sector of the device; pass a
/// `hadris_storage` `Partition` to format a partition. Written are both boot
/// regions, the FATs, the allocation bitmaps, the recommended up-case table
/// and the root directory with its label, bitmap and up-case entries; the
/// rest of the cluster heap is left as it is. The boot sector is written
/// last, and the device is flushed before the volume is mounted; use
/// `into_inner` to mount it with other [`MountOptions`].
///
/// Fails with [`ErrorKind::InvalidInput`] when an option is out of range,
/// with [`ErrorKind::NoSpace`] when the device is smaller than 1 MiB or too
/// small for the clusters asked for, with [`ErrorKind::LimitExceeded`] when
/// the FAT or heap offset does not fit its field, with
/// [`ErrorKind::Unsupported`] when its blocks are larger than 4096 bytes,
/// and with [`ErrorKind::ReadOnly`] when it refuses writes. Nothing is
/// written unless the options are valid. On any failure, including the
/// final mount, the [`MountError`] gives `dev` back.
pub async fn format<D: BlockDevice, C: Clock>(
    mut dev: D,
    options: FormatOptions<C>,
) -> Result<ExFatFs<D, FixedTable<64>, C>, MountError<D, D::Error>> {
    if let Err(error) = write_volume(&mut dev, &options).await {
        return Err(MountError::new(error, dev));
    }
    ExFatFs::open_with(dev, MountOptions::new().with_clock(options.clock)).await
}

async fn write_volume<D: BlockDevice, C: Clock>(
    dev: &mut D,
    options: &FormatOptions<C>,
) -> FsResult<(), D::Error> {
    let block_size = dev.block_size().get() as usize;
    let mut block = new_block(block_size)?;
    let device_bytes = dev.block_count().saturating_mul(block_size as u64);
    let layout = plan(device_bytes, block_size as u32, options)?;
    let serial = options.volume_id.unwrap_or_else(|| volume_id(options.clock.now()));
    let block = &mut block;
    let sector = layout.sector();
    let region = raw::BOOT_REGION_SECTORS * sector;
    let zero_len = |len: u64| usize::try_from(len).map_err(|_| ErrorKind::LimitExceeded);

    write_bytes(dev, block, 0, None, zero_len(2 * region)?).await?;
    let fat_len = (layout.fat_length as u64) << layout.sector_shift;
    for copy in 0..layout.fats as u64 {
        let fat_start = ((layout.fat_offset as u64) << layout.sector_shift) + fat_len * copy;
        write_bytes(dev, block, fat_start, None, zero_len(fat_len)?).await?;
        let mut fat = [0u8; 8];
        fat[..4].copy_from_slice(&raw::FAT_MEDIA.to_le_bytes());
        fat[4..].copy_from_slice(&raw::FAT_END.to_le_bytes());
        write_bytes(dev, block, fat_start, Some(&fat), 8).await?;
        for (first, clusters) in [
            (raw::FIRST_CLUSTER, layout.bitmap_clusters * layout.fats as u32),
            (layout.upcase_first(), layout.upcase_clusters),
            (layout.root(), 1),
        ] {
            for cluster in first..first + clusters {
                let last = cluster + 1 == first + clusters
                    || (first == raw::FIRST_CLUSTER && (cluster + 1 - first) % layout.bitmap_clusters == 0);
                let next = if last { raw::FAT_END } else { cluster + 1 };
                write_bytes(dev, block, fat_start + cluster as u64 * 4, Some(&next.to_le_bytes()), 4).await?;
            }
        }
    }

    let used = layout.used() as usize;
    for copy in 0..layout.fats {
        let bitmap_at = layout.cluster_at(layout.bitmap_first(copy));
        write_bytes(dev, block, bitmap_at, None, zero_len(layout.bitmap_clusters as u64 * layout.cluster())?).await?;
        let mut chunk = [0u8; CHUNK];
        let mut done = 0;
        while done * 8 < used {
            let n = (used - done * 8).div_ceil(8).min(CHUNK);
            for (index, byte) in chunk[..n].iter_mut().enumerate() {
                let bits = (used - (done + index) * 8).min(8);
                *byte = if bits == 8 { 0xFF } else { (1u8 << bits) - 1 };
            }
            write_bytes(dev, block, bitmap_at + done as u64, Some(&chunk[..n]), n).await?;
            done += n;
        }
    }
    let table = raw::RECOMMENDED_UPCASE_TABLE;
    let upcase_at = layout.cluster_at(layout.upcase_first());
    write_bytes(dev, block, upcase_at, None, zero_len(layout.upcase_clusters as u64 * layout.cluster())?).await?;
    write_bytes(dev, block, upcase_at, Some(table), table.len()).await?;
    let root_at = layout.cluster_at(layout.root());
    write_bytes(dev, block, root_at, None, zero_len(layout.cluster())?).await?;
    let (entries, count) = root_entries(&layout, options);
    for (index, entry) in entries[..count].iter().enumerate() {
        write_bytes(dev, block, root_at + (index * ENTRY_SIZE) as u64, Some(entry), ENTRY_SIZE).await?;
    }

    let boot = boot_sector(&layout, options, serial);
    let boot = bytemuck::bytes_of(&boot);
    let signature = raw::EXTENDED_BOOT_SIGNATURE.to_le_bytes();
    let mut sum = raw::boot_checksum(0, 0, boot);
    let zeros = [0u8; CHUNK];
    for index in 0..raw::BOOT_REGION_SECTORS - 1 {
        let mut left = if index == 0 { sector as usize - boot.len() } else { sector as usize };
        while left > 0 {
            let n = left.min(CHUNK);
            let mut bytes = zeros;
            if (1..=8).contains(&index) && n == left {
                bytes[n - 4..n].copy_from_slice(&signature);
            }
            sum = raw::boot_checksum(sum, index.max(1), &bytes[..n]);
            left -= n;
        }
    }
    for base in [region, 0] {
        for index in 1..=8u64 {
            let at = base + (index + 1) * sector - 4;
            write_bytes(dev, block, at, Some(&signature), 4).await?;
        }
        let checksum_at = base + (raw::BOOT_REGION_SECTORS - 1) * sector;
        for word in 0..sector / 4 {
            write_bytes(dev, block, checksum_at + word * 4, Some(&sum.to_le_bytes()), 4).await?;
        }
        write_bytes(dev, block, base, Some(boot), boot.len()).await?;
    }
    dev.flush().await?;
    Ok(())
}

}

#[cfg(test)]
mod tests {
    use super::*;
    use hadris_fs::NoClock;

    #[test]
    fn layouts_match_mkfs_exfat() {
        let layout = plan(64 << 20, 512, &FormatOptions::<NoClock>::new()).unwrap();
        assert_eq!(layout.fat_offset, 2048);
        assert_eq!(layout.heap_offset, 4096);
        assert_eq!(layout.root(), 5);
        assert_eq!(
            plan(512 << 10, 512, &FormatOptions::<NoClock>::new()).err(),
            Some(ErrorKind::NoSpace)
        );
        let odd = FormatOptions::<NoClock>::new().with_cluster_size(3000);
        assert_eq!(
            plan(8 << 20, 512, &odd).err(),
            Some(ErrorKind::InvalidInput)
        );
    }
}
