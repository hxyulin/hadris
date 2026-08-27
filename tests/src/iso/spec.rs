//! Test-only raw-image oracle derived from the ECMA-119:1987 primary-volume
//! rules.
//!
//! It checks the descriptor sequence, redundant endian fields, volume bounds,
//! both path tables, the directory hierarchy, record bounds and padding,
//! Level 1 identifiers, file extents, and file content without help from any
//! implementation under test.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::SECTOR_SIZE;
use super::model::IsoState;
use crate::harness::join_path;
use crate::harness::tree::EntryData;

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathRecord {
    name: Vec<u8>,
    extent: u32,
    parent: u16,
}

#[derive(Clone, Debug)]
struct DirectoryInfo {
    identifier: Vec<u8>,
    extent: u32,
    parent: String,
}

#[derive(Clone, Debug)]
struct RawRecord {
    extent: u32,
    data_len: u32,
    flags: u8,
    extended_attr_blocks: u8,
    identifier: Vec<u8>,
}

pub fn snapshot(bytes: &[u8]) -> Result<IsoState, String> {
    let pvd = find_primary_descriptor(bytes)?;
    let volume_blocks = both_u32(pvd, 80, "volume space size")? as usize;
    let block_size = both_u16(pvd, 128, "logical block size")? as usize;
    if block_size != SECTOR_SIZE {
        return Err(format!(
            "logical block size is {block_size}, expected {SECTOR_SIZE}"
        ));
    }
    let volume_len = volume_blocks
        .checked_mul(block_size)
        .ok_or_else(|| "volume size overflows usize".to_string())?;
    if volume_len > bytes.len() {
        return Err(format!(
            "declared volume length {volume_len} exceeds image length {}",
            bytes.len()
        ));
    }
    let path_table_size = both_u32(pvd, 132, "path table size")? as usize;
    let little_path_lba = le_u32(pvd, 140)?;
    let big_path_lba = be_u32(pvd, 148)?;
    let little = parse_path_table(bytes, little_path_lba, path_table_size, false, volume_len)?;
    let big = parse_path_table(bytes, big_path_lba, path_table_size, true, volume_len)?;
    if little != big {
        return Err("little- and big-endian path tables differ".to_string());
    }

    let root = parse_record(&pvd[156..], 156)?;
    if root.identifier != [0] || root.flags & 0x02 == 0 {
        return Err("PVD root record is not the root directory".to_string());
    }
    let volume_id = ascii_field(&pvd[40..72])?;
    let mut entries = BTreeMap::new();
    let mut directories = BTreeMap::new();
    directories.insert(
        "/".to_string(),
        DirectoryInfo {
            identifier: vec![0],
            extent: root.extent,
            parent: "/".to_string(),
        },
    );
    let mut visited = BTreeSet::new();
    read_directory(
        bytes,
        volume_len,
        &root,
        "/",
        root.extent,
        &mut visited,
        &mut directories,
        &mut entries,
    )?;
    validate_path_table(&little, &directories)?;
    Ok(IsoState { volume_id, entries })
}

fn find_primary_descriptor(bytes: &[u8]) -> Result<&[u8], String> {
    let mut primary = None;
    for sector in 16..=255 {
        let descriptor = image_slice(bytes, sector * SECTOR_SIZE, SECTOR_SIZE, bytes.len())?;
        if &descriptor[1..6] != b"CD001" || descriptor[6] != 1 {
            return Err(format!(
                "invalid volume descriptor header at sector {sector}"
            ));
        }
        match descriptor[0] {
            1 => {
                if primary.replace(descriptor).is_some() {
                    return Err("multiple primary volume descriptors".to_string());
                }
            }
            255 => {
                if descriptor[7..].iter().any(|byte| *byte != 0) {
                    return Err("volume descriptor terminator body is not zero-filled".to_string());
                }
                return primary.ok_or_else(|| "descriptor sequence has no primary".to_string());
            }
            0..=3 => {}
            kind => return Err(format!("invalid volume descriptor type {kind}")),
        }
    }
    Err("descriptor sequence has no terminator".to_string())
}

fn parse_record(bytes: &[u8], image_offset: usize) -> Result<RawRecord, String> {
    let len = *bytes
        .first()
        .ok_or_else(|| format!("missing directory record at byte {image_offset}"))?
        as usize;
    if len < 34 || len > bytes.len() {
        return Err(format!(
            "invalid directory record length {len} at byte {image_offset}"
        ));
    }
    let name_len = bytes[32] as usize;
    let minimum = 33 + name_len + usize::from(name_len.is_multiple_of(2));
    if name_len == 0 || len < minimum {
        return Err(format!(
            "directory record at byte {image_offset} has invalid identifier bounds"
        ));
    }
    if name_len.is_multiple_of(2) && bytes[33 + name_len] != 0 {
        return Err(format!(
            "directory record at byte {image_offset} has nonzero identifier padding"
        ));
    }
    if bytes[25] & 0x60 != 0 {
        return Err(format!(
            "directory record at byte {image_offset} sets reserved flags"
        ));
    }
    if bytes[26] != 0 || bytes[27] != 0 {
        return Err(format!(
            "directory record at byte {image_offset} is interleaved"
        ));
    }
    if both_u16(bytes, 28, "directory volume sequence")? != 1 {
        return Err(format!(
            "directory record at byte {image_offset} is not on volume one"
        ));
    }
    Ok(RawRecord {
        extended_attr_blocks: bytes[1],
        extent: both_u32(bytes, 2, "directory extent")?,
        data_len: both_u32(bytes, 10, "directory data length")?,
        flags: bytes[25],
        identifier: bytes[33..33 + name_len].to_vec(),
    })
}

#[allow(clippy::too_many_arguments)]
fn read_directory(
    image: &[u8],
    volume_len: usize,
    record: &RawRecord,
    path: &str,
    parent_extent: u32,
    visited: &mut BTreeSet<u32>,
    directories: &mut BTreeMap<String, DirectoryInfo>,
    entries: &mut BTreeMap<String, EntryData>,
) -> Result<(), String> {
    if !visited.insert(record.extent) {
        return Err(format!(
            "directory extent {} is reused at {path}",
            record.extent
        ));
    }
    let data_lba = record.extent + u32::from(record.extended_attr_blocks);
    let start = data_lba as usize * SECTOR_SIZE;
    let data = image_slice(image, start, record.data_len as usize, volume_len)?;
    let mut offset = 0;
    let mut ordinary = Vec::new();
    let mut position = 0;
    while offset < data.len() {
        if data[offset] == 0 {
            let next = ((offset / SECTOR_SIZE) + 1) * SECTOR_SIZE;
            if data[offset..next.min(data.len())]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(format!("directory {path} has nonzero sector padding"));
            }
            offset = next;
            continue;
        }
        let len = data[offset] as usize;
        if offset % SECTOR_SIZE + len > SECTOR_SIZE {
            return Err(format!(
                "directory record in {path} crosses a logical-sector boundary"
            ));
        }
        let raw = parse_record(&data[offset..], start + offset)?;
        match position {
            0 if raw.identifier == [0] && raw.flags & 0x02 != 0 && raw.extent == record.extent => {}
            1 if raw.identifier == [1] && raw.flags & 0x02 != 0 && raw.extent == parent_extent => {}
            0 | 1 => return Err(format!("directory {path} has invalid dot records")),
            _ if raw.identifier == [0] || raw.identifier == [1] => {
                return Err(format!("directory {path} repeats a dot record"));
            }
            _ => ordinary.push(raw),
        }
        position += 1;
        offset += len;
    }
    if position < 2 {
        return Err(format!(
            "directory {path} does not contain dot and dot-dot records"
        ));
    }

    for child in ordinary {
        if child.flags & 0x84 != 0 {
            return Err(format!("entry in {path} is associated or multi-extent"));
        }
        let name = decode_primary_identifier(&child.identifier, child.flags & 0x02 != 0)?;
        let child_path = join_path(path, &name);
        if child.flags & 0x02 != 0 {
            if child.data_len == 0 {
                return Err(format!("directory {child_path} has zero length"));
            }
            if entries
                .insert(child_path.clone(), EntryData::Directory)
                .is_some()
            {
                return Err(format!("duplicate entry {child_path}"));
            }
            directories.insert(
                child_path.clone(),
                DirectoryInfo {
                    identifier: child.identifier.clone(),
                    extent: child.extent,
                    parent: path.to_string(),
                },
            );
            read_directory(
                image,
                volume_len,
                &child,
                &child_path,
                record.extent,
                visited,
                directories,
                entries,
            )?;
        } else {
            let data_lba = child.extent + u32::from(child.extended_attr_blocks);
            let contents = if child.data_len == 0 {
                Vec::new()
            } else {
                image_slice(
                    image,
                    data_lba as usize * SECTOR_SIZE,
                    child.data_len as usize,
                    volume_len,
                )?
                .to_vec()
            };
            if entries
                .insert(child_path.clone(), EntryData::File(contents))
                .is_some()
            {
                return Err(format!("duplicate entry {child_path}"));
            }
        }
    }
    Ok(())
}

fn decode_primary_identifier(identifier: &[u8], directory: bool) -> Result<String, String> {
    if !identifier.is_ascii() {
        return Err(format!("non-ASCII primary identifier {identifier:?}"));
    }
    let text = std::str::from_utf8(identifier).map_err(|error| error.to_string())?;
    if directory {
        if text.len() > 8 || text.is_empty() || !text.bytes().all(is_d_character) {
            return Err(format!("invalid level-1 directory identifier {text:?}"));
        }
        return Ok(text.to_string());
    }
    let (name, version) = text
        .rsplit_once(';')
        .ok_or_else(|| format!("file identifier {text:?} has no version"))?;
    if version != "1" {
        return Err(format!("file identifier {text:?} has invalid version"));
    }
    let (stem, extension) = name.split_once('.').unwrap_or((name, ""));
    if stem.is_empty()
        || stem.len() > 8
        || extension.len() > 3
        || !stem.bytes().all(is_d_character)
        || !extension.bytes().all(is_d_character)
    {
        return Err(format!("invalid level-1 file identifier {text:?}"));
    }
    Ok(name.to_string())
}

fn is_d_character(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
}

fn parse_path_table(
    image: &[u8],
    lba: u32,
    size: usize,
    big_endian: bool,
    volume_len: usize,
) -> Result<Vec<PathRecord>, String> {
    if lba == 0 || size == 0 {
        return Err("primary path table is absent".to_string());
    }
    let data = image_slice(image, lba as usize * SECTOR_SIZE, size, volume_len)?;
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        if data.len() - offset < 8 {
            return Err("truncated path table record".to_string());
        }
        let name_len = data[offset] as usize;
        let record_len = 8 + name_len + usize::from(!name_len.is_multiple_of(2));
        if name_len == 0 || record_len > data.len() - offset {
            return Err("invalid path table identifier length".to_string());
        }
        let extent = if big_endian {
            be_u32(data, offset + 2)?
        } else {
            le_u32(data, offset + 2)?
        };
        let parent = if big_endian {
            be_u16(data, offset + 6)?
        } else {
            le_u16(data, offset + 6)?
        };
        records.push(PathRecord {
            name: data[offset + 8..offset + 8 + name_len].to_vec(),
            extent,
            parent,
        });
        offset += record_len;
    }
    Ok(records)
}

fn validate_path_table(
    actual: &[PathRecord],
    directories: &BTreeMap<String, DirectoryInfo>,
) -> Result<(), String> {
    let root = directories
        .get("/")
        .ok_or_else(|| "root directory was not recorded".to_string())?;
    let mut expected = vec![PathRecord {
        name: root.identifier.clone(),
        extent: root.extent,
        parent: 1,
    }];
    let mut queue = VecDeque::from(["/".to_string()]);
    while let Some(parent_path) = queue.pop_front() {
        let parent_index = expected
            .iter()
            .position(|record| {
                directories
                    .get(&parent_path)
                    .is_some_and(|directory| directory.extent == record.extent)
            })
            .ok_or_else(|| format!("missing path table parent {parent_path}"))?
            + 1;
        let mut children: Vec<_> = directories
            .iter()
            .filter(|(path, directory)| path.as_str() != "/" && directory.parent == parent_path)
            .collect();
        children.sort_by(|left, right| left.1.identifier.cmp(&right.1.identifier));
        for (path, directory) in children {
            expected.push(PathRecord {
                name: directory.identifier.clone(),
                extent: directory.extent,
                parent: parent_index as u16,
            });
            queue.push_back(path.clone());
        }
    }
    if actual != expected {
        return Err(format!(
            "path table mismatch\nexpected: {expected:?}\nactual: {actual:?}"
        ));
    }
    Ok(())
}

fn ascii_field(bytes: &[u8]) -> Result<String, String> {
    if !bytes.is_ascii() {
        return Err("non-ASCII primary volume field".to_string());
    }
    Ok(std::str::from_utf8(bytes)
        .map_err(|error| error.to_string())?
        .trim_end_matches(' ')
        .to_string())
}

fn image_slice(bytes: &[u8], offset: usize, len: usize, limit: usize) -> Result<&[u8], String> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| "image range overflows usize".to_string())?;
    if end > limit || end > bytes.len() {
        return Err(format!("image range {offset}..{end} is out of bounds"));
    }
    Ok(&bytes[offset..end])
}

fn both_u16(bytes: &[u8], offset: usize, field: &str) -> Result<u16, String> {
    let little = le_u16(bytes, offset)?;
    let big = be_u16(bytes, offset + 2)?;
    if little != big {
        return Err(format!("{field} endian copies differ: {little} != {big}"));
    }
    Ok(little)
}

fn both_u32(bytes: &[u8], offset: usize, field: &str) -> Result<u32, String> {
    let little = le_u32(bytes, offset)?;
    let big = be_u32(bytes, offset + 4)?;
    if little != big {
        return Err(format!("{field} endian copies differ: {little} != {big}"));
    }
    Ok(little)
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let raw = image_slice(bytes, offset, 2, bytes.len())?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn be_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let raw = image_slice(bytes, offset, 2, bytes.len())?;
    Ok(u16::from_be_bytes([raw[0], raw[1]]))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw = image_slice(bytes, offset, 4, bytes.len())?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw = image_slice(bytes, offset, 4, bytes.len())?;
    Ok(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
}
