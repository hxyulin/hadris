use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use hadris_apfs::sync::Container;
use hadris_apfs::types::filesystem::{DT_DIR, DT_LNK, DT_REG};
use hadris_part::PartitionKind;
use hadris_storage::BlockSize;
use hadris_storage::ReadOnly;
use hadris_storage::sync::StreamDevice;

#[derive(Debug, Parser)]
#[command(version, about = "Inspect APFS containers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print basic block-zero APFS container information.
    Info {
        /// Path to an APFS container device or image.
        path: PathBuf,
        /// Host/device logical sector size.
        #[arg(long, default_value_t = 512)]
        sector_size: u32,
        /// Treat the path as a whole-disk image and auto-select the APFS GPT partition.
        #[arg(long)]
        gpt: bool,
        /// GPT/MBR partition index to open (defaults to the first APFS GPT partition with --gpt).
        #[arg(long)]
        partition: Option<usize>,
        /// Read and print a file from each volume root directory by name.
        #[arg(long)]
        read_root_file: Option<String>,
    },
    /// Print a regular file by path from a volume root directory.
    Cat {
        /// Path to an APFS container device or image.
        path: PathBuf,
        /// File path to read, relative to the volume root.
        path_in_volume: String,
        /// Host/device logical sector size.
        #[arg(long, default_value_t = 512)]
        sector_size: u32,
        /// Treat the path as a whole-disk image and auto-select the APFS GPT partition.
        #[arg(long)]
        gpt: bool,
        /// GPT/MBR partition index to open (defaults to the first APFS GPT partition with --gpt).
        #[arg(long)]
        partition: Option<usize>,
    },
    /// List a directory in a volume.
    Ls {
        /// Path to an APFS container device or image.
        path: PathBuf,
        /// Directory path to list, relative to the volume root; empty/omitted lists the root.
        #[arg(default_value = "")]
        path_in_volume: String,
        /// Host/device logical sector size.
        #[arg(long, default_value_t = 512)]
        sector_size: u32,
        /// Treat the path as a whole-disk image and auto-select the APFS GPT partition.
        #[arg(long)]
        gpt: bool,
        /// GPT/MBR partition index to open (defaults to the first APFS GPT partition with --gpt).
        #[arg(long)]
        partition: Option<usize>,
    },
    /// Print inode metadata for a path in a volume.
    Stat {
        /// Path to an APFS container device or image.
        path: PathBuf,
        /// File or directory path, relative to the volume root.
        path_in_volume: String,
        /// Host/device logical sector size.
        #[arg(long, default_value_t = 512)]
        sector_size: u32,
        /// Treat the path as a whole-disk image and auto-select the APFS GPT partition.
        #[arg(long)]
        gpt: bool,
        /// GPT/MBR partition index to open (defaults to the first APFS GPT partition with --gpt).
        #[arg(long)]
        partition: Option<usize>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Info {
            path,
            sector_size,
            gpt,
            partition,
            read_root_file,
        } => info(path, sector_size, gpt, partition, read_root_file),
        Command::Cat {
            path,
            path_in_volume,
            sector_size,
            gpt,
            partition,
        } => cat(path, path_in_volume, sector_size, gpt, partition),
        Command::Ls {
            path,
            path_in_volume,
            sector_size,
            gpt,
            partition,
        } => ls(path, path_in_volume, sector_size, gpt, partition),
        Command::Stat {
            path,
            path_in_volume,
            sector_size,
            gpt,
            partition,
        } => stat(path, path_in_volume, sector_size, gpt, partition),
    }
}

/// Formats an APFS nanosecond-since-epoch timestamp as a Unix timestamp with
/// fractional seconds, since we don't depend on a calendar-formatting crate.
fn format_apfs_time(nanos_since_epoch: u64) -> String {
    let seconds = nanos_since_epoch / 1_000_000_000;
    let subsec_nanos = nanos_since_epoch % 1_000_000_000;
    format!("{seconds}.{subsec_nanos:09} unix")
}

enum Action {
    Info {
        path: PathBuf,
        read_root_file: Option<String>,
    },
    Cat(String),
    Ls(String),
    Stat(String),
}

impl Action {
    fn run<D>(self, container: &mut Container<D>) -> anyhow::Result<()>
    where
        D: hadris_storage::sync::BlockDevice,
    {
        match self {
            Self::Info {
                path,
                read_root_file,
            } => print_container_info(&path, container, read_root_file.as_deref()),
            Self::Cat(path) => cat_from_container(container, &path),
            Self::Ls(path) => ls_from_container(container, &path),
            Self::Stat(path) => stat_from_container(container, &path),
        }
    }
}

fn info(
    path: PathBuf,
    sector_size: u32,
    gpt: bool,
    partition: Option<usize>,
    read_root_file: Option<String>,
) -> anyhow::Result<()> {
    let action = Action::Info {
        path: path.clone(),
        read_root_file,
    };
    open_container(&path, sector_size, gpt, partition, action)
}

fn open_container(
    path: &Path,
    sector_size: u32,
    gpt: bool,
    partition: Option<usize>,
    action: Action,
) -> anyhow::Result<()> {
    let block_size = BlockSize::new(sector_size).context("sector size must be non-zero")?;
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = hadris_storage::host::file_len(&file).context("determining container size")?;
    let mut device = StreamDevice::with_block_count(
        ReadOnly::new(hadris_io::StdIo::new(file)),
        block_size,
        len / u64::from(sector_size),
    );
    if gpt || partition.is_some() {
        let disk = hadris_part::sync::read(&mut device).context("reading partition table")?;
        let selected = disk
            .partitions()
            .find(|part| match (partition, part.kind()) {
                (Some(index), _) => part.index() == index,
                (None, PartitionKind::Gpt(guid)) => guid == hadris_part::gpt::types::APPLE_APFS,
                (None, _) => false,
            })
            .context("no matching APFS partition found")?;
        let view =
            hadris_part::sync::open(device, &selected).context("opening partition device")?;
        let mut container = Container::open(view).with_context(|| {
            format!(
                "opening APFS partition {} at LBA {}",
                selected.index(),
                selected.start()
            )
        })?;
        action.run(&mut container)
    } else {
        let mut container = Container::open(device).or_else(|error| {
            bail!("opening APFS container failed: {error}. If this is a full-disk/GPT image, retry with --gpt")
        })?;
        action.run(&mut container)
    }
}

fn cat(
    path: PathBuf,
    path_in_volume: String,
    sector_size: u32,
    gpt: bool,
    partition: Option<usize>,
) -> anyhow::Result<()> {
    open_container(
        &path,
        sector_size,
        gpt,
        partition,
        Action::Cat(path_in_volume),
    )
}

fn cat_from_container<D>(container: &mut Container<D>, path_in_volume: &str) -> anyhow::Result<()>
where
    D: hadris_storage::sync::BlockDevice,
{
    let latest = container.latest_superblock()?;
    let volumes = container.volume_superblocks(&latest)?;
    for volume in &volumes {
        let Some(entry) = container.resolve_path(volume, path_in_volume)? else {
            continue;
        };
        match entry.file_type() {
            DT_REG => {}
            DT_DIR => bail!("{path_in_volume} is a directory"),
            DT_LNK => match container.symlink_target(volume, entry.file_id)? {
                Some(target) => bail!("{path_in_volume} is a symlink to {target}"),
                None => bail!("{path_in_volume} is a symlink"),
            },
            other => bail!("{path_in_volume} is not a regular file (type {other})"),
        }
        let inode = container
            .inode_record(volume, entry.file_id)?
            .context("inode record not found")?;
        if inode.is_compressed() {
            bail!("{path_in_volume} is compressed, which this reader does not decode");
        }
        let extents = container.file_extents(volume, inode.private_id)?;
        let size = hadris_apfs::types::filesystem::stream_size(&inode, &extents);
        let mut out = std::io::stdout().lock();
        let mut buf = vec![0_u8; 1 << 20];
        let mut offset = 0;
        loop {
            let n = container.read_extents_at(&extents, size, offset, &mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            offset += n as u64;
        }
        return Ok(());
    }
    bail!("file not found: {path_in_volume}")
}

fn ls(
    path: PathBuf,
    path_in_volume: String,
    sector_size: u32,
    gpt: bool,
    partition: Option<usize>,
) -> anyhow::Result<()> {
    open_container(
        &path,
        sector_size,
        gpt,
        partition,
        Action::Ls(path_in_volume),
    )
}

fn ls_from_container<D>(container: &mut Container<D>, path_in_volume: &str) -> anyhow::Result<()>
where
    D: hadris_storage::sync::BlockDevice,
{
    let latest = container.latest_superblock()?;
    let volumes = container.volume_superblocks(&latest)?;
    let mut found = false;
    for volume in &volumes {
        let directory_id = if path_in_volume.trim_matches('/').is_empty() {
            hadris_apfs::types::filesystem::INODE_ROOT_DIRECTORY
        } else {
            match container.resolve_path(volume, path_in_volume)? {
                Some(entry) if entry.file_type() == DT_DIR => entry.file_id,
                Some(_) => bail!("{path_in_volume} is not a directory"),
                None => continue,
            }
        };
        found = true;
        println!("{}:", volume.name().unwrap_or("<invalid utf8>"));
        for entry in container.directory_owned_entries(volume, directory_id)? {
            let kind = entry.file_type();
            print!("  {} (inode {}, type {kind})", entry.name, entry.file_id);
            if kind == DT_LNK
                && let Some(target) = container.symlink_target(volume, entry.file_id)?
            {
                print!(" -> {target}");
            }
            println!();
        }
    }
    if !found {
        bail!("path not found: {path_in_volume}");
    }
    Ok(())
}

fn stat(
    path: PathBuf,
    path_in_volume: String,
    sector_size: u32,
    gpt: bool,
    partition: Option<usize>,
) -> anyhow::Result<()> {
    open_container(
        &path,
        sector_size,
        gpt,
        partition,
        Action::Stat(path_in_volume),
    )
}

fn stat_from_container<D>(container: &mut Container<D>, path_in_volume: &str) -> anyhow::Result<()>
where
    D: hadris_storage::sync::BlockDevice,
{
    let latest = container.latest_superblock()?;
    let volumes = container.volume_superblocks(&latest)?;
    for volume in &volumes {
        let inode_id = if path_in_volume.trim_matches('/').is_empty() {
            hadris_apfs::types::filesystem::INODE_ROOT_DIRECTORY
        } else {
            match container.resolve_path(volume, path_in_volume)? {
                Some(entry) => entry.file_id,
                None => continue,
            }
        };
        let Some(inode) = container.inode_record(volume, inode_id)? else {
            continue;
        };
        println!("volume: {}", volume.name().unwrap_or("<invalid utf8>"));
        println!("  inode: {}", inode.id);
        println!("  parent: {}", inode.parent_id);
        println!("  mode: {:#o}", inode.mode);
        println!("  owner: {}:{}", inode.owner, inode.group);
        println!("  size: {}", container.file_size(volume, inode_id)?);
        if inode.is_compressed() {
            println!("  compressed: yes");
        }
        if u32::from(inode.mode) & 0o170000 == 0o120000
            && let Some(target) = container.symlink_target(volume, inode_id)?
        {
            println!("  target: {target}");
        }
        println!("  link/child count: {}", inode.link_or_child_count);
        println!("  created: {}", format_apfs_time(inode.create_time_ns));
        println!(
            "  modified: {}",
            format_apfs_time(inode.modification_time_ns)
        );
        println!("  changed: {}", format_apfs_time(inode.change_time_ns));
        println!("  accessed: {}", format_apfs_time(inode.access_time_ns));
        return Ok(());
    }
    bail!("path not found: {path_in_volume}")
}

fn print_container_info<D>(
    path: &Path,
    container: &mut Container<D>,
    read_root_file: Option<&str>,
) -> anyhow::Result<()>
where
    D: hadris_storage::sync::BlockDevice,
{
    let latest = container.latest_superblock()?;
    let mappings = container.checkpoint_mappings(&latest)?;
    let object_map = container.object_map(&latest)?;
    let object_map_value_count = container
        .object_map_values(&latest)
        .map(|values| values.len())
        .unwrap_or(0);
    let space_manager = container.space_manager_summary(&latest).ok();
    let chunk_free_sum = container
        .space_manager_chunk_infos(&latest)
        .ok()
        .map(|chunks| {
            chunks
                .iter()
                .map(|chunk| u64::from(chunk.free_count))
                .sum::<u64>()
        });
    let volume_locations = match container.volume_object_map_values(&latest) {
        Ok(values) => values,
        Err(error) => {
            eprintln!(
                "warning: object-map B-tree walk did not resolve volume locations yet: {error}"
            );
            Vec::new()
        }
    };
    let volume_superblocks = match container.volume_superblocks(&latest) {
        Ok(volumes) => volumes,
        Err(error) => {
            eprintln!("warning: volume superblock parsing failed: {error}");
            Vec::new()
        }
    };
    let root_tree_locations: Vec<(u64, hadris_apfs::types::ObjectMapValue)> = volume_superblocks
        .iter()
        .filter_map(|volume| {
            container
                .resolve_volume_object(volume, volume.root_tree_oid)
                .ok()
                .flatten()
                .map(|value| (volume.object.identifier, value))
        })
        .collect();
    print_info(
        space_manager.as_ref(),
        chunk_free_sum,
        path,
        &latest,
        mappings.len(),
        &object_map,
        object_map_value_count,
        &volume_locations,
        &volume_superblocks,
        &root_tree_locations,
    );
    for volume in &volume_superblocks {
        match container.root_directory_owned_entries(volume) {
            Ok(entries) if !entries.is_empty() => {
                println!(
                    "  root directory entries for '{}':",
                    volume.name().unwrap_or("<invalid utf8>")
                );
                for entry in entries {
                    print!(
                        "    {} -> inode {} (type {})",
                        entry.name,
                        entry.file_id,
                        entry.file_type()
                    );
                    if entry.file_type() == DT_REG
                        && let Ok(Some(inode)) = container.inode_record(volume, entry.file_id)
                    {
                        let stream_id = inode.private_id;
                        if let Ok(extents) = container.file_extents(volume, stream_id)
                            && !extents.is_empty()
                        {
                            print!(" extents:");
                            for extent in extents {
                                print!(
                                    " [logical {} len {} block {}]",
                                    extent.logical_address, extent.length, extent.physical_block
                                );
                            }
                        }
                        if let Ok(bytes) = container.read_file(volume, entry.file_id, 64)
                            && !bytes.is_empty()
                        {
                            let printable = String::from_utf8_lossy(&bytes);
                            print!(" preview {:?}", printable.trim_end_matches('\0'));
                        }
                    }
                    println!();
                    if read_root_file == Some(entry.name.as_str()) {
                        let bytes = container.read_file(volume, entry.file_id, 1024 * 1024)?;
                        println!("  contents of '{}':", entry.name);
                        let mut out = std::io::stdout().lock();
                        out.write_all(&bytes)?;
                        if !bytes.ends_with(b"\n") {
                            writeln!(out)?;
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(error) => eprintln!("warning: root directory listing failed: {error}"),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn print_info(
    space_manager: Option<&hadris_apfs::types::SpaceManagerSummary>,
    chunk_free_sum: Option<u64>,
    path: &Path,
    sb: &hadris_apfs::types::ContainerSuperblock,
    checkpoint_mappings: usize,
    object_map: &hadris_apfs::types::ObjectMapBlock,
    object_map_value_count: usize,
    volume_locations: &[(
        hadris_apfs::types::ObjectMapKey,
        hadris_apfs::types::ObjectMapValue,
    )],
    volume_superblocks: &[hadris_apfs::types::VolumeSuperblock],
    root_tree_locations: &[(u64, hadris_apfs::types::ObjectMapValue)],
) {
    println!("APFS container: {}", path.display());
    println!("  block size: {}", sb.block_size);
    println!("  block count: {}", sb.block_count);
    println!("  object map oid: {}", sb.object_map_oid);
    println!("  transaction: {}", sb.object.transaction_identifier);
    println!("  checkpoint mappings: {checkpoint_mappings}");
    println!("  object map tree oid: {}", object_map.tree_oid);
    println!("  object map snapshots: {}", object_map.snapshot_count);
    println!("  object map values walked: {object_map_value_count}");
    if let Some(space_manager) = space_manager {
        let device = space_manager.main_device;
        let used = device.block_count.saturating_sub(device.free_count);
        println!(
            "  space manager: {} free / {} total blocks ({} used, block size {})",
            device.free_count, device.block_count, used, space_manager.block_size
        );
        if let Some(chunk_free_sum) = chunk_free_sum {
            let matches = if chunk_free_sum == device.free_count {
                "matches"
            } else {
                "MISMATCH"
            };
            println!("  chunk-info free sum: {chunk_free_sum} ({matches} header free count)");
        }
    } else {
        println!("  space manager: unavailable");
    }
    println!("  volumes:");
    for oid in sb.volumes() {
        if let Some((key, value)) = volume_locations
            .iter()
            .find(|(key, _)| key.oid == oid || (key.oid & 0x0fff_ffff_ffff_ffff) == oid)
        {
            if let Some(volume) = volume_superblocks
                .iter()
                .find(|volume| volume.object.identifier == key.oid)
            {
                if let Some((_, root_tree)) = root_tree_locations
                    .iter()
                    .find(|(volume_oid, _)| *volume_oid == volume.object.identifier)
                {
                    println!(
                        "    {} '{}' -> block {} (root tree block {}, files {}, dirs {})",
                        oid,
                        volume.name().unwrap_or("<invalid utf8>"),
                        value.address,
                        root_tree.address,
                        volume.number_files,
                        volume.number_directories
                    );
                } else {
                    println!(
                        "    {} '{}' -> block {} (xid {}, files {}, dirs {})",
                        oid,
                        volume.name().unwrap_or("<invalid utf8>"),
                        value.address,
                        key.xid,
                        volume.number_files,
                        volume.number_directories
                    );
                }
            } else {
                println!(
                    "    {} -> block {} (xid {}, size {})",
                    oid, value.address, key.xid, value.size
                );
            }
        } else {
            println!("    {oid}");
        }
    }
}
