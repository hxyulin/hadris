//! Test-only raw-image oracle derived from the exFAT specification.
//!
//! It reads an image with no help from any implementation under test and
//! rejects structural violations: a bad boot region or checksum, a backup
//! boot region that differs, a set VolumeDirty flag, a wrong PercentInUse,
//! bad FAT entries 0 and 1, a missing or duplicated Allocation Bitmap,
//! Up-case Table or Volume Label entry, an up-case table that fails its
//! checksum or the mandatory mapping, entry sets with a bad checksum, name
//! hash, name or layout, duplicate names, directories whose sizes are not
//! whole clusters, chains that loop, leave the heap, end early or run on,
//! cross-links, and a bitmap that differs from the clusters the tree uses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::fat::MUTABLE_ATTRS;
use crate::fat::model::{EntryState, FsState};
use crate::fat::spec::ImageGeometry;
use crate::harness::join_path;
use crate::harness::tree::EntryData;

const FAT_MEDIA: u32 = 0xFFFF_FFF8;
const FAT_END: u32 = 0xFFFF_FFFF;
const FAT_BAD: u32 = 0xFFFF_FFF7;
const MAX_DIRECTORY: u64 = 256 << 20;
const ATTR_DIRECTORY: u16 = 0x10;
const NO_FAT_CHAIN: u8 = 0x02;
const ALLOCATION_POSSIBLE: u8 = 0x01;
/// Characters the specification forbids in names, besides controls.
const FORBIDDEN: [u16; 9] = [0x22, 0x2A, 0x2F, 0x3A, 0x3C, 0x3E, 0x3F, 0x5C, 0x7C];

struct Geometry {
    sector: usize,
    cluster: usize,
    volume_len: usize,
    fat_start: usize,
    heap_start: usize,
    cluster_count: u32,
    root: u32,
    flags: u16,
    percent: u8,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    let raw = slice(bytes, at, 2)?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    let raw = slice(bytes, at, 4)?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn u64_at(bytes: &[u8], at: usize) -> Result<u64, String> {
    let raw = slice(bytes, at, 8)?;
    let mut value = [0u8; 8];
    value.copy_from_slice(raw);
    Ok(u64::from_le_bytes(value))
}

fn slice(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    at.checked_add(len)
        .and_then(|end| bytes.get(at..end))
        .ok_or_else(|| format!("read of {len} bytes at {at:#x} is past the image"))
}

fn parse_geometry(bytes: &[u8]) -> Result<Geometry, String> {
    if slice(bytes, 3, 8)? != b"EXFAT   " {
        return Err("FileSystemName is not EXFAT".into());
    }
    if u16_at(bytes, 510)? != 0xAA55 {
        return Err("missing boot signature".into());
    }
    if slice(bytes, 11, 53)?.iter().any(|&byte| byte != 0) {
        return Err("MustBeZero is not zero".into());
    }
    let sector_shift = *slice(bytes, 108, 1)?.first().unwrap_or(&0) as u32;
    let cluster_shift = sector_shift + *slice(bytes, 109, 1)?.first().unwrap_or(&0) as u32;
    if !(9..=12).contains(&sector_shift) || cluster_shift > 25 {
        return Err(format!("bad shifts {sector_shift}/{cluster_shift}"));
    }
    if slice(bytes, 110, 1)?[0] != 1 {
        return Err("NumberOfFats is not 1".into());
    }
    if u16_at(bytes, 104)? != 0x0100 {
        return Err("FileSystemRevision is not 1.00".into());
    }
    let sector = 1usize << sector_shift;
    let volume_len = u64_at(bytes, 72)? as usize * sector;
    let fat_offset = u32_at(bytes, 80)? as usize;
    let fat_len = u32_at(bytes, 84)? as usize;
    let heap_offset = u32_at(bytes, 88)? as usize;
    let cluster_count = u32_at(bytes, 92)?;
    if volume_len > bytes.len() {
        return Err(format!(
            "VolumeLength {volume_len} exceeds the image of {}",
            bytes.len()
        ));
    }
    if fat_offset < 24 || fat_offset + fat_len > heap_offset {
        return Err("FAT region overlaps the boot region or the heap".into());
    }
    if (cluster_count as usize + 2) * 4 > fat_len * sector {
        return Err("FatLength is too small for ClusterCount".into());
    }
    let cluster = 1usize << cluster_shift;
    if heap_offset * sector + cluster_count as usize * cluster > volume_len {
        return Err("the cluster heap runs past VolumeLength".into());
    }
    let root = u32_at(bytes, 96)?;
    if root < 2 || root > cluster_count + 1 {
        return Err(format!(
            "FirstClusterOfRootDirectory {root} is outside the heap"
        ));
    }
    Ok(Geometry {
        sector,
        cluster,
        volume_len,
        fat_start: fat_offset * sector,
        heap_start: heap_offset * sector,
        cluster_count,
        root,
        flags: u16_at(bytes, 106)?,
        percent: slice(bytes, 112, 1)?[0],
    })
}

fn boot_checksum(bytes: &[u8], sector: usize) -> u32 {
    let mut sum = 0u32;
    for (at, &byte) in bytes[..11 * sector].iter().enumerate() {
        if at == 106 || at == 107 || at == 112 {
            continue;
        }
        sum = (if sum & 1 != 0 { 0x8000_0000u32 } else { 0 })
            .wrapping_add(sum >> 1)
            .wrapping_add(byte as u32);
    }
    sum
}

fn validate_boot_region(bytes: &[u8], geo: &Geometry) -> Result<(), String> {
    let sector = geo.sector;
    let region = slice(bytes, 0, 24 * sector)?;
    let sum = boot_checksum(region, sector);
    for index in 0..sector / 4 {
        if u32_at(region, 11 * sector + index * 4)? != sum {
            return Err(format!(
                "boot checksum word {index} does not match {sum:#010x}"
            ));
        }
    }
    for index in 1..=8 {
        if u32_at(region, (index + 1) * sector - 4)? != 0xAA55_0000 {
            return Err(format!("extended boot sector {index} lacks its signature"));
        }
    }
    for at in 0..12 * sector {
        if (at == 106 || at == 107 || at == 112) || region[at] == region[12 * sector + at] {
            continue;
        }
        return Err(format!("backup boot region differs at byte {at}"));
    }
    if geo.flags & 0x0002 != 0 {
        return Err("VolumeDirty is set".into());
    }
    if geo.flags & 0x0001 != 0 {
        return Err("ActiveFat selects a second FAT".into());
    }
    Ok(())
}

struct Oracle<'a> {
    bytes: &'a [u8],
    geo: Geometry,
    claimed: BTreeMap<u32, String>,
    upcase: Vec<u16>,
}

impl Oracle<'_> {
    fn fat(&self, cluster: u32) -> Result<u32, String> {
        u32_at(self.bytes, self.geo.fat_start + cluster as usize * 4)
    }

    fn cluster_at(&self, cluster: u32) -> Result<&[u8], String> {
        if cluster < 2 || cluster > self.geo.cluster_count + 1 {
            return Err(format!("cluster {cluster} is outside the heap"));
        }
        slice(
            self.bytes,
            self.geo.heap_start + (cluster as usize - 2) * self.geo.cluster,
            self.geo.cluster,
        )
    }

    /// The clusters of an allocation of `len` bytes, claimed for `owner`.
    fn allocation(
        &mut self,
        first: u32,
        len: u64,
        contiguous: bool,
        owner: &str,
    ) -> Result<Vec<u32>, String> {
        let count = len.div_ceil(self.geo.cluster as u64) as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        if first < 2 || first > self.geo.cluster_count + 1 {
            return Err(format!("{owner} starts at invalid cluster {first}"));
        }
        let mut clusters = Vec::with_capacity(count);
        if contiguous {
            if first as usize + count - 1 > self.geo.cluster_count as usize + 1 {
                return Err(format!("contiguous {owner} runs past the heap"));
            }
            clusters.extend((0..count as u32).map(|step| first + step));
        } else {
            let mut cluster = first;
            let mut seen = BTreeSet::new();
            loop {
                if !seen.insert(cluster) {
                    return Err(format!("cluster loop at {cluster} in {owner}"));
                }
                clusters.push(cluster);
                let next = self.fat(cluster)?;
                if clusters.len() == count {
                    if next != FAT_END {
                        return Err(format!(
                            "{owner} chain continues past its {count} clusters to {next:#x}"
                        ));
                    }
                    break;
                }
                match next {
                    FAT_END => {
                        return Err(format!(
                            "{owner} chain ends after {} of {count} clusters",
                            clusters.len()
                        ));
                    }
                    FAT_BAD => return Err(format!("{owner} chain runs into a bad cluster")),
                    next if next < 2 || next > self.geo.cluster_count + 1 => {
                        return Err(format!("{owner} chain links to invalid cluster {next:#x}"));
                    }
                    next => cluster = next,
                }
            }
        }
        for &cluster in &clusters {
            if let Some(previous) = self.claimed.insert(cluster, owner.to_string()) {
                return Err(format!(
                    "cluster {cluster} is cross-linked between {previous} and {owner}"
                ));
            }
        }
        Ok(clusters)
    }

    fn read(&self, clusters: &[u32], len: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(clusters.len() * self.geo.cluster);
        for &cluster in clusters {
            out.extend_from_slice(self.cluster_at(cluster)?);
        }
        out.truncate(len);
        Ok(out)
    }

    /// The root chain, whose length only its FAT chain gives.
    fn root_chain(&mut self) -> Result<Vec<u32>, String> {
        let mut clusters = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cluster = self.geo.root;
        loop {
            if !seen.insert(cluster) {
                return Err(format!("cluster loop at {cluster} in the root directory"));
            }
            clusters.push(cluster);
            match self.fat(cluster)? {
                FAT_END => break,
                next if next < 2 || next > self.geo.cluster_count + 1 => {
                    return Err(format!("root directory chain links to {next:#x}"));
                }
                next => cluster = next,
            }
        }
        for &cluster in &clusters {
            if let Some(previous) = self.claimed.insert(cluster, "/".into()) {
                return Err(format!("root cluster {cluster} is also used by {previous}"));
            }
        }
        Ok(clusters)
    }

    fn upper(&self, unit: u16) -> u16 {
        self.upcase.get(unit as usize).copied().unwrap_or(unit)
    }

    fn read_directory(
        &mut self,
        bytes: &[u8],
        path: &str,
        output: &mut BTreeMap<String, EntryState>,
        system: &mut SystemEntries,
    ) -> Result<(), String> {
        let root = path == "/";
        let mut names: Vec<Vec<u16>> = Vec::new();
        let mut children = Vec::new();
        let entries: Vec<&[u8]> = bytes.chunks_exact(32).collect();
        let mut index = 0;
        let mut ended = false;
        while index < entries.len() {
            let entry = entries[index];
            let kind = entry[0];
            if ended {
                if kind != 0 {
                    return Err(format!(
                        "{path}: entry {index} follows the end of directory"
                    ));
                }
                index += 1;
                continue;
            }
            match kind {
                0x00 => ended = true,
                kind if kind & 0x80 == 0 => {}
                0x81..=0x83 if !root => {
                    return Err(format!("{path}: system entry {kind:#04x} outside the root"));
                }
                0x81 => system.bitmap.push(entry.to_vec()),
                0x82 => system.upcase.push(entry.to_vec()),
                0x83 => system.label.push(entry.to_vec()),
                0x85 => {
                    let count = 1 + entry[1] as usize;
                    if !(3..=19).contains(&count) || index + count > entries.len() {
                        return Err(format!(
                            "{path}: File entry {index} has SecondaryCount {}",
                            entry[1]
                        ));
                    }
                    let set: Vec<&[u8]> = entries[index..index + count].to_vec();
                    let (name, state, child) = self.file_set(&set, path)?;
                    let upcased: Vec<u16> = name.iter().map(|&unit| self.upper(unit)).collect();
                    if names.contains(&upcased) {
                        return Err(format!(
                            "{path}: duplicate name {}",
                            String::from_utf16_lossy(&name)
                        ));
                    }
                    names.push(upcased);
                    let child_path = join_path(path, &String::from_utf16_lossy(&name));
                    output.insert(child_path.clone(), state);
                    if let Some(child) = child {
                        children.push((child_path, child));
                    }
                    index += count;
                    continue;
                }
                kind if kind & 0x40 != 0 => {
                    return Err(format!(
                        "{path}: secondary entry {index} ({kind:#04x}) follows no primary"
                    ));
                }
                kind if kind & 0x20 != 0 => {
                    index += 1 + entry[1] as usize;
                    continue;
                }
                kind => {
                    return Err(format!(
                        "{path}: unknown critical primary entry {kind:#04x}"
                    ));
                }
            }
            index += 1;
        }
        for (child_path, contents) in children {
            self.read_directory(&contents, &child_path, output, system)?;
        }
        Ok(())
    }

    /// Checks a File entry set and returns its name, its model state and,
    /// for a directory, its contents.
    #[allow(clippy::type_complexity)]
    fn file_set(
        &mut self,
        set: &[&[u8]],
        parent: &str,
    ) -> Result<(Vec<u16>, EntryState, Option<Vec<u8>>), String> {
        let primary = set[0];
        let stream = set[1];
        if stream[0] != 0xC0 {
            return Err(format!(
                "{parent}: File entry is not followed by a Stream Extension"
            ));
        }
        let mut sum = 0u16;
        for (index, entry) in set.iter().enumerate() {
            for (at, &byte) in entry.iter().enumerate() {
                if index == 0 && (at == 2 || at == 3) {
                    continue;
                }
                sum = (if sum & 1 != 0 { 0x8000u16 } else { 0 })
                    .wrapping_add(sum >> 1)
                    .wrapping_add(byte as u16);
            }
        }
        if sum != u16_at(primary, 2)? {
            return Err(format!("{parent}: SetChecksum mismatch"));
        }
        let name_len = stream[3] as usize;
        let name_entries = name_len.div_ceil(15);
        if name_len == 0 || 2 + name_entries > set.len() {
            return Err(format!(
                "{parent}: NameLength {name_len} does not fit the set"
            ));
        }
        let mut name = Vec::with_capacity(name_len);
        for entry in &set[2..2 + name_entries] {
            if entry[0] != 0xC1 {
                return Err(format!(
                    "{parent}: expected a File Name entry, found {:#04x}",
                    entry[0]
                ));
            }
            for unit in 0..15 {
                if name.len() < name_len {
                    name.push(u16_at(entry, 2 + unit * 2)?);
                }
            }
        }
        for entry in &set[2 + name_entries..] {
            if entry[0] & 0xE0 != 0xE0 {
                return Err(format!(
                    "{parent}: critical secondary {:#04x} after the name",
                    entry[0]
                ));
            }
        }
        let text = String::from_utf16_lossy(&name);
        if name
            .iter()
            .any(|&unit| unit < 0x20 || FORBIDDEN.contains(&unit))
            || text == "."
            || text == ".."
        {
            return Err(format!("{parent}: invalid name {text:?}"));
        }
        let mut hash = 0u16;
        for &unit in &name {
            for byte in self.upper(unit).to_le_bytes() {
                hash = (if hash & 1 != 0 { 0x8000u16 } else { 0 })
                    .wrapping_add(hash >> 1)
                    .wrapping_add(byte as u16);
            }
        }
        if hash != u16_at(stream, 4)? {
            return Err(format!("{parent}: NameHash of {text:?} does not match"));
        }
        let path = join_path(parent, &text);
        let attributes = u16_at(primary, 4)?;
        let flags = stream[1];
        let valid = u64_at(stream, 8)?;
        let first = u32_at(stream, 20)?;
        let len = u64_at(stream, 24)?;
        if valid > len {
            return Err(format!(
                "{path}: ValidDataLength {valid} exceeds DataLength {len}"
            ));
        }
        if flags & ALLOCATION_POSSIBLE == 0 && (first != 0 || len != 0) {
            return Err(format!("{path}: an allocation without AllocationPossible"));
        }
        if len == 0 && first != 0 {
            return Err(format!("{path}: an empty allocation names cluster {first}"));
        }
        let clusters = self.allocation(first, len, flags & NO_FAT_CHAIN != 0, &path)?;
        for extra in &set[2 + name_entries..] {
            if extra[1] & ALLOCATION_POSSIBLE != 0 {
                let len = u64_at(extra, 24)?;
                self.allocation(u32_at(extra, 20)?, len, extra[1] & NO_FAT_CHAIN != 0, &path)?;
            }
        }
        let attrs = (attributes & MUTABLE_ATTRS as u16) as u8;
        if attributes & ATTR_DIRECTORY != 0 {
            if len == 0 || len % self.geo.cluster as u64 != 0 || len > MAX_DIRECTORY || valid != len
            {
                return Err(format!(
                    "{path}: directory DataLength {len} ValidDataLength {valid}"
                ));
            }
            let contents = self.read(&clusters, len as usize)?;
            let state = EntryState {
                data: EntryData::Directory,
                attrs,
            };
            return Ok((name, state, Some(contents)));
        }
        let mut contents = self.read(&clusters, valid as usize)?;
        contents.resize(len as usize, 0);
        let state = EntryState {
            data: EntryData::File(contents),
            attrs,
        };
        Ok((name, state, None))
    }
}

#[derive(Default)]
struct SystemEntries {
    bitmap: Vec<Vec<u8>>,
    upcase: Vec<Vec<u8>>,
    label: Vec<Vec<u8>>,
}

/// Decodes an up-case table, compressed or not, into 65536 mappings.
fn decode_upcase(table: &[u8]) -> Result<Vec<u16>, String> {
    let mut out: Vec<u16> = (0..=0xFFFFu32).map(|code| code as u16).collect();
    let units: Vec<u16> = table
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let mut code = 0usize;
    let mut index = 0;
    while index < units.len() && code <= 0xFFFF {
        let unit = units[index];
        if unit == 0xFFFF && code != 0xFFFF {
            let run = *units
                .get(index + 1)
                .ok_or("up-case table ends in a run marker")? as usize;
            code += run;
            index += 2;
            continue;
        }
        out[code] = unit;
        code += 1;
        index += 1;
    }
    Ok(out)
}

fn table_checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |sum, &byte| {
        (if sum & 1 != 0 { 0x8000_0000u32 } else { 0 })
            .wrapping_add(sum >> 1)
            .wrapping_add(byte as u32)
    })
}

fn volume(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| error.to_string())
}

/// The allocation bitmap's set bits, as cluster numbers.
fn allocated(geo: &Geometry, bitmap: &[u8]) -> Result<BTreeSet<u32>, String> {
    let mut used = BTreeSet::new();
    for index in 0..geo.cluster_count as usize {
        let byte = *bitmap
            .get(index / 8)
            .ok_or("the allocation bitmap is shorter than ClusterCount")?;
        if byte & (1 << (index % 8)) != 0 {
            used.insert(index as u32 + 2);
        }
    }
    Ok(used)
}

/// The parts of the boot sector that the limit exercises size themselves
/// from. exFAT has no fixed root directory.
pub fn geometry(path: &Path) -> Result<ImageGeometry, String> {
    let bytes = volume(path)?;
    let geo = parse_geometry(&bytes)?;
    Ok(ImageGeometry {
        bits: 0,
        cluster_size: geo.cluster,
        cluster_count: geo.cluster_count,
        root_entry_count: 0,
    })
}

/// Clusters the allocation bitmap marks free.
pub fn free_clusters(path: &Path) -> Result<u32, String> {
    let bytes = volume(path)?;
    let (report, _) = check(&bytes)?;
    Ok(report.cluster_count - report.used)
}

struct Summary {
    cluster_count: u32,
    used: u32,
}

/// Validates the image at `path` and returns its semantic tree.
pub fn snapshot(path: &Path) -> Result<FsState, String> {
    let bytes = volume(path)?;
    Ok(check(&bytes)?.1)
}

fn check(bytes: &[u8]) -> Result<(Summary, FsState), String> {
    let geo = parse_geometry(bytes)?;
    if geo.volume_len < 24 * geo.sector {
        return Err("the volume is smaller than its boot regions".into());
    }
    validate_boot_region(bytes, &geo)?;
    let mut oracle = Oracle {
        bytes,
        geo,
        claimed: BTreeMap::new(),
        upcase: Vec::new(),
    };
    if oracle.fat(0)? != FAT_MEDIA || oracle.fat(1)? != FAT_END {
        return Err("FAT entries 0 and 1 are not F8FFFFFF and FFFFFFFF".into());
    }
    let root = oracle.root_chain()?;
    let root_bytes = oracle.read(&root, root.len() * oracle.geo.cluster)?;
    let mut system = SystemEntries::default();
    for entry in root_bytes.chunks_exact(32) {
        match entry[0] {
            0x00 => break,
            0x81 => system.bitmap.push(entry.to_vec()),
            0x82 => system.upcase.push(entry.to_vec()),
            _ => {}
        }
    }
    let [upcase] = system.upcase.as_slice() else {
        return Err(format!(
            "{} Up-case Table entries in the root",
            system.upcase.len()
        ));
    };
    let upcase_len = u64_at(upcase, 24)?;
    let clusters =
        oracle.allocation(u32_at(upcase, 20)?, upcase_len, false, "the up-case table")?;
    let table = oracle.read(&clusters, upcase_len as usize)?;
    if table_checksum(&table) != u32_at(upcase, 4)? {
        return Err("the up-case table does not match its TableChecksum".into());
    }
    oracle.upcase = decode_upcase(&table)?;
    for code in 0..128u16 {
        let expected = if (0x61..=0x7A).contains(&code) {
            code - 0x20
        } else {
            code
        };
        if oracle.upcase[code as usize] != expected {
            return Err(format!(
                "the up-case table maps {code:#x} to {:#x}",
                oracle.upcase[code as usize]
            ));
        }
    }
    let [bitmap] = system.bitmap.as_slice() else {
        return Err(format!(
            "{} Allocation Bitmap entries in the root",
            system.bitmap.len()
        ));
    };
    if bitmap[1] & 1 != 0 {
        return Err("the Allocation Bitmap entry is marked as the second bitmap".into());
    }
    let bitmap_len = u64_at(bitmap, 24)?;
    if bitmap_len < (oracle.geo.cluster_count as u64).div_ceil(8) {
        return Err("the allocation bitmap is shorter than ClusterCount".into());
    }
    let clusters = oracle.allocation(
        u32_at(bitmap, 20)?,
        bitmap_len,
        false,
        "the allocation bitmap",
    )?;
    let bits = oracle.read(&clusters, bitmap_len as usize)?;

    let mut entries = BTreeMap::new();
    let mut all = SystemEntries::default();
    oracle.read_directory(&root_bytes, "/", &mut entries, &mut all)?;
    if all.bitmap.len() != 1 || all.upcase.len() != 1 || all.label.len() > 1 {
        return Err("the root holds a system entry more than once".into());
    }
    let label = match all.label.first() {
        None => String::new(),
        Some(entry) => {
            let count = entry[1] as usize;
            if count > 11 {
                return Err(format!("the volume label has {count} characters"));
            }
            let units: Vec<u16> = (0..count)
                .map(|index| u16_at(entry, 2 + index * 2))
                .collect::<Result<_, _>>()?;
            String::from_utf16_lossy(&units)
        }
    };

    let used = allocated(&oracle.geo, &bits)?;
    let claimed: BTreeSet<u32> = oracle.claimed.keys().copied().collect();
    if let Some(cluster) = used.difference(&claimed).next() {
        return Err(format!(
            "allocated cluster {cluster} is not owned by any entry"
        ));
    }
    if let Some(cluster) = claimed.difference(&used).next() {
        return Err(format!(
            "cluster {cluster} of {} is free in the allocation bitmap",
            oracle.claimed[cluster]
        ));
    }
    let count = oracle.geo.cluster_count;
    let floor = (used.len() as u64 * 100 / count as u64) as u8;
    let ceil = (used.len() as u64 * 100).div_ceil(count as u64) as u8;
    let percent = oracle.geo.percent;
    if percent != 0xFF && percent != floor && percent != ceil {
        return Err(format!(
            "PercentInUse is {percent}, the bitmap says {floor}"
        ));
    }
    let summary = Summary {
        cluster_count: count,
        used: used.len() as u32,
    };
    Ok((summary, FsState { label, entries }))
}
