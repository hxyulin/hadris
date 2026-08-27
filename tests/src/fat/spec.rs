//! Test-only raw-image oracle derived from the FAT on-disk rules.
//!
//! It reads an image with no help from any implementation under test and
//! rejects structural violations: bad geometry, divergent FAT copies, invalid
//! reserved entries, FAT32 metadata inconsistencies, cluster loops and
//! cross-links, malformed directory records, orphaned or inconsistent long
//! names, duplicate short aliases, and unowned allocated clusters.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::model::{EntryState, FsState};
use super::{MUTABLE_ATTRS, fat_path_eq};
use crate::harness::join_path;
use crate::harness::tree::EntryData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FatKind {
    Fat12,
    Fat16,
    Fat32,
}

struct Geometry {
    kind: FatKind,
    bytes_per_sector: usize,
    sectors_per_cluster: usize,
    reserved_sectors: usize,
    fat_count: usize,
    sectors_per_fat: usize,
    root_entry_count: usize,
    root_cluster: u32,
    root_dir_sector: usize,
    data_sector: usize,
    cluster_count: u32,
    media: u8,
}

/// The parts of the BPB that scenarios size themselves from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageGeometry {
    pub bits: u8,
    pub cluster_size: usize,
    pub cluster_count: u32,
    /// Zero for FAT32, whose root directory grows like any other.
    pub root_entry_count: usize,
}

pub fn geometry(path: &Path) -> Result<ImageGeometry, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let geometry = parse_geometry(&bytes)?;
    Ok(ImageGeometry {
        bits: fat_bits(geometry.kind),
        cluster_size: geometry.bytes_per_sector * geometry.sectors_per_cluster,
        cluster_count: geometry.cluster_count,
        root_entry_count: geometry.root_entry_count,
    })
}

/// Counts clusters whose FAT entry is zero, independently of any FSInfo hint.
pub fn free_clusters(path: &Path) -> Result<u32, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let geometry = parse_geometry(&bytes)?;
    count_free_clusters(&bytes, &geometry)
}

fn count_free_clusters(bytes: &[u8], geometry: &Geometry) -> Result<u32, String> {
    let mut free = 0;
    for cluster in 2..=geometry.cluster_count + 1 {
        if fat_entry(bytes, geometry, cluster)? == 0 {
            free += 1;
        }
    }
    Ok(free)
}

/// Validates the image at `path` and returns its semantic tree. `expected_bits`
/// guards against a formatter silently choosing a different FAT width.
pub fn snapshot(path: &Path, expected_bits: u8) -> Result<FsState, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let geometry = parse_geometry(&bytes)?;
    if fat_bits(geometry.kind) != expected_bits {
        return Err(format!(
            "oracle detected FAT{} instead of FAT{expected_bits}",
            fat_bits(geometry.kind)
        ));
    }
    validate_fat_copies(&bytes, &geometry)?;
    validate_reserved_entries(&bytes, &geometry)?;
    validate_fat32_metadata(&bytes, &geometry)?;
    let label_offset = if geometry.kind == FatKind::Fat32 {
        71
    } else {
        43
    };
    let label = ascii_field(slice(&bytes, label_offset, 11)?)?;
    let mut oracle = Oracle {
        bytes,
        geometry,
        claimed_clusters: BTreeMap::new(),
        root_label: None,
    };
    let mut entries = BTreeMap::new();
    if oracle.geometry.kind == FatKind::Fat32 {
        let root = oracle.read_chain(oracle.geometry.root_cluster, "/")?;
        oracle.read_directory(&root, "/", None, 0, &mut entries)?;
    } else {
        let start = oracle.geometry.root_dir_sector * oracle.geometry.bytes_per_sector;
        let len = oracle.geometry.root_entry_count * 32;
        let root = slice(&oracle.bytes, start, len)?.to_vec();
        oracle.read_directory(&root, "/", None, 0, &mut entries)?;
    }
    if oracle.root_label.as_deref() != Some(label.as_str()) {
        return Err(format!(
            "root-directory label {:?} differs from BPB label {label:?}",
            oracle.root_label
        ));
    }
    oracle.validate_allocations()?;
    Ok(FsState { label, entries })
}

struct Oracle {
    bytes: Vec<u8>,
    geometry: Geometry,
    claimed_clusters: BTreeMap<u32, String>,
    root_label: Option<String>,
}

impl Oracle {
    fn read_directory(
        &mut self,
        bytes: &[u8],
        parent: &str,
        current_cluster: Option<u32>,
        expected_parent_cluster: u32,
        output: &mut BTreeMap<String, EntryState>,
    ) -> Result<(), String> {
        if let Some(cluster) = current_cluster {
            validate_dot_entries(bytes, cluster, expected_parent_cluster, parent)?;
        }
        let mut aliases = BTreeSet::new();
        let mut lfn = Vec::new();
        let mut pending_dirs = Vec::new();
        for (index, raw) in bytes.chunks_exact(32).enumerate() {
            match raw[0] {
                0x00 => break,
                0xe5 => {
                    lfn.clear();
                    continue;
                }
                _ => {}
            }
            let attrs = raw[11];
            if attrs == 0x0f {
                lfn.push(parse_lfn_slot(raw)?);
                continue;
            }
            if attrs & 0x08 != 0 {
                if parent == "/" {
                    if self.root_label.is_some() {
                        return Err("multiple root-directory volume labels".to_string());
                    }
                    self.root_label = Some(ascii_field(&raw[..11])?);
                }
                lfn.clear();
                continue;
            }
            let alias: [u8; 11] = raw[..11]
                .try_into()
                .map_err(|_| "short alias has wrong width".to_string())?;
            let short_name = decode_short_name(&alias, raw[12]);
            if short_name == "." || short_name == ".." {
                if parent == "/" {
                    return Err(format!("root directory contains a {short_name} entry"));
                }
                if !lfn.is_empty() {
                    return Err(format!(
                        "long-name entries precede the {short_name} entry in {parent}"
                    ));
                }
                if index > 1 {
                    return Err(format!(
                        "{short_name} entry at slot {index} of {parent} is not one of the first two"
                    ));
                }
                continue;
            }
            validate_short_alias(&alias, parent)?;
            if !aliases.insert(alias) {
                return Err(format!(
                    "duplicate short alias {:?} in {parent}",
                    String::from_utf8_lossy(&alias)
                ));
            }
            let name = if lfn.is_empty() {
                short_name
            } else {
                decode_lfn(&lfn, &alias)?
            };
            lfn.clear();
            let path = join_path(parent, &name);
            if output.keys().any(|existing| fat_path_eq(existing, &path)) {
                return Err(format!("duplicate name {path} in {parent}"));
            }
            let cluster = u32::from(read_u16(raw, 26)?) | (u32::from(read_u16(raw, 20)?) << 16);
            let size = read_u32(raw, 28)? as usize;
            let stable_attrs = attrs & MUTABLE_ATTRS;
            if attrs & 0x10 != 0 {
                if cluster < 2 {
                    return Err(format!("directory {path} has invalid cluster {cluster}"));
                }
                if size != 0 {
                    return Err(format!("directory {path} has nonzero size {size}"));
                }
                output.insert(
                    path.clone(),
                    EntryState {
                        data: EntryData::Directory,
                        attrs: stable_attrs,
                    },
                );
                pending_dirs.push((path, cluster));
            } else {
                let contents = self.read_file(cluster, size, &path)?;
                output.insert(
                    path,
                    EntryState {
                        data: EntryData::File(contents),
                        attrs: stable_attrs,
                    },
                );
            }
        }
        if !lfn.is_empty() {
            return Err(format!("orphaned long-name entries in {parent}"));
        }
        for (path, cluster) in pending_dirs {
            let contents = self.read_chain(cluster, &path)?;
            self.read_directory(
                &contents,
                &path,
                Some(cluster),
                current_cluster.unwrap_or(0),
                output,
            )?;
        }
        Ok(())
    }

    fn validate_allocations(&self) -> Result<(), String> {
        for cluster in 2..=self.geometry.cluster_count + 1 {
            let value = fat_entry(&self.bytes, &self.geometry, cluster)?;
            let bad = match self.geometry.kind {
                FatKind::Fat12 => 0x0ff7,
                FatKind::Fat16 => 0xfff7,
                FatKind::Fat32 => 0x0fff_fff7,
            };
            if value != 0 && value != bad && !self.claimed_clusters.contains_key(&cluster) {
                return Err(format!(
                    "allocated cluster {cluster} is not owned by any entry"
                ));
            }
        }
        Ok(())
    }

    fn read_file(&mut self, cluster: u32, size: usize, path: &str) -> Result<Vec<u8>, String> {
        if size == 0 {
            if cluster != 0 {
                return Err(format!("empty file {path} owns cluster {cluster}"));
            }
            return Ok(Vec::new());
        }
        if cluster < 2 {
            return Err(format!(
                "non-empty file {path} has invalid cluster {cluster}"
            ));
        }
        let chain = self.read_chain(cluster, path)?;
        let cluster_size = self.geometry.bytes_per_sector * self.geometry.sectors_per_cluster;
        let expected_clusters = size.div_ceil(cluster_size);
        if chain.len() != expected_clusters * cluster_size {
            return Err(format!(
                "file {path} has {} clusters, expected {expected_clusters}",
                chain.len() / cluster_size
            ));
        }
        Ok(chain[..size].to_vec())
    }

    fn read_chain(&mut self, first: u32, owner: &str) -> Result<Vec<u8>, String> {
        let mut output = Vec::new();
        let mut visited = BTreeSet::new();
        let mut cluster = first;
        loop {
            if cluster < 2 || cluster > self.geometry.cluster_count + 1 {
                return Err(format!("{owner} references invalid cluster {cluster}"));
            }
            if !visited.insert(cluster) {
                return Err(format!("cluster loop at {cluster} in {owner}"));
            }
            if let Some(previous) = self.claimed_clusters.insert(cluster, owner.to_string()) {
                return Err(format!(
                    "cluster {cluster} is cross-linked between {previous} and {owner}"
                ));
            }
            let offset = self.cluster_offset(cluster)?;
            let cluster_size = self.geometry.bytes_per_sector * self.geometry.sectors_per_cluster;
            output.extend_from_slice(slice(&self.bytes, offset, cluster_size)?);
            match self.next_cluster(cluster)? {
                Some(next) => cluster = next,
                None => return Ok(output),
            }
        }
    }

    fn cluster_offset(&self, cluster: u32) -> Result<usize, String> {
        let sector = self
            .geometry
            .data_sector
            .checked_add((cluster as usize - 2) * self.geometry.sectors_per_cluster)
            .ok_or_else(|| "cluster offset overflow".to_string())?;
        sector
            .checked_mul(self.geometry.bytes_per_sector)
            .ok_or_else(|| "cluster byte offset overflow".to_string())
    }

    fn next_cluster(&self, cluster: u32) -> Result<Option<u32>, String> {
        let value = fat_entry(&self.bytes, &self.geometry, cluster)?;
        let eoc = match self.geometry.kind {
            FatKind::Fat12 => 0x0ff8,
            FatKind::Fat16 => 0xfff8,
            FatKind::Fat32 => 0x0fff_fff8,
        };
        let bad = match self.geometry.kind {
            FatKind::Fat12 => 0x0ff7,
            FatKind::Fat16 => 0xfff7,
            FatKind::Fat32 => 0x0fff_fff7,
        };
        if value >= eoc {
            Ok(None)
        } else if value == bad {
            Err(format!("cluster {cluster} points to a bad-cluster marker"))
        } else if value < 2 || value > self.geometry.cluster_count + 1 {
            Err(format!(
                "cluster {cluster} has invalid successor {value:#x}"
            ))
        } else {
            Ok(Some(value))
        }
    }
}

fn parse_geometry(bytes: &[u8]) -> Result<Geometry, String> {
    if slice(bytes, 510, 2)? != [0x55, 0xaa] {
        return Err("missing FAT boot signature".to_string());
    }
    let bytes_per_sector = usize::from(read_u16(bytes, 11)?);
    let sectors_per_cluster = usize::from(*slice(bytes, 13, 1)?.first().unwrap());
    let reserved_sectors = usize::from(read_u16(bytes, 14)?);
    let fat_count = usize::from(*slice(bytes, 16, 1)?.first().unwrap());
    let root_entry_count = usize::from(read_u16(bytes, 17)?);
    let total16 = usize::from(read_u16(bytes, 19)?);
    let total_sectors = if total16 == 0 {
        read_u32(bytes, 32)? as usize
    } else {
        total16
    };
    let fat16 = usize::from(read_u16(bytes, 22)?);
    let sectors_per_fat = if fat16 == 0 {
        read_u32(bytes, 36)? as usize
    } else {
        fat16
    };
    if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
        || sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
        || sectors_per_cluster > 128
        || reserved_sectors == 0
        || fat_count == 0
        || sectors_per_fat == 0
    {
        return Err("invalid FAT geometry".to_string());
    }
    let root_dir_sectors = (root_entry_count * 32).div_ceil(bytes_per_sector);
    let overhead = reserved_sectors
        .checked_add(fat_count * sectors_per_fat)
        .and_then(|value| value.checked_add(root_dir_sectors))
        .ok_or_else(|| "FAT geometry overflow".to_string())?;
    let data_sectors = total_sectors
        .checked_sub(overhead)
        .ok_or_else(|| "FAT data region underflow".to_string())?;
    let cluster_count = (data_sectors / sectors_per_cluster) as u32;
    let kind = if cluster_count < 4085 {
        FatKind::Fat12
    } else if cluster_count < 65525 {
        FatKind::Fat16
    } else {
        FatKind::Fat32
    };
    let root_cluster = if kind == FatKind::Fat32 {
        read_u32(bytes, 44)? & 0x0fff_ffff
    } else {
        0
    };
    let volume_len = total_sectors
        .checked_mul(bytes_per_sector)
        .ok_or_else(|| "FAT volume length overflow".to_string())?;
    if volume_len > bytes.len() {
        return Err(format!(
            "FAT volume needs {volume_len} bytes but image has {}",
            bytes.len()
        ));
    }
    if kind == FatKind::Fat32 {
        if root_entry_count != 0 || fat16 != 0 || root_cluster < 2 {
            return Err("invalid FAT32 root or FAT-size fields".to_string());
        }
    } else if root_entry_count == 0 || fat16 == 0 {
        return Err("invalid FAT12/16 root or FAT-size fields".to_string());
    }
    let fat_entries = match kind {
        FatKind::Fat12 => sectors_per_fat * bytes_per_sector * 2 / 3,
        FatKind::Fat16 => sectors_per_fat * bytes_per_sector / 2,
        FatKind::Fat32 => sectors_per_fat * bytes_per_sector / 4,
    };
    if fat_entries < cluster_count as usize + 2 {
        return Err("FAT is too small for the data-region cluster count".to_string());
    }
    let root_dir_sector = reserved_sectors + fat_count * sectors_per_fat;
    Ok(Geometry {
        kind,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        fat_count,
        sectors_per_fat,
        root_entry_count,
        root_cluster,
        root_dir_sector,
        data_sector: root_dir_sector + root_dir_sectors,
        cluster_count,
        media: bytes[21],
    })
}

fn validate_fat_copies(bytes: &[u8], geometry: &Geometry) -> Result<(), String> {
    let fat_len = geometry.sectors_per_fat * geometry.bytes_per_sector;
    let first_start = geometry.reserved_sectors * geometry.bytes_per_sector;
    let first = slice(bytes, first_start, fat_len)?;
    for index in 1..geometry.fat_count {
        let start = first_start + index * fat_len;
        if slice(bytes, start, fat_len)? != first {
            return Err(format!("FAT copy {index} differs from FAT copy 0"));
        }
    }
    Ok(())
}

fn validate_reserved_entries(bytes: &[u8], geometry: &Geometry) -> Result<(), String> {
    let first = fat_entry(bytes, geometry, 0)?;
    if first as u8 != geometry.media {
        return Err(format!(
            "FAT media byte {:#04x} differs from BPB {:#04x}",
            first as u8, geometry.media
        ));
    }
    let second = fat_entry(bytes, geometry, 1)?;
    let minimum = match geometry.kind {
        FatKind::Fat12 => 0x0ff8,
        FatKind::Fat16 => 0xfff8,
        FatKind::Fat32 => 0x0fff_fff8,
    };
    if second < minimum {
        return Err(format!("reserved FAT[1] entry is invalid: {second:#x}"));
    }
    Ok(())
}

fn validate_fat32_metadata(bytes: &[u8], geometry: &Geometry) -> Result<(), String> {
    if geometry.kind != FatKind::Fat32 {
        return Ok(());
    }
    let fsinfo_sector = usize::from(read_u16(bytes, 48)?);
    let backup_sector = usize::from(read_u16(bytes, 50)?);
    let fsinfo = fsinfo_sector * geometry.bytes_per_sector;
    if read_u32(bytes, fsinfo)? != 0x4161_5252
        || read_u32(bytes, fsinfo + 484)? != 0x6141_7272
        || read_u32(bytes, fsinfo + 508)? != 0xaa55_0000
    {
        return Err("invalid FAT32 FSInfo signatures".to_string());
    }
    let free_count = read_u32(bytes, fsinfo + 488)?;
    let next_free = read_u32(bytes, fsinfo + 492)?;
    // Stricter than the specification, which only recommends an accurate
    // count at dismount; dosfstools reports a stale count as an error.
    if free_count != u32::MAX {
        let actual = count_free_clusters(bytes, geometry)?;
        if free_count != actual {
            return Err(format!(
                "FAT32 FSInfo free count {free_count} differs from the {actual} free clusters in the FAT"
            ));
        }
    }
    if next_free != u32::MAX && !(2..=geometry.cluster_count + 1).contains(&next_free) {
        return Err("FAT32 FSInfo next-free hint is outside the cluster heap".to_string());
    }
    if backup_sector != 0 && backup_sector != u16::MAX as usize {
        let primary = slice(bytes, 0, geometry.bytes_per_sector)?;
        let backup = slice(
            bytes,
            backup_sector * geometry.bytes_per_sector,
            geometry.bytes_per_sector,
        )?;
        if primary != backup {
            return Err("FAT32 backup boot sector differs from the primary".to_string());
        }
    }
    Ok(())
}

fn fat_entry(bytes: &[u8], geometry: &Geometry, cluster: u32) -> Result<u32, String> {
    let fat_start = geometry.reserved_sectors * geometry.bytes_per_sector;
    match geometry.kind {
        FatKind::Fat12 => {
            let offset = fat_start + cluster as usize + cluster as usize / 2;
            let pair = u16::from_le_bytes(
                slice(bytes, offset, 2)?
                    .try_into()
                    .map_err(|_| "invalid FAT12 entry".to_string())?,
            );
            Ok(u32::from(if cluster & 1 == 0 {
                pair & 0x0fff
            } else {
                pair >> 4
            }))
        }
        FatKind::Fat16 => Ok(u32::from(read_u16(
            bytes,
            fat_start + cluster as usize * 2,
        )?)),
        FatKind::Fat32 => Ok(read_u32(bytes, fat_start + cluster as usize * 4)? & 0x0fff_ffff),
    }
}

struct LfnSlot {
    sequence: u8,
    last: bool,
    checksum: u8,
    units: [u16; 13],
}

fn parse_lfn_slot(raw: &[u8]) -> Result<LfnSlot, String> {
    let sequence = raw[0] & 0x1f;
    if sequence == 0 || raw[12] != 0 || read_u16(raw, 26)? != 0 {
        return Err("invalid long-name directory entry".to_string());
    }
    let mut units = [0_u16; 13];
    for (target, offset) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30]
        .into_iter()
        .enumerate()
    {
        units[target] = read_u16(raw, offset)?;
    }
    Ok(LfnSlot {
        sequence,
        last: raw[0] & 0x40 != 0,
        checksum: raw[13],
        units,
    })
}

fn decode_lfn(slots: &[LfnSlot], alias: &[u8; 11]) -> Result<String, String> {
    let total = slots
        .first()
        .filter(|slot| slot.last)
        .map(|slot| slot.sequence)
        .ok_or_else(|| "long-name sequence is missing its last-entry marker".to_string())?;
    if slots.len() != usize::from(total)
        || slots
            .iter()
            .enumerate()
            .any(|(index, slot)| slot.sequence != total - index as u8)
    {
        return Err("long-name sequence numbers are inconsistent".to_string());
    }
    let checksum = short_checksum(alias);
    if slots.iter().any(|slot| slot.checksum != checksum) {
        return Err("long-name checksum does not match its short alias".to_string());
    }
    let mut units = Vec::new();
    for slot in slots.iter().rev() {
        for unit in slot.units {
            match unit {
                0x0000 | 0xffff => break,
                value => units.push(value),
            }
        }
    }
    String::from_utf16(&units).map_err(|error| error.to_string())
}

fn short_checksum(alias: &[u8; 11]) -> u8 {
    alias.iter().fold(0_u8, |checksum, byte| {
        checksum.rotate_right(1).wrapping_add(*byte)
    })
}

/// The 8.3 character rules from the Microsoft FAT specification: no
/// lowercase letters, no leading space or period, and none of the reserved
/// punctuation. `0x05` may only stand in for a leading `0xE5`.
fn validate_short_alias(alias: &[u8; 11], parent: &str) -> Result<(), String> {
    const RESERVED: &[u8] = b"\"*+,./:;<=>?[\\]|";
    let display = String::from_utf8_lossy(alias);
    if matches!(alias[0], b' ' | b'.') {
        return Err(format!(
            "short alias {display:?} in {parent} starts with {:?}",
            alias[0] as char
        ));
    }
    for (index, byte) in alias.iter().enumerate() {
        let bad = (*byte < 0x20 && !(index == 0 && *byte == 0x05))
            || *byte == 0x7f
            || byte.is_ascii_lowercase()
            || RESERVED.contains(byte);
        if bad {
            return Err(format!(
                "short alias {display:?} in {parent} contains invalid byte {byte:#04x}"
            ));
        }
    }
    Ok(())
}

fn decode_short_name(alias: &[u8; 11], case: u8) -> String {
    let mut base = alias[..8].to_vec();
    let mut extension = alias[8..].to_vec();
    while base.last() == Some(&b' ') {
        base.pop();
    }
    while extension.last() == Some(&b' ') {
        extension.pop();
    }
    if base.first() == Some(&0x05) {
        base[0] = 0xe5;
    }
    if case & 0x08 != 0 {
        base.make_ascii_lowercase();
    }
    if case & 0x10 != 0 {
        extension.make_ascii_lowercase();
    }
    let mut name = String::from_utf8_lossy(&base).into_owned();
    if !extension.is_empty() {
        name.push('.');
        name.push_str(&String::from_utf8_lossy(&extension));
    }
    name
}

fn validate_dot_entries(
    bytes: &[u8],
    current_cluster: u32,
    parent_cluster: u32,
    path: &str,
) -> Result<(), String> {
    let dot = slice(bytes, 0, 32)?;
    let dotdot = slice(bytes, 32, 32)?;
    if &dot[..11] != b".          " || dot[11] & 0x10 == 0 {
        return Err(format!("directory {path} has an invalid . entry"));
    }
    if &dotdot[..11] != b"..         " || dotdot[11] & 0x10 == 0 {
        return Err(format!("directory {path} has an invalid .. entry"));
    }
    let dot_cluster = u32::from(read_u16(dot, 26)?) | (u32::from(read_u16(dot, 20)?) << 16);
    let dotdot_cluster =
        u32::from(read_u16(dotdot, 26)?) | (u32::from(read_u16(dotdot, 20)?) << 16);
    if dot_cluster != current_cluster {
        return Err(format!(
            "directory {path} . points to {dot_cluster}, expected {current_cluster}"
        ));
    }
    if dotdot_cluster != parent_cluster {
        return Err(format!(
            "directory {path} .. points to {dotdot_cluster}, expected {parent_cluster}"
        ));
    }
    Ok(())
}

fn ascii_field(bytes: &[u8]) -> Result<String, String> {
    if !bytes.is_ascii() {
        return Err("non-ASCII BPB text field".to_string());
    }
    Ok(String::from_utf8(bytes.to_vec())
        .map_err(|error| error.to_string())?
        .trim()
        .to_string())
}

fn fat_bits(kind: FatKind) -> u8 {
    match kind {
        FatKind::Fat12 => 12,
        FatKind::Fat16 => 16,
        FatKind::Fat32 => 32,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        slice(bytes, offset, 2)?
            .try_into()
            .map_err(|_| "invalid u16 field".to_string())?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        slice(bytes, offset, 4)?
            .try_into()
            .map_err(|_| "invalid u32 field".to_string())?,
    ))
}

fn slice(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8], String> {
    bytes
        .get(offset..offset.saturating_add(len))
        .ok_or_else(|| format!("image is truncated at offset {offset} for {len} bytes"))
}
