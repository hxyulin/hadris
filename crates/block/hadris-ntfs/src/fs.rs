use hadris_fs::{
    Attributes, Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats,
    Metadata, MountError, MountOptions, Name, NodeId, OpenMode, Permissions,
};
use hadris_storage::BlockIndex;

use super::FileSystem;
use super::storage::BlockDevice;
use crate::error::{Detail, Error};
use crate::raw;
use crate::record::{
    self, Attr, Body, FileName, MAX_LIST_ENTRY, MAX_RECORD, RecordHeader, Runs, reference_record,
    reference_sequence,
};
use crate::volume::{
    DirIndex, Extents, Geometry, Info, MAX_BLOCK, MftExtents, PAGE, Upcase, UpcaseExtents,
};

/// The raw cursor after the last entry.
const CURSOR_END: u64 = 1 << 56;
/// Bits of a cursor that hold the byte offset within an index node.
const CURSOR_SHIFT: u32 = 16;
/// Bytes of a UTF-16 attribute name.
const NAME_BYTES: usize = 510;

/// A node id the caller gave that names nothing valid is an invalid handle,
/// unless the device failed or the volume is cut short.
fn handle<E>(err: Error<E>) -> Error<E> {
    match (err.kind(), Detail::of(&err)) {
        (ErrorKind::Io, _) | (_, Some(Detail::OutsideVolume)) => err,
        (ErrorKind::Unsupported, _) => err,
        _ => ErrorKind::InvalidHandle.into(),
    }
}

/// A parsing failure as a filesystem error.
fn fs<T, E>(result: Result<T, Detail>) -> FsResult<T, E> {
    result.map_err(Error::from)
}

/// A `$MFT` in more extents than the reader keeps.
fn unsupported_list<E>() -> Error<E> {
    Detail::AttributeList.error(ErrorKind::Unsupported)
}

/// POSIX permissions derived from the type and the DOS read-only bit.
fn permissions(file_type: FileType, attributes: Attributes) -> Permissions {
    let bits = if file_type.is_dir() { 0o755 } else { 0o644 };
    if attributes.contains(Attributes::READ_ONLY) {
        Permissions::new(bits & !0o222)
    } else {
        Permissions::new(bits)
    }
}

/// How a stored name matches a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    No,
    Exact,
    Folded,
}

/// Whether a directory listing leaves an entry out: DOS aliases of long
/// names, and the metadata files, as other NTFS drivers do.
fn hidden(reference: u64, name: &FileName<'_>) -> bool {
    name.namespace == raw::FILE_NAME_DOS || reference_record(reference) < raw::RECORD_FIRST_USER
}

/// Where an attribute's value is.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// Resident, at `Slot` in the base record or in the given extension
    /// record.
    Resident(Option<u64>, Slot),
    /// Non-resident, with every mapping pair in the base record.
    Base,
    /// Non-resident, in the extents an `$ATTRIBUTE_LIST` names.
    Listed,
}

/// An attribute's value: where it is, its flags and its sizes.
#[derive(Debug, Clone, Copy)]
struct Head {
    flags: u16,
    size: u64,
    initialized: u64,
    place: Place,
}

impl Head {
    fn of(attr: &Attr<'_>, place: Place) -> Self {
        let (size, initialized) = match attr.body {
            Body::Resident(value) => (value.len() as u64, value.len() as u64),
            Body::NonResident(nr) => (nr.data_size, nr.initialized_size.min(nr.data_size)),
        };
        Self {
            flags: attr.flags,
            size,
            initialized,
            place,
        }
    }
}

/// A directory's `$I30` index: its root, where its allocation and bitmap
/// are, and how many index blocks it has.
struct Index<'a> {
    root: DirIndex<'a>,
    allocation: Option<Head>,
    bitmap: Option<Head>,
    blocks: u64,
}

/// Which attribute of a record an instance is.
#[derive(Debug, Clone, Copy)]
enum Slot {
    /// The position among the record's attributes, for a record without an
    /// `$ATTRIBUTE_LIST`.
    Ordinal(usize),
    /// The instance number an `$ATTRIBUTE_LIST` entry gives.
    Id(u16),
}

/// An attribute instance found in a record or its `$ATTRIBUTE_LIST`.
#[derive(Debug, Clone, Copy)]
struct Found {
    start_vcn: u64,
    /// The extension record that holds it, or `None` for the base record.
    record: Option<u64>,
    slot: Slot,
    name_len: usize,
}

io_transform! {

/// Reads `buf.len()` bytes from byte `offset` of `dev`, which holds `len`
/// bytes.
async fn read_bytes<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    let end = offset
        .checked_add(buf.len() as u64)
        .ok_or(Detail::OutsideVolume)?;
    if end > len {
        return Err(Detail::OutsideVolume.into());
    }
    let bs = u64::from(dev.block_size().get());
    let mut done = 0;
    let mut pos = offset;
    while done < buf.len() {
        let block = BlockIndex::new(pos / bs);
        let within = (pos % bs) as usize;
        let left = buf.len() - done;
        if within == 0 && left as u64 >= bs {
            let whole = left - left % bs as usize;
            dev.read_blocks(block, &mut buf[done..done + whole])
                .await?;
            done += whole;
            pos += whole as u64;
        } else {
            let mut scratch = [0u8; MAX_BLOCK];
            let scratch = &mut scratch[..bs as usize];
            dev.read_blocks(block, scratch).await?;
            let n = (bs as usize - within).min(left);
            buf[done..done + n].copy_from_slice(&scratch[within..within + n]);
            done += n;
            pos += n as u64;
        }
    }
    Ok(())
}

/// Reads `buf.len()` bytes from byte `offset` of the stream that `runs`
/// maps. Sparse runs read as zeros.
async fn read_runs<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    runs: &[u8],
    offset: u64,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    let end = offset.checked_add(buf.len() as u64).ok_or(Detail::DataRun)?;
    let mut start = 0u64;
    let mut filled = 0usize;
    for run in Runs::new(runs) {
        if filled == buf.len() {
            break;
        }
        let run = run?;
        let bytes = run.len.checked_mul(geo.cluster_size).ok_or(Detail::DataRun)?;
        let run_end = start.checked_add(bytes).ok_or(Detail::DataRun)?;
        if run_end > offset && start < end {
            let from = offset.max(start);
            let to = end.min(run_end);
            let n = (to - from) as usize;
            let out = &mut buf[filled..filled + n];
            match run.lcn {
                None => out.fill(0),
                Some(lcn) => {
                    let at = lcn
                        .checked_mul(geo.cluster_size)
                        .and_then(|at| at.checked_add(from - start))
                        .ok_or(Detail::DataRun)?;
                    read_bytes(dev, geo.device_len, at, out).await?;
                }
            }
            filled += n;
        }
        start = run_end;
    }
    if filled < buf.len() {
        return Err(Detail::DataRun.into());
    }
    Ok(())
}

/// Reads from byte `offset` of a non-resident stream of `size` bytes whose
/// first `initialized` bytes are stored, and returns how many bytes it read.
async fn read_stream<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    runs: &[u8],
    size: u64,
    initialized: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, Error<D::Error>> {
    if offset >= size {
        return Ok(0);
    }
    let n = (size - offset).min(buf.len() as u64) as usize;
    let stored = initialized.saturating_sub(offset).min(n as u64) as usize;
    if stored > 0 {
        read_runs(dev, geo, runs, offset, &mut buf[..stored]).await?;
    }
    buf[stored..n].fill(0);
    Ok(n)
}

/// The upper-case form of a UTF-16 code unit. A zero entry, as in a
/// sparse table, leaves the unit as it is.
async fn upper<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    upcase: &mut Upcase,
    unit: u16,
) -> Result<u16, Error<D::Error>> {
    let page = unit / PAGE as u16;
    let within = usize::from(unit) % PAGE;
    let mapped = if page == 0 {
        upcase.low[within]
    } else {
        if upcase.page_index != Some(page) {
            load_page(dev, geo, &upcase.stream, page, &mut upcase.page).await?;
            upcase.page_index = Some(page);
        }
        upcase.page[within]
    };
    Ok(if mapped == 0 { unit } else { mapped })
}

/// How a stored UTF-16LE name matches `query`: exactly, only with case
/// folded through `$UpCase` (when `fold` allows it), or not at all.
async fn name_match<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    upcase: &mut Upcase,
    stored: &[u8],
    query: &str,
    fold: bool,
) -> Result<Match, Error<D::Error>> {
    let mut left = record::utf16_chars(stored);
    let mut right = query.chars();
    let mut matched = Match::Exact;
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ok(matched),
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                let (Ok(a), Ok(b)) = (u16::try_from(u32::from(a)), u16::try_from(u32::from(b))) else {
                    return Ok(Match::No);
                };
                if !fold || upper(dev, geo, upcase, a).await? != upper(dev, geo, upcase, b).await? {
                    return Ok(Match::No);
                }
                matched = Match::Folded;
            }
            _ => return Ok(Match::No),
        }
    }
}

/// Reads from byte `offset` of a stream kept in `extents`, and returns how
/// many bytes it read.
async fn read_extents<D: BlockDevice, const N: usize, const S: usize>(
    dev: &mut D,
    geo: &Geometry,
    extents: &Extents<N, S>,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, Error<D::Error>> {
    if offset >= extents.size {
        return Ok(0);
    }
    let n = (extents.size - offset).min(buf.len() as u64) as usize;
    let stored = extents.initialized.saturating_sub(offset).min(n as u64) as usize;
    let end = offset + stored as u64;
    let mut done = offset;
    let mut index = 0;
    while done < end {
        let Some((vcn, runs)) = extents.span(index) else {
            return Err(Detail::DataRun.into());
        };
        let start = vcn.checked_mul(geo.cluster_size).ok_or(Detail::DataRun)?;
        let stop = match extents.span(index + 1) {
            Some((next, _)) => next.checked_mul(geo.cluster_size).ok_or(Detail::DataRun)?,
            None => u64::MAX,
        };
        index += 1;
        if start > done {
            return Err(Detail::DataRun.into());
        }
        let to = end.min(stop);
        if to > done {
            let part = &mut buf[(done - offset) as usize..(to - offset) as usize];
            read_runs(dev, geo, runs, done - start, part).await?;
            done = to;
        }
    }
    buf[stored..n].fill(0);
    Ok(n)
}

/// Reads MFT record `index` into `buf` and checks it.
async fn read_record<D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    index: u64,
    buf: &mut [u8],
) -> Result<RecordHeader, Error<D::Error>> {
    let size = info.geo.mft_record_size;
    let buf = &mut buf[..size];
    let offset = index.checked_mul(size as u64).ok_or(Detail::Record)?;
    if offset.checked_add(size as u64).is_none_or(|end| end > info.mft.size) {
        return Err(Detail::Record.into());
    }
    read_extents(dev, &info.geo, &info.mft, offset, buf).await?;
    Ok(record::file_record(buf)?)
}

/// Reads the record a file reference names and checks its sequence number.
async fn read_reference<D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    reference: u64,
    buf: &mut [u8],
) -> Result<RecordHeader, Error<D::Error>> {
    let header = read_record(dev, info, reference_record(reference), buf).await?;
    let sequence = reference_sequence(reference);
    if sequence != 0 && sequence != header.sequence {
        return Err(Detail::StaleReference.into());
    }
    Ok(header)
}

/// Finds the next attribute of `kind` after `pos`, in the record's
/// `$ATTRIBUTE_LIST` or, without one, in the record itself, and writes its
/// name into `name`. `pos` starts at 0.
async fn next_attr<D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    rec: &[u8],
    base: u64,
    kind: u32,
    pos: &mut u64,
    name: &mut [u8; NAME_BYTES],
) -> Result<Option<Found>, Error<D::Error>> {
    let Some(list) = record::find_attr(rec, raw::ATTR_ATTRIBUTE_LIST, &[])? else {
        for (i, attr) in record::Attrs::new(rec)?.enumerate().skip(*pos as usize) {
            let attr = attr?;
            *pos = i as u64 + 1;
            if attr.kind != kind {
                continue;
            }
            let start_vcn = match attr.body {
                Body::Resident(_) => 0,
                Body::NonResident(nr) => nr.start_vcn,
            };
            name[..attr.name.len()].copy_from_slice(attr.name);
            return Ok(Some(Found {
                start_vcn,
                record: None,
                slot: Slot::Ordinal(i),
                name_len: attr.name.len(),
            }));
        }
        *pos = u64::MAX;
        return Ok(None);
    };
    let mut chunk = [0u8; MAX_LIST_ENTRY];
    loop {
        if *pos >= list.len() {
            return Ok(None);
        }
        let bytes = match list.body {
            Body::Resident(value) => &value[*pos as usize..],
            Body::NonResident(nr) => {
                let initialized = nr.initialized_size.min(nr.data_size);
                let n = read_stream(dev, &info.geo, nr.runs, nr.data_size, initialized, *pos, &mut chunk).await?;
                &chunk[..n]
            }
        };
        let entry = record::list_entry(bytes)?;
        *pos += entry.len as u64;
        if entry.kind != kind {
            continue;
        }
        name[..entry.name.len()].copy_from_slice(entry.name);
        let record = (reference_record(entry.reference) != base).then_some(entry.reference);
        return Ok(Some(Found {
            start_vcn: entry.start_vcn,
            record,
            slot: Slot::Id(entry.id),
            name_len: entry.name.len(),
        }));
    }
}

/// The attribute instance `found` names, from `rec` or read into `ext`.
async fn load<'a, D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    rec: &'a [u8],
    ext: &'a mut [u8; MAX_RECORD],
    kind: u32,
    found: &Found,
) -> Result<Attr<'a>, Error<D::Error>> {
    let source: &'a [u8] = match found.record {
        None => rec,
        Some(reference) => {
            read_reference(dev, info, reference, &mut ext[..]).await?;
            let ext: &'a [u8; MAX_RECORD] = ext;
            &ext[..info.geo.mft_record_size]
        }
    };
    let attr = match found.slot {
        Slot::Ordinal(i) => record::Attrs::new(source)?.nth(i).transpose()?.filter(|attr| attr.kind == kind),
        Slot::Id(id) => record::find_id(source, kind, id)?,
    };
    attr.ok_or(Detail::AttributeList.into())
}

/// Finds the value of the attribute of `kind` named `name` (UTF-16LE, empty
/// for unnamed) of the node whose base record `rec` is record `base`.
async fn stream_head<D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    rec: &[u8],
    base: u64,
    kind: u32,
    name: &[u8],
) -> Result<Option<Head>, Error<D::Error>> {
    if record::find_attr(rec, raw::ATTR_ATTRIBUTE_LIST, &[])?.is_none() {
        for (i, attr) in record::Attrs::new(rec)?.enumerate() {
            let attr = attr?;
            if attr.kind != kind || attr.name != name {
                continue;
            }
            let place = match attr.body {
                Body::Resident(_) => Place::Resident(None, Slot::Ordinal(i)),
                Body::NonResident(nr) if nr.start_vcn == 0 => Place::Base,
                Body::NonResident(_) => continue,
            };
            return Ok(Some(Head::of(&attr, place)));
        }
        return Ok(None);
    }
    let mut pos = 0;
    let mut found_name = [0u8; NAME_BYTES];
    let mut ext = [0u8; MAX_RECORD];
    while let Some(found) = next_attr(dev, info, rec, base, kind, &mut pos, &mut found_name).await? {
        if found.start_vcn != 0 || &found_name[..found.name_len] != name {
            continue;
        }
        let attr = load(dev, info, rec, &mut ext, kind, &found).await?;
        let place = match attr.body {
            Body::Resident(_) => Place::Resident(found.record, found.slot),
            Body::NonResident(_) => Place::Listed,
        };
        return Ok(Some(Head::of(&attr, place)));
    }
    Ok(None)
}

/// Reads from byte `offset` of a value `stream_head` found, and returns how
/// many bytes it read. Compressed and encrypted values are unsupported.
#[allow(clippy::too_many_arguments)]
async fn stream_read<D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    rec: &[u8],
    base: u64,
    kind: u32,
    name: &[u8],
    head: &Head,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, Error<D::Error>> {
    if head.flags & raw::ATTR_FLAG_COMPRESSED != 0 {
        return Err(Detail::Compressed.into());
    }
    if head.flags & raw::ATTR_FLAG_ENCRYPTED != 0 {
        return Err(Detail::Encrypted.into());
    }
    match head.place {
        Place::Resident(record, slot) => {
            let mut ext = [0u8; MAX_RECORD];
            let found = Found {
                start_vcn: 0,
                record,
                slot,
                name_len: name.len(),
            };
            let attr = load(dev, info, rec, &mut ext, kind, &found).await?;
            let Body::Resident(value) = attr.body else {
                return Err(Detail::Attribute.into());
            };
            let Some(rest) = usize::try_from(offset).ok().and_then(|at| value.get(at..)) else {
                return Ok(0);
            };
            let n = rest.len().min(buf.len());
            buf[..n].copy_from_slice(&rest[..n]);
            Ok(n)
        }
        Place::Base => {
            let attr = record::find_instance(rec, kind, name, 0)?.ok_or(Detail::Attribute)?;
            let Body::NonResident(nr) = attr.body else {
                return Err(Detail::Attribute.into());
            };
            read_stream(dev, &info.geo, nr.runs, head.size, head.initialized, offset, buf).await
        }
        Place::Listed => {
            if offset >= head.size {
                return Ok(0);
            }
            let n = (head.size - offset).min(buf.len() as u64) as usize;
            let stored = head.initialized.saturating_sub(offset).min(n as u64) as usize;
            if stored > 0 {
                read_listed(dev, info, rec, base, kind, name, offset, &mut buf[..stored]).await?;
            }
            buf[stored..n].fill(0);
            Ok(n)
        }
    }
}

/// Reads `buf.len()` bytes from byte `offset` of a non-resident value whose
/// extents an `$ATTRIBUTE_LIST` names, reading only the extension records
/// of the extents the range touches.
#[allow(clippy::too_many_arguments)]
async fn read_listed<D: BlockDevice>(
    dev: &mut D,
    info: &Info,
    rec: &[u8],
    base: u64,
    kind: u32,
    name: &[u8],
    offset: u64,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    let cluster = info.geo.cluster_size;
    let end = offset.checked_add(buf.len() as u64).ok_or(Detail::DataRun)?;
    let mut pos = 0;
    let mut found_name = [0u8; NAME_BYTES];
    let mut ext = [0u8; MAX_RECORD];
    let mut pending: Option<Found> = None;
    let mut done = offset;
    loop {
        let next = loop {
            match next_attr(dev, info, rec, base, kind, &mut pos, &mut found_name).await? {
                Some(found) if &found_name[..found.name_len] == name => break Some(found),
                Some(_) => {}
                None => break None,
            }
        };
        if let Some(current) = pending {
            let start = current.start_vcn.checked_mul(cluster).ok_or(Detail::DataRun)?;
            let stop = match next {
                Some(next) => next.start_vcn.checked_mul(cluster).ok_or(Detail::DataRun)?,
                None => u64::MAX,
            };
            if stop <= start {
                return Err(Detail::AttributeList.into());
            }
            let to = end.min(stop);
            if to > done {
                if start > done {
                    return Err(Detail::DataRun.into());
                }
                let attr = load(dev, info, rec, &mut ext, kind, &current).await?;
                let Body::NonResident(nr) = attr.body else {
                    return Err(Detail::AttributeList.into());
                };
                let part = &mut buf[(done - offset) as usize..(to - offset) as usize];
                read_runs(dev, &info.geo, nr.runs, done - start, part).await?;
                done = to;
            }
        }
        match next {
            Some(next) if done < end => pending = Some(next),
            _ => break,
        }
    }
    if done < end {
        return Err(Detail::DataRun.into());
    }
    Ok(())
}

async fn load_page<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    stream: &UpcaseExtents,
    page: u16,
    out: &mut [u16; PAGE],
) -> Result<(), Error<D::Error>> {
    let mut bytes = [0u8; PAGE * 2];
    let offset = u64::from(page) * (PAGE * 2) as u64;
    read_extents(dev, geo, stream, offset, &mut bytes).await?;
    for (unit, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        *unit = u16::from_le_bytes([pair[0], pair[1]]);
    }
    Ok(())
}

/// Reads the volume structures.
async fn mount<D: BlockDevice>(dev: &mut D) -> Result<Info, Error<D::Error>> {
    let block = dev.block_size().get() as usize;
    if block > MAX_BLOCK {
        return Err(Detail::BlockSize.into());
    }
    let device_len = dev
        .block_count()
        .checked_mul(block as u64)
        .ok_or(Detail::OutsideVolume)?;
    let mut sector = [0u8; 512];
    if device_len < 512 {
        return Err(Detail::BootSector.into());
    }
    read_bytes(dev, device_len, 0, &mut sector).await?;
    let boot: raw::BootSector = bytemuck::pod_read_unaligned(&sector);
    let geo = Geometry::new(&boot, device_len)?;

    let size = geo.mft_record_size;
    let mut rec = [0u8; MAX_RECORD];
    read_bytes(dev, device_len, geo.mft_offset, &mut rec[..size]).await?;
    record::file_record(&mut rec[..size])?;
    let rec = &rec[..size];
    let mft = match record::find_instance(rec, raw::ATTR_DATA, &[], 0)? {
        Some(Attr { body: Body::NonResident(nr), .. }) => MftExtents::new(&nr).ok_or(Detail::Attribute)?,
        _ => return Err(Detail::Attribute.into()),
    };
    let mut info = Info {
        geo,
        mft,
        upcase: Upcase {
            stream: UpcaseExtents::empty(),
            low: [0; PAGE],
            page: [0; PAGE],
            page_index: None,
        },
        root_sequence: 0,
        free_clusters: None,
    };
    if record::find_attr(rec, raw::ATTR_ATTRIBUTE_LIST, &[])?.is_some() {
        let mut pos = 0;
        let mut name = [0u8; NAME_BYTES];
        let mut ext = [0u8; MAX_RECORD];
        while let Some(found) = next_attr(dev, &info, rec, raw::RECORD_MFT, raw::ATTR_DATA, &mut pos, &mut name).await? {
            if found.name_len != 0 || found.start_vcn == 0 {
                continue;
            }
            let attr = load(dev, &info, rec, &mut ext, raw::ATTR_DATA, &found).await?;
            let Body::NonResident(nr) = attr.body else {
                return Err(Detail::AttributeList.into());
            };
            info.mft.push(&nr).ok_or(unsupported_list())?;
        }
    }

    let mut buf = [0u8; MAX_RECORD];
    let root = read_record(dev, &info, raw::RECORD_ROOT, &mut buf).await?;
    if !root.is_dir() {
        return Err(Detail::Record.into());
    }
    info.root_sequence = root.sequence;

    read_record(dev, &info, raw::RECORD_UPCASE, &mut buf).await?;
    const UPCASE_BYTES: u64 = 65536 * 2;
    info.upcase.stream = match record::find_instance(&buf[..size], raw::ATTR_DATA, &[], 0)? {
        Some(Attr { body: Body::NonResident(nr), .. }) if nr.data_size == UPCASE_BYTES => {
            UpcaseExtents::new(&nr).ok_or(Detail::Upcase)?
        }
        _ => return Err(Detail::Upcase.into()),
    };
    let Info { upcase, .. } = &mut info;
    load_page(dev, &geo, &upcase.stream, 0, &mut upcase.low).await?;
    Ok(info)
}

/// An NTFS volume on a block device, read-only.
///
/// It reads the boot sector, the location of `$MFT` and the first page of
/// `$UpCase` when opened, and needs no allocator. It implements
/// `hadris_fs` `FileSystem` read-only: node ids are file references (the
/// MFT record number, with the sequence number in the top 16 bits), so they
/// are stable and `forget` does nothing. Hard links share one id. Write
/// methods fail with [`ErrorKind::ReadOnly`].
///
/// Listings leave out DOS 8.3 aliases and the metadata files (MFT records
/// below 16, such as `$MFT`), which `lookup` still finds.
/// Names in the Win32 namespace compare case-insensitively through the
/// volume's `$UpCase` table, names in the POSIX namespace exactly.
///
/// ```rust,ignore
/// let mut ntfs = NtfsFs::mount(dev, MountOptions::new())?;
/// let node = ntfs.resolve(b"/docs/readme.txt", Resolve::Lexical)?;
/// let n = ntfs.read(node, 0, &mut buf)?;
/// ```
#[derive(Debug)]
pub struct NtfsFs<D> {
    dev: D,
    info: Info,
}

impl<D: BlockDevice> NtfsFs<D> {
    /// Mounts the volume on `dev`, read-only. NTFS reads only the primary
    /// boot sector, so [`MountOptions::backup_boot`] and the other options
    /// do not apply.
    ///
    /// Fails with [`ErrorKind::Corrupt`] when the boot sector, `$MFT`, the
    /// root directory or `$UpCase` are invalid, and with
    /// [`ErrorKind::Unsupported`] for device blocks or records above 4096
    /// bytes, or a `$MFT` in more than 32 extents. The device comes back in
    /// the [`MountError`].
    ///
    /// @hadris-spec NTFS:Master-File-Table
    /// @hadris-compliance partial
    /// @hadris-tests read::open_blank_volume, crafted::open_rejects_bad_boot_sectors, crafted::fragmented_mft_is_followed
    /// @hadris-fuzz ntfs_read
    /// @hadris-note Reads `$MFT` from its base record and extension records and checks file references; `$MFTMirr` recovery is not supported.
    pub async fn mount(mut dev: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        let _ = options;
        match mount(&mut dev).await {
            Ok(info) => Ok(Self { dev, info }),
            Err(err) => Err(MountError::new(err, dev)),
        }
    }

    /// Gives the device back. The volume is read-only, so there is nothing
    /// to sync and this never fails.
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        Ok(self.dev)
    }

    /// The volume serial number.
    pub fn volume_serial(&self) -> u64 {
        self.info.geo.serial
    }

    /// Bytes per sector.
    pub fn sector_size(&self) -> u32 {
        self.info.geo.sector_size
    }

    /// Bytes per cluster.
    pub fn cluster_size(&self) -> u64 {
        self.info.geo.cluster_size
    }

    /// Sectors in the volume.
    pub fn total_sectors(&self) -> u64 {
        self.info.geo.total_sectors
    }

    /// Bytes per MFT record.
    pub fn mft_record_size(&self) -> u32 {
        self.info.geo.mft_record_size as u32
    }

    /// Bytes per index record, as the boot sector gives it.
    pub fn index_record_size(&self) -> u32 {
        self.info.geo.index_record_size as u32
    }

    async fn count_free(&mut self) -> Result<u64, Error<D::Error>> {
        let mut rec = [0u8; MAX_RECORD];
        read_record(&mut self.dev, &self.info, raw::RECORD_BITMAP, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = raw::RECORD_BITMAP;
        let head = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &[])
            .await?
            .ok_or(Detail::Attribute)?;
        let total = self.info.geo.total_clusters;
        let mut used = 0u64;
        let mut chunk = [0u8; 512];
        let mut offset = 0u64;
        while offset * 8 < total {
            let n = stream_read(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &[], &head, offset, &mut chunk).await?;
            if n == 0 {
                return Err(Detail::Attribute.into());
            }
            for (i, byte) in chunk[..n].iter().enumerate() {
                let first = (offset + i as u64) * 8;
                if first >= total {
                    break;
                }
                let bits = (total - first).min(8) as u32;
                let mask = if bits == 8 { 0xFF } else { (1u8 << bits) - 1 };
                used += u64::from((byte & mask).count_ones());
            }
            offset += n as u64;
        }
        Ok(total.saturating_sub(used))
    }

    /// Borrows the device.
    pub fn device(&self) -> &D {
        &self.dev
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// Reads the record of `node` into `buf`.
    async fn node_record(&mut self, node: NodeId, buf: &mut [u8]) -> FsResult<RecordHeader, D::Error> {
        let raw_id = node.get();
        let header = read_record(&mut self.dev, &self.info, reference_record(raw_id), buf)
            .await
            .map_err(handle)?;
        let sequence = reference_sequence(raw_id);
        if sequence != 0 && sequence != header.sequence {
            return Err(ErrorKind::InvalidHandle.into());
        }
        Ok(header)
    }

    async fn dir_record(&mut self, node: NodeId, buf: &mut [u8]) -> FsResult<(), D::Error> {
        if !self.node_record(node, buf).await?.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(())
    }

    /// The index root, allocation and bitmap of the directory whose record
    /// `rec` is record `base`, and its number of index blocks. An index
    /// root in an extension record is read into `ext`.
    async fn dir_index<'a>(
        &mut self,
        rec: &'a [u8],
        base: u64,
        ext: &'a mut [u8; MAX_RECORD],
    ) -> Result<Index<'a>, Error<D::Error>> {
        let root = match record::find_attr(rec, raw::ATTR_INDEX_ROOT, &raw::I30)? {
            Some(attr) => attr,
            None => {
                let mut pos = 0;
                let mut name = [0u8; NAME_BYTES];
                let found = loop {
                    match next_attr(&mut self.dev, &self.info, rec, base, raw::ATTR_INDEX_ROOT, &mut pos, &mut name).await? {
                        Some(found) if name[..found.name_len] == raw::I30 => break found,
                        Some(_) => {}
                        None => return Err(Detail::Index.into()),
                    }
                };
                load(&mut self.dev, &self.info, rec, ext, raw::ATTR_INDEX_ROOT, &found).await?
            }
        };
        let root = DirIndex::new(&root)?;
        let allocation = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_INDEX_ALLOCATION, &raw::I30).await?;
        let bitmap = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_BITMAP, &raw::I30).await?;
        let blocks = match allocation {
            Some(head) => root.blocks(head.size)?,
            None => 0,
        };
        Ok(Index {
            root,
            allocation,
            bitmap,
            blocks,
        })
    }

    /// Whether index block `block` is in use, from the directory's bitmap.
    async fn block_in_use(&mut self, rec: &[u8], base: u64, bitmap: Option<Head>, block: u64) -> Result<bool, Error<D::Error>> {
        let head = bitmap.ok_or(Detail::Index)?;
        let mut byte = [0u8];
        let n = stream_read(&mut self.dev, &self.info, rec, base, raw::ATTR_BITMAP, &raw::I30, &head, block / 8, &mut byte).await?;
        if n == 0 {
            return Err(Detail::Index.into());
        }
        Ok(byte[0] & 1 << (block % 8) != 0)
    }

    /// Reads index block `block` into `buf` and checks it.
    #[allow(clippy::too_many_arguments)]
    async fn index_block(
        &mut self,
        rec: &[u8],
        base: u64,
        allocation: Option<Head>,
        size: usize,
        block: u64,
        buf: &mut [u8],
    ) -> Result<(), Error<D::Error>> {
        let head = allocation.ok_or(Detail::Index)?;
        let buf = &mut buf[..size];
        let offset = block * size as u64;
        let n = stream_read(&mut self.dev, &self.info, rec, base, raw::ATTR_INDEX_ALLOCATION, &raw::I30, &head, offset, buf).await?;
        if n != size || &buf[..4] != b"INDX" {
            return Err(Detail::Record.into());
        }
        record::apply_fixups(buf)?;
        Ok(())
    }

    /// Calls `visit` with the name and length of each named data stream of
    /// a node, the alternate data streams.
    ///
    /// @hadris-spec NTFS:Named-Streams
    /// @hadris-compliance partial
    /// @hadris-tests crafted::named_streams_are_listed_and_read, read::named_streams_read_back
    /// @hadris-fuzz ntfs_read
    /// @hadris-note Lists and reads named `$DATA` attributes; other attribute types are not exposed as streams.
    pub async fn streams(&mut self, node: NodeId, mut visit: impl FnMut(&str, u64)) -> FsResult<(), D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        self.node_record(node, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = reference_record(node.get());
        let mut pos = 0;
        let mut found_name = [0u8; NAME_BYTES];
        let mut ext = [0u8; MAX_RECORD];
        let mut text = [0u8; 1024];
        while let Some(found) = next_attr(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &mut pos, &mut found_name).await? {
            if found.name_len == 0 || found.start_vcn != 0 {
                continue;
            }
            let stream = &found_name[..found.name_len];
            let len = load(&mut self.dev, &self.info, rec, &mut ext, raw::ATTR_DATA, &found).await?.len();
            let n = record::utf16_to_utf8(stream, &mut text).ok_or(ErrorKind::LimitExceeded)?;
            let name = core::str::from_utf8(&text[..n]).map_err(|_| Error::<D::Error>::from(Detail::Attribute))?;
            visit(name, len);
        }
        Ok(())
    }

    /// Reads from the named data stream `stream` of a node at `offset`.
    /// Stream names compare case-insensitively. [`ErrorKind::NotFound`]
    /// when the node has no such stream.
    pub async fn read_stream_at(
        &mut self,
        node: NodeId,
        stream: &str,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        self.node_record(node, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = reference_record(node.get());
        let mut pos = 0;
        let mut found_name = [0u8; NAME_BYTES];
        while let Some(found) = next_attr(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &mut pos, &mut found_name).await? {
            if found.name_len == 0 || found.start_vcn != 0 {
                continue;
            }
            let stored = &found_name[..found.name_len];
            let Info { geo, upcase, .. } = &mut self.info;
            if name_match(&mut self.dev, geo, upcase, stored, stream, true).await? == Match::No {
                continue;
            }
            let head = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, stored)
                .await?
                .ok_or(Error::<D::Error>::from(Detail::Attribute))?;
            return stream_read(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, stored, &head, offset, buf).await;
        }
        Err(ErrorKind::NotFound.into())
    }
}


impl<D: BlockDevice> FileSystem for NtfsFs<D> {
    type DeviceError = D::Error;

    /// Read-only, with hard links; names are UTF-16 and compared as Win32
    /// does.
    fn capabilities(&self) -> Capabilities {
        self.info.capabilities()
    }

    /// The root directory, MFT record 5.
    fn root(&self) -> NodeId {
        self.info.root()
    }

    /// Finds `name` in `dir`, including the metadata files and DOS aliases
    /// that listings leave out.
    ///
    /// A listed entry whose name is exactly `name` wins; otherwise the
    /// first entry that matches with case folded, as Windows does for
    /// Win32 and DOS names. POSIX names match only exactly. A `$UpCase`
    /// page that cannot be read fails the lookup only when no entry
    /// matches exactly.
    ///
    /// @hadris-spec NTFS:Directory-Index
    /// @hadris-compliance partial
    /// @hadris-tests crafted::names_fold_case_through_upcase, read::large_directory_lists_every_entry
    /// @hadris-fuzz ntfs_read
    /// @hadris-note Walks every index node instead of descending the B-tree by key.
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        let Ok(query) = name.to_str() else {
            return Err(ErrorKind::NotFound.into());
        };
        let mut rec = [0u8; MAX_RECORD];
        self.dir_record(dir, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = reference_record(dir.get());
        let mut ext = [0u8; MAX_RECORD];
        let Index {
            root: index,
            allocation,
            bitmap,
            blocks,
        } = self.dir_index(rec, base, &mut ext).await?;
        let mut block = [0u8; MAX_RECORD];
        let mut best: Option<(u8, u64)> = None;
        let mut deferred = None;
        let mut node = 0u64;
        'nodes: while node <= blocks {
            let (buf, header) = if node == 0 {
                (index.root, 0x10)
            } else {
                if !self.block_in_use(rec, base, bitmap, node - 1).await? {
                    node += 1;
                    continue;
                }
                self.index_block(rec, base, allocation, index.block_size, node - 1, &mut block).await?;
                (&block[..index.block_size], 0x18)
            };
            let area = fs(record::index_node(buf, header))?;
            let mut pos = area.first;
            loop {
                let entry = fs(record::index_entry(buf, area, pos))?;
                pos = entry.next;
                let Some((reference, file_name)) = entry.entry else {
                    break;
                };
                let fold = file_name.namespace != raw::FILE_NAME_POSIX;
                let Info { geo, upcase, .. } = &mut self.info;
                let matched = match name_match(&mut self.dev, geo, upcase, file_name.name, query, fold).await {
                    Ok(matched) => matched,
                    Err(err) => {
                        deferred.get_or_insert(err);
                        continue;
                    }
                };
                let rank = match matched {
                    Match::No => continue,
                    Match::Exact => 0,
                    Match::Folded => 2,
                } + u8::from(hidden(reference, &file_name));
                if best.is_none_or(|(held, _)| rank < held) {
                    best = Some((rank, reference));
                }
                if rank == 0 {
                    break 'nodes;
                }
            }
            node += 1;
        }
        if let Some(err) = deferred {
            if best.is_none_or(|(rank, _)| rank > 0) {
                return Err(err);
            }
        }
        match best {
            Some((_, reference)) if reference_record(reference) == raw::RECORD_ROOT => Ok(self.root()),
            Some((_, reference)) => Ok(NodeId::new(reference).ok_or(Detail::Index)?),
            None => Err(ErrorKind::NotFound.into()),
        }
    }

    /// The entry at or after `from`, with the metadata `stat` gives. A
    /// cursor holds the index node and the byte offset of the next entry in
    /// it.
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        self.dir_record(dir, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = reference_record(dir.get());
        let mut ext = [0u8; MAX_RECORD];
        let Index {
            root: index,
            allocation,
            bitmap,
            blocks,
        } = self.dir_index(rec, base, &mut ext).await?;
        let mut block = [0u8; MAX_RECORD];
        let mut raw_cursor = from.into_raw();
        let mut name = [0u8; DirEntry::MAX_NAME];
        let (reference, len, next) = 'found: loop {
            if raw_cursor >= CURSOR_END {
                return Ok(None);
            }
            let node = raw_cursor >> CURSOR_SHIFT;
            if node > blocks {
                raw_cursor = CURSOR_END;
                continue;
            }
            let (buf, header) = if node == 0 {
                (index.root, 0x10)
            } else {
                if !self.block_in_use(rec, base, bitmap, node - 1).await? {
                    raw_cursor = (node + 1) << CURSOR_SHIFT;
                    continue;
                }
                self.index_block(rec, base, allocation, index.block_size, node - 1, &mut block).await?;
                (&block[..index.block_size], 0x18)
            };
            let area = fs(record::index_node(buf, header))?;
            let mut pos = (raw_cursor & ((1 << CURSOR_SHIFT) - 1)) as usize;
            if pos == 0 {
                pos = area.first;
            }
            loop {
                let entry = fs(record::index_entry(buf, area, pos))?;
                pos = entry.next;
                let Some((reference, file_name)) = entry.entry else {
                    raw_cursor = (node + 1) << CURSOR_SHIFT;
                    break;
                };
                if hidden(reference, &file_name) {
                    continue;
                }
                let len = record::utf16_to_utf8(file_name.name, &mut name).ok_or(Detail::FileName)?;
                break 'found (reference, len, DirCursor::from_raw(node << CURSOR_SHIFT | pos as u64));
            }
        };
        let node = NodeId::new(reference).ok_or(Detail::Index)?;
        let meta = self.stat(node).await?;
        let entry = DirEntry::new(Name::new(&name[..len]), node, meta, next).map_err(|_| Detail::FileName)?;
        Ok(Some(entry))
    }

    /// Metadata of a node: type, size, the four times and the DOS
    /// attributes of `$STANDARD_INFORMATION`, and the number of names that
    /// are not DOS aliases. A directory's size is 0.
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        let header = self.node_record(node, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = reference_record(node.get());
        let file_type = if header.is_dir() { FileType::Dir } else { FileType::File };
        let mut meta = Metadata::new(file_type, permissions(file_type, Attributes::NONE));
        if let Some(Attr { body: Body::Resident(value), .. }) = fs(record::find_attr(rec, raw::ATTR_STANDARD_INFORMATION, &[]))? {
            if value.len() >= 0x24 {
                let bits = record::u32_at(value, 0x20);
                let mut attributes = Attributes::empty();
                for (flag, attribute) in [
                    (raw::FILE_ATTRIBUTE_READONLY, Attributes::READ_ONLY),
                    (raw::FILE_ATTRIBUTE_HIDDEN, Attributes::HIDDEN),
                    (raw::FILE_ATTRIBUTE_SYSTEM, Attributes::SYSTEM),
                    (raw::FILE_ATTRIBUTE_ARCHIVE, Attributes::ARCHIVE),
                ] {
                    if bits & flag != 0 {
                        attributes |= attribute;
                    }
                }
                meta = Metadata::new(file_type, permissions(file_type, attributes)).with_attributes(attributes);
                let time = |at| record::nt_time(record::u64_at(value, at));
                if let Some(time) = time(0) {
                    meta = meta.with_created(time);
                }
                if let Some(time) = time(8) {
                    meta = meta.with_modified(time);
                }
                if let Some(time) = time(0x10) {
                    meta = meta.with_changed(time);
                }
                if let Some(time) = time(0x18) {
                    meta = meta.with_accessed(time);
                }
            }
        }
        let mut names = 0u64;
        let mut pos = 0;
        let mut found_name = [0u8; NAME_BYTES];
        let mut ext = [0u8; MAX_RECORD];
        while let Some(found) = next_attr(&mut self.dev, &self.info, rec, base, raw::ATTR_FILE_NAME, &mut pos, &mut found_name).await? {
            let attr = load(&mut self.dev, &self.info, rec, &mut ext, raw::ATTR_FILE_NAME, &found).await?;
            let Body::Resident(value) = attr.body else {
                return Err(Error::<D::Error>::from(Detail::FileName));
            };
            if fs(record::file_name(value))?.namespace != raw::FILE_NAME_DOS {
                names += 1;
            }
        }
        if !header.is_dir() {
            let head = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &[]).await?;
            meta = meta.with_len(head.ok_or(Error::<D::Error>::from(Detail::Attribute))?.size);
        }
        Ok(meta.with_nlink(names.max(1)))
    }

    /// Reads from the unnamed data stream of a file at `offset`. Bytes past
    /// the initialized size and sparse runs read as zeros. Directories fail
    /// with [`ErrorKind::IsADirectory`], compressed and encrypted streams
    /// with [`ErrorKind::Unsupported`].
    ///
    /// @hadris-spec NTFS:Data-Stream
    /// @hadris-compliance partial
    /// @hadris-tests read::files_read_back, crafted::streams_past_the_volume_fail, crafted::attribute_lists_join_extension_records
    /// @hadris-fuzz ntfs_read
    /// @hadris-note Reads resident, non-resident, sparse and partly initialized streams, also across extension records; compressed and encrypted streams are unsupported.
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        if self.node_record(node, &mut rec).await?.is_dir() {
            return Err(ErrorKind::IsADirectory.into());
        }
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = reference_record(node.get());
        let head = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &[])
            .await?
            .ok_or(Error::<D::Error>::from(Detail::Attribute))?;
        stream_read(&mut self.dev, &self.info, rec, base, raw::ATTR_DATA, &[], &head, offset, buf).await
    }

    /// The volume's clusters and the free ones, counted from `$Bitmap`
    /// once and then kept.
    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        let total = self.info.geo.total_clusters;
        let free = match self.info.free_clusters {
            Some(free) => free,
            None => {
                let free = self.count_free().await?;
                self.info.free_clusters = Some(free);
                free
            }
        };
        let block = u32::try_from(self.info.geo.cluster_size).map_err(|_| ErrorKind::LimitExceeded)?;
        Ok(FsStats::new(total, free, block))
    }

    /// Does nothing: NTFS node ids are stable.
    fn forget(&mut self, node: NodeId, count: u64) {
        let _ = (node, count);
    }

    /// The directory containing `dir`, from its `$FILE_NAME`. The root is
    /// its own parent.
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        self.dir_record(dir, &mut rec).await?;
        let base = reference_record(dir.get());
        if base == raw::RECORD_ROOT {
            return Ok(self.root());
        }
        let rec = &rec[..self.info.geo.mft_record_size];
        let mut pos = 0;
        let mut found_name = [0u8; NAME_BYTES];
        let mut ext = [0u8; MAX_RECORD];
        let Some(found) = next_attr(&mut self.dev, &self.info, rec, base, raw::ATTR_FILE_NAME, &mut pos, &mut found_name).await? else {
            return Err(Error::<D::Error>::from(Detail::FileName));
        };
        let attr = load(&mut self.dev, &self.info, rec, &mut ext, raw::ATTR_FILE_NAME, &found).await?;
        let Body::Resident(value) = attr.body else {
            return Err(Error::<D::Error>::from(Detail::FileName));
        };
        let parent = fs(record::file_name(value))?.parent;
        if reference_record(parent) == raw::RECORD_ROOT {
            return Ok(self.root());
        }
        Ok(NodeId::new(parent).ok_or(Detail::FileName)?)
    }

    /// The `$VOLUME_NAME` of `$Volume`, or `None` when it is missing or
    /// empty.
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        read_record(&mut self.dev, &self.info, raw::RECORD_VOLUME, &mut rec).await?;
        let rec = &rec[..self.info.geo.mft_record_size];
        let base = raw::RECORD_VOLUME;
        let Some(head) = stream_head(&mut self.dev, &self.info, rec, base, raw::ATTR_VOLUME_NAME, &[]).await? else {
            return Ok(None);
        };
        let mut name = [0u8; NAME_BYTES];
        let n = stream_read(&mut self.dev, &self.info, rec, base, raw::ATTR_VOLUME_NAME, &[], &head, 0, &mut name).await?;
        let len = record::utf16_to_utf8(&name[..n & !1], buf).ok_or(ErrorKind::LimitExceeded)?;
        if len == 0 {
            return Ok(None);
        }
        Ok(Some(core::str::from_utf8(&buf[..len]).map_err(|_| Detail::Attribute)?))
    }

    /// Opens a file for reading. Directories fail with
    /// [`ErrorKind::IsADirectory`], writing with [`ErrorKind::ReadOnly`].
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        if self.node_record(node, &mut rec).await?.is_dir() {
            return Err(ErrorKind::IsADirectory.into());
        }
        if mode == OpenMode::Write {
            return Err(ErrorKind::ReadOnly.into());
        }
        Ok(())
    }

    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    /// NTFS here has no symlinks: every node fails with
    /// [`ErrorKind::InvalidInput`].
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let mut rec = [0u8; MAX_RECORD];
        self.node_record(node, &mut rec).await?;
        let _ = buf;
        Err(ErrorKind::InvalidInput.into())
    }
}
}
