//! RRIP reader support
//!
//! This module provides RRIP (Rock Ridge) metadata reading, CE continuation
//! area following, and RRIP-aware directory iteration.

use alloc::string::String;
use alloc::vec::Vec;

use crate::directory::{DirectoryRecord, DirectoryRef};
use crate::io::{self, IsoCursor, LogicalSector, Read, Seek, SeekFrom};
use crate::rrip::{NmFlags, PnEntry, PxEntry, SlComponentFlags, TfFlags};
use crate::susp::{ContinuationArea, SystemUseField, SystemUseIter};

use super::IsoImage;

/// Information about SUSP/RRIP presence detected from the root directory.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SuspInfo {
    /// Whether SUSP was detected (SP entry found).
    pub detected: bool,
    /// Number of bytes to skip at the start of each system use area (from SP entry).
    pub bytes_skipped: u8,
    /// Whether RRIP extensions were detected (ER entry with RRIP identifier found).
    pub rrip_detected: bool,
}

/// Detect SUSP and RRIP presence by examining the root directory's "." entry.
///
/// According to the SUSP spec, the SP entry (if present) must appear in the
/// system use area of the root directory's "." entry. The ER entry declaring
/// RRIP should also be present (possibly in a CE continuation area).
pub(crate) fn detect_susp_rrip<DATA: Read + Seek>(
    data: &mut IsoCursor<DATA>,
    root_extent: LogicalSector,
) -> io::Result<SuspInfo> {
    let mut info = SuspInfo::default();

    // Seek to root directory and parse the first record ("." entry)
    data.seek(SeekFrom::Start(root_extent.0 as u64 * 2048))?;
    let dot_record = DirectoryRecord::parse(data)?;
    let su = dot_record.system_use();
    if su.is_empty() {
        return Ok(info);
    }

    // First pass: look for SP entry and inline ER/CE
    let mut ce_entry: Option<ContinuationArea> = None;

    for field in SystemUseIter::new(su, 0) {
        match &field {
            SystemUseField::SuspIdentifier(sp) => {
                if sp.is_valid() {
                    info.detected = true;
                    info.bytes_skipped = sp.bytes_skipped;
                }
            }
            SystemUseField::ExtensionReference(er) => {
                if is_rrip_identifier(er) {
                    info.rrip_detected = true;
                    return Ok(info);
                }
            }
            SystemUseField::ContinuationArea(ce) => {
                ce_entry = Some(*ce);
            }
            SystemUseField::Terminator => break,
            _ => {}
        }
    }

    // If we found SP but not ER inline, follow CE continuation areas
    if info.detected && !info.rrip_detected {
        let mut depth = 0;
        while let Some(ce) = ce_entry.take() {
            depth += 1;
            if depth > 16 {
                break;
            }

            let ce_offset = ce.sector.read() as u64 * 2048 + ce.offset.read() as u64;
            let ce_len = ce.length.read() as usize;
            if ce_len == 0 || ce_len > 1024 * 1024 {
                break;
            }

            let mut ce_buf = alloc::vec![0u8; ce_len];
            data.seek(SeekFrom::Start(ce_offset))?;
            data.read_exact(&mut ce_buf)?;

            for field in SystemUseIter::new(&ce_buf, 0) {
                match &field {
                    SystemUseField::ExtensionReference(er) => {
                        if is_rrip_identifier(er) {
                            info.rrip_detected = true;
                            return Ok(info);
                        }
                    }
                    SystemUseField::ContinuationArea(next_ce) => {
                        ce_entry = Some(*next_ce);
                    }
                    SystemUseField::Terminator => break,
                    _ => {}
                }
            }
        }
    }

    Ok(info)
}

/// Check if an ExtensionReference entry identifies RRIP.
fn is_rrip_identifier(er: &crate::susp::ExtensionReference) -> bool {
    let id_len = er.identifier_len as usize;
    if id_len == 0 || 4 + id_len > er.buf.len() {
        return false;
    }
    let identifier = &er.buf[4..4 + id_len];
    matches!(
        identifier,
        b"RRIP_1991A" | b"IEEE_P1282" | b"IEEE_1282"
    )
}

/// Collect all system use entries from a directory record, following CE
/// continuation area chains.
///
/// This function parses the inline system use area and then reads any
/// continuation areas referenced by CE entries. CE chains are followed
/// with a depth limit of 16 and a size limit of 1MB per allocation.
pub fn collect_su_entries<DATA: Read + Seek>(
    record: &DirectoryRecord,
    image: &IsoImage<DATA>,
    bytes_to_skip: u8,
) -> io::Result<Vec<SystemUseField>> {
    let su = record.system_use();
    let mut fields = Vec::new();
    let mut ce_entry: Option<ContinuationArea> = None;

    for field in SystemUseIter::new(su, bytes_to_skip as usize) {
        match &field {
            SystemUseField::ContinuationArea(ce) => {
                ce_entry = Some(*ce);
            }
            SystemUseField::Terminator => {
                fields.push(field);
                return Ok(fields);
            }
            _ => {}
        }
        fields.push(field);
    }

    // Follow CE chains
    let mut depth = 0;
    while let Some(ce) = ce_entry.take() {
        depth += 1;
        if depth > 16 {
            break;
        }

        let ce_offset = ce.sector.read() as u64 * 2048 + ce.offset.read() as u64;
        let ce_len = ce.length.read() as usize;
        if ce_len == 0 || ce_len > 1024 * 1024 {
            break;
        }

        let mut ce_buf = alloc::vec![0u8; ce_len];
        image.read_bytes_at(ce_offset, &mut ce_buf)?;

        for field in SystemUseIter::new(&ce_buf, 0) {
            match &field {
                SystemUseField::ContinuationArea(next_ce) => {
                    ce_entry = Some(*next_ce);
                }
                SystemUseField::Terminator => {
                    fields.push(field);
                    return Ok(fields);
                }
                _ => {}
            }
            fields.push(field);
        }
    }

    Ok(fields)
}

// ── RRIP Metadata Types ──

/// A parsed date/time from RRIP TF entries.
#[derive(Debug, Clone, Copy)]
pub struct RripDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// GMT offset in 15-minute intervals (-48..+52)
    pub gmt_offset: i8,
}

/// Parsed RRIP timestamps from a TF entry.
#[derive(Debug, Clone, Default)]
pub struct RripTimestamps {
    pub creation: Option<RripDateTime>,
    pub modify: Option<RripDateTime>,
    pub access: Option<RripDateTime>,
    pub attributes: Option<RripDateTime>,
    pub backup: Option<RripDateTime>,
    pub expiration: Option<RripDateTime>,
    pub effective: Option<RripDateTime>,
}

/// Assembled RRIP metadata from system use entries.
#[derive(Debug, Clone, Default)]
pub struct RripMetadata {
    pub posix_attributes: Option<PxEntry>,
    pub device_number: Option<PnEntry>,
    pub alternate_name: Option<String>,
    pub symlink_target: Option<String>,
    pub timestamps: Option<RripTimestamps>,
    /// CL - sector of the relocated child directory
    pub child_link: Option<u32>,
    /// PL - sector of the original parent directory
    pub parent_link: Option<u32>,
    /// RE - this entry is a relocated directory placeholder
    pub is_relocated: bool,
}

impl RripMetadata {
    /// Assemble RRIP metadata from a list of system use fields.
    pub fn from_fields(fields: &[SystemUseField]) -> Self {
        let mut meta = Self::default();

        // Collect NM name fragments
        let mut nm_parts: Vec<&[u8]> = Vec::new();
        let mut nm_is_current = false;
        let mut nm_is_parent = false;

        // Collect SL components across entries
        let mut sl_components: Vec<&crate::rrip::SlComponent> = Vec::new();
        let mut has_sl = false;

        for field in fields {
            match field {
                SystemUseField::PosixAttributes(px) => {
                    meta.posix_attributes = Some(*px);
                }
                SystemUseField::DeviceNumber(pn) => {
                    meta.device_number = Some(*pn);
                }
                SystemUseField::AlternateName(nm) => {
                    if nm.flags.contains(NmFlags::CURRENT) {
                        nm_is_current = true;
                    } else if nm.flags.contains(NmFlags::PARENT) {
                        nm_is_parent = true;
                    } else if !nm.name.is_empty() {
                        nm_parts.push(&nm.name);
                    }
                }
                SystemUseField::SymbolicLink(sl) => {
                    has_sl = true;
                    for comp in &sl.components {
                        sl_components.push(comp);
                    }
                }
                SystemUseField::Timestamps(tf) => {
                    meta.timestamps = Some(parse_tf_timestamps(tf));
                }
                SystemUseField::ChildLink(cl) => {
                    meta.child_link = Some(cl.child_directory_location.read());
                }
                SystemUseField::ParentLink(pl) => {
                    meta.parent_link = Some(pl.parent_directory_location.read());
                }
                SystemUseField::Relocated => {
                    meta.is_relocated = true;
                }
                _ => {}
            }
        }

        // Assemble NM name
        if nm_is_current {
            meta.alternate_name = Some(String::from("."));
        } else if nm_is_parent {
            meta.alternate_name = Some(String::from(".."));
        } else if !nm_parts.is_empty() {
            let total_len: usize = nm_parts.iter().map(|p| p.len()).sum();
            let mut name_bytes = Vec::with_capacity(total_len);
            for part in &nm_parts {
                name_bytes.extend_from_slice(part);
            }
            meta.alternate_name = Some(String::from_utf8_lossy(&name_bytes).into_owned());
        }

        // Assemble SL symlink path
        if has_sl {
            meta.symlink_target = Some(assemble_symlink_path(&sl_components));
        }

        meta
    }
}

/// Parse TF timestamps from raw bytes based on flags.
fn parse_tf_timestamps(tf: &crate::rrip::TfEntry) -> RripTimestamps {
    let mut ts = RripTimestamps::default();
    let long_form = tf.flags.contains(TfFlags::LONG_FORM);
    let stamp_size = if long_form { 17 } else { 7 };
    let data = &tf.timestamps;
    let mut offset = 0;

    let flags_in_order = [
        TfFlags::CREATION,
        TfFlags::MODIFY,
        TfFlags::ACCESS,
        TfFlags::ATTRIBUTES,
        TfFlags::BACKUP,
        TfFlags::EXPIRATION,
        TfFlags::EFFECTIVE,
    ];

    // Parse timestamps in order, assigning to the correct field
    let mut parsed: [Option<RripDateTime>; 7] = [None; 7];
    for (i, flag) in flags_in_order.iter().enumerate() {
        if tf.flags.contains(*flag) {
            if offset + stamp_size <= data.len() {
                parsed[i] = Some(if long_form {
                    parse_long_timestamp(&data[offset..offset + 17])
                } else {
                    parse_short_timestamp(&data[offset..offset + 7])
                });
                offset += stamp_size;
            }
        }
    }

    ts.creation = parsed[0];
    ts.modify = parsed[1];
    ts.access = parsed[2];
    ts.attributes = parsed[3];
    ts.backup = parsed[4];
    ts.expiration = parsed[5];
    ts.effective = parsed[6];

    ts
}

/// Parse a 7-byte short-form ISO 9660 timestamp.
fn parse_short_timestamp(data: &[u8]) -> RripDateTime {
    RripDateTime {
        year: 1900 + data[0] as u16,
        month: data[1],
        day: data[2],
        hour: data[3],
        minute: data[4],
        second: data[5],
        gmt_offset: data[6] as i8,
    }
}

/// Parse a 17-byte long-form ISO 9660 timestamp (ASCII "YYYYMMDDHHMMSScc" + gmt_offset).
fn parse_long_timestamp(data: &[u8]) -> RripDateTime {
    let parse_num = |start: usize, len: usize| -> u16 {
        let s = core::str::from_utf8(&data[start..start + len]).unwrap_or("0");
        s.parse().unwrap_or(0)
    };

    RripDateTime {
        year: parse_num(0, 4),
        month: parse_num(4, 2) as u8,
        day: parse_num(6, 2) as u8,
        hour: parse_num(8, 2) as u8,
        minute: parse_num(10, 2) as u8,
        second: parse_num(12, 2) as u8,
        gmt_offset: data[16] as i8,
    }
}

/// Assemble a symlink path from SL components.
fn assemble_symlink_path(components: &[&crate::rrip::SlComponent]) -> String {
    let mut path = String::new();
    let mut pending_content: Vec<u8> = Vec::new();
    let mut first = true;

    for comp in components {
        if comp.flags.contains(SlComponentFlags::ROOT) {
            path.push('/');
            first = false;
        } else if comp.flags.contains(SlComponentFlags::CURRENT) {
            if !first {
                path.push('/');
            }
            path.push('.');
            first = false;
        } else if comp.flags.contains(SlComponentFlags::PARENT) {
            if !first {
                path.push('/');
            }
            path.push_str("..");
            first = false;
        } else {
            // Regular component content
            pending_content.extend_from_slice(&comp.content);

            // If CONTINUE flag is set, more content follows for this component
            if comp.flags.contains(SlComponentFlags::CONTINUE) {
                continue;
            }

            // Emit the component
            if !first && !path.ends_with('/') {
                path.push('/');
            }
            path.push_str(&String::from_utf8_lossy(&pending_content));
            pending_content.clear();
            first = false;
        }
    }

    // Flush any remaining content
    if !pending_content.is_empty() {
        if !first && !path.ends_with('/') {
            path.push('/');
        }
        path.push_str(&String::from_utf8_lossy(&pending_content));
    }

    path
}

/// Read the data_len of a directory by reading its "." entry.
pub(crate) fn read_dir_size<DATA: Read + Seek>(
    image: &IsoImage<DATA>,
    sector: LogicalSector,
) -> io::Result<DirectoryRef> {
    let byte_offset = sector.0 as u64 * 2048;
    let mut buf = [0u8; 34];
    image.read_bytes_at(byte_offset, &mut buf)?;
    let header: &crate::directory::DirectoryRecordHeader =
        bytemuck::from_bytes(&buf[..core::mem::size_of::<crate::directory::DirectoryRecordHeader>()]);
    Ok(DirectoryRef {
        extent: sector,
        size: header.data_len.read() as usize,
    })
}

