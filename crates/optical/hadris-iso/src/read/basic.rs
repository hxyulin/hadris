//! Allocation-free ISO 9660 reader.

use core::char::decode_utf16;

use super::directory::{DirectoryRecord, DirectoryRecordHeader, FileFlags};
use super::io::{self, Read, Seek, SeekFrom};
use super::volume::{
    PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor, VolumeDescriptor,
    VolumeDescriptorHeader, VolumeDescriptorType,
};
use crate::joliet::JolietLevel;

const DESCRIPTOR_SIZE: u64 = 2048;
const FIRST_DESCRIPTOR: u64 = 16;
const MIN_RECORD_SIZE: usize = core::mem::size_of::<DirectoryRecordHeader>() + 1;

/// A filename namespace exposed by an ISO image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoNamespace {
    /// The primary ISO 9660 directory tree.
    Primary,
    /// A Joliet supplementary directory tree.
    Joliet(JolietLevel),
}

/// A root directory and the namespace it represents.
#[derive(Debug, Clone, Copy)]
pub struct IsoRoot {
    namespace: IsoNamespace,
    extent: u32,
    size: u32,
    block_size: u16,
}

impl IsoRoot {
    /// Returns the root's filename namespace.
    pub const fn namespace(self) -> IsoNamespace {
        self.namespace
    }

    /// Returns the root directory's byte length.
    pub const fn size(self) -> u32 {
        self.size
    }

    /// Returns the image logical-block size used by this tree.
    pub const fn block_size(self) -> u16 {
        self.block_size
    }
}

/// An error produced while decoding an entry name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// The on-disk name is not valid for its namespace.
    InvalidEncoding,
    /// The caller-provided UTF-8 buffer is too small.
    BufferTooSmall,
}

impl core::fmt::Display for NameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidEncoding => f.write_str("invalid ISO filename encoding"),
            Self::BufferTooSmall => f.write_str("filename output buffer is too small"),
        }
    }
}

impl core::error::Error for NameError {}

/// A borrowed, allocation-free view of an ISO directory-entry name.
#[derive(Debug, Clone, Copy)]
pub struct IsoName<'a> {
    raw: &'a [u8],
    namespace: IsoNamespace,
}

impl<'a> IsoName<'a> {
    /// Returns the name exactly as stored in the directory record.
    pub const fn raw(self) -> &'a [u8] {
        self.raw
    }

    /// Returns the name's namespace and encoding.
    pub const fn namespace(self) -> IsoNamespace {
        self.namespace
    }

    /// Compares this name with a UTF-8 path component without allocating.
    pub fn matches(self, candidate: &str) -> bool {
        if self.raw == [0] {
            return candidate == ".";
        }
        if self.raw == [1] {
            return candidate == "..";
        }
        match self.namespace {
            IsoNamespace::Primary => {
                strip_primary_version(self.raw).eq_ignore_ascii_case(candidate.as_bytes())
            }
            IsoNamespace::Joliet(_) => joliet_matches(strip_joliet_version(self.raw), candidate),
        }
    }

    /// Decodes the name as UTF-8 into `output` and returns the initialized text.
    pub fn decode_into(self, output: &mut [u8]) -> Result<&str, NameError> {
        if self.raw == [0] {
            return write_special(output, b".");
        }
        if self.raw == [1] {
            return write_special(output, b"..");
        }
        match self.namespace {
            IsoNamespace::Primary => {
                let raw = strip_primary_version(self.raw);
                if !raw.is_ascii() {
                    return Err(NameError::InvalidEncoding);
                }
                if raw.len() > output.len() {
                    return Err(NameError::BufferTooSmall);
                }
                output[..raw.len()].copy_from_slice(raw);
                core::str::from_utf8(&output[..raw.len()]).map_err(|_| NameError::InvalidEncoding)
            }
            IsoNamespace::Joliet(_) => decode_joliet_into(strip_joliet_version(self.raw), output),
        }
    }
}

fn write_special<'a>(output: &'a mut [u8], value: &[u8]) -> Result<&'a str, NameError> {
    if value.len() > output.len() {
        return Err(NameError::BufferTooSmall);
    }
    output[..value.len()].copy_from_slice(value);
    core::str::from_utf8(&output[..value.len()]).map_err(|_| NameError::InvalidEncoding)
}

fn strip_primary_version(raw: &[u8]) -> &[u8] {
    if let Some(pos) = raw.iter().rposition(|byte| *byte == b';')
        && pos + 1 < raw.len()
        && raw[pos + 1..].iter().all(u8::is_ascii_digit)
    {
        &raw[..pos]
    } else {
        raw
    }
}

fn strip_joliet_version(raw: &[u8]) -> &[u8] {
    if raw.len() % 2 != 0 {
        return raw;
    }
    let mut pos = raw.len();
    while pos >= 2 && raw[pos - 2] == 0 && raw[pos - 1].is_ascii_digit() {
        pos -= 2;
    }
    if pos + 2 <= raw.len() && pos >= 2 && raw[pos - 2..pos] == [0, b';'] {
        &raw[..pos - 2]
    } else {
        raw
    }
}

fn joliet_matches(raw: &[u8], candidate: &str) -> bool {
    if raw.len() % 2 != 0 {
        return false;
    }
    let units = raw
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    let mut expected = candidate.chars();
    for decoded in decode_utf16(units) {
        let Ok(ch) = decoded else {
            return false;
        };
        if expected.next() != Some(ch) {
            return false;
        }
    }
    expected.next().is_none()
}

/// Copies the Rock Ridge alternate name (SUSP `NM` entries) out of an inline
/// system-use area into `out`, concatenating a `CONTINUE`-flagged run. Returns
/// the number of bytes written, or `None` when there is no usable `NM` entry.
///
/// Only the inline system-use area is parsed; a name continued into a SUSP `CE`
/// continuation area is not followed, which keeps this allocation- and I/O-free.
/// The `CURRENT`/`PARENT` NM aliases (the "." and ".." names) yield `None`.
fn rrip_nm_into(su: &[u8], out: &mut [u8]) -> Option<usize> {
    const NM_CONTINUE: u8 = 0x01;
    const NM_CURRENT: u8 = 0x02;
    const NM_PARENT: u8 = 0x04;

    let mut written = 0usize;
    let mut found = false;
    let mut i = 0usize;
    // Each SUSP entry is [SIG0, SIG1, LEN, VER, ...]; LEN covers the whole entry.
    while i + 4 <= su.len() {
        let len = su[i + 2] as usize;
        if len < 4 || i + len > su.len() {
            break;
        }
        if &su[i..i + 2] == b"ST" {
            break; // SUSP terminator
        }
        if &su[i..i + 2] == b"NM" && len >= 5 {
            let flags = su[i + 4];
            if flags & (NM_CURRENT | NM_PARENT) != 0 {
                return None;
            }
            let content = &su[i + 5..i + len];
            if written + content.len() > out.len() {
                return None; // caller buffer too small for the full name
            }
            out[written..written + content.len()].copy_from_slice(content);
            written += content.len();
            found = true;
            if flags & NM_CONTINUE == 0 {
                break;
            }
        }
        i += len;
    }

    found.then_some(written)
}

fn decode_joliet_into<'a>(raw: &[u8], output: &'a mut [u8]) -> Result<&'a str, NameError> {
    if raw.len() % 2 != 0 {
        return Err(NameError::InvalidEncoding);
    }
    let units = raw
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    let mut written = 0;
    for decoded in decode_utf16(units) {
        let ch = decoded.map_err(|_| NameError::InvalidEncoding)?;
        let needed = ch.len_utf8();
        if written + needed > output.len() {
            return Err(NameError::BufferTooSmall);
        }
        ch.encode_utf8(&mut output[written..written + needed]);
        written += needed;
    }
    core::str::from_utf8(&output[..written]).map_err(|_| NameError::InvalidEncoding)
}

#[derive(Debug, Clone, Copy)]
struct DirectoryLocation {
    extent: u32,
    size: u32,
    block_size: u16,
    namespace: IsoNamespace,
}

impl From<IsoRoot> for DirectoryLocation {
    fn from(root: IsoRoot) -> Self {
        Self {
            extent: root.extent,
            size: root.size,
            block_size: root.block_size,
            namespace: root.namespace,
        }
    }
}

/// An owned, fixed-size directory entry returned by the allocation-free reader.
#[derive(Debug, Clone, Copy)]
pub struct IsoDirEntry {
    record: DirectoryRecord,
    directory: DirectoryLocation,
    continuation_offset: Option<u32>,
    total_size: u64,
}

impl IsoDirEntry {
    /// Returns the underlying ISO directory record.
    pub const fn record(&self) -> &DirectoryRecord {
        &self.record
    }

    /// Returns an allocation-free view of the filename.
    pub fn name(&self) -> IsoName<'_> {
        IsoName {
            raw: self.record.name(),
            namespace: self.directory.namespace,
        }
    }

    /// Returns whether this entry is a directory.
    pub fn is_directory(&self) -> bool {
        self.record.is_directory()
    }

    /// Returns whether this entry is a regular file.
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }

    /// Returns whether the file uses multiple extents.
    pub const fn is_multi_extent(&self) -> bool {
        self.continuation_offset.is_some()
    }

    /// Returns the total data length across all file extents.
    pub const fn total_size(&self) -> u64 {
        self.total_size
    }

    /// Returns the raw system-use area.
    pub fn system_use(&self) -> &[u8] {
        self.record.system_use()
    }

    /// Resolves the Rock Ridge alternate name (SUSP `NM`) into `output`, without
    /// allocating.
    ///
    /// Returns `None` when the entry carries no usable `NM` field — a plain
    /// ISO 9660/Joliet image, or a "."/".." alias — in which case
    /// [`Self::name`] is the name to use. Otherwise the RRIP name is validated
    /// as UTF-8 and written into `output`. Only the inline system-use area is
    /// parsed; a name continued into a SUSP `CE` area is not followed.
    pub fn rrip_name_into<'b>(&self, output: &'b mut [u8]) -> Option<Result<&'b str, NameError>> {
        let len = rrip_nm_into(self.system_use(), output)?;
        Some(core::str::from_utf8(&output[..len]).map_err(|_| NameError::InvalidEncoding))
    }

    /// Returns `true` if this entry's Rock Ridge name equals `candidate`.
    ///
    /// Returns `false` when the entry has no `NM` field or the name does not
    /// fit POSIX `NAME_MAX` (255 bytes). Allocation-free.
    pub fn rrip_name_matches(&self, candidate: &str) -> bool {
        let mut buffer = [0u8; 255];
        match rrip_nm_into(self.system_use(), &mut buffer) {
            Some(len) => &buffer[..len] == candidate.as_bytes(),
            None => false,
        }
    }
}

/// Allocation-free ISO 9660 image reader.
#[derive(Debug)]
pub struct IsoReader<R> {
    source: R,
    primary: IsoRoot,
    joliet: Option<IsoRoot>,
}

impl<R> IsoReader<R> {
    /// Consumes the reader and returns its underlying data source.
    pub fn into_inner(self) -> R {
        self.source
    }

    /// Returns the primary ISO 9660 root.
    pub const fn primary_root(&self) -> IsoRoot {
        self.primary
    }

    /// Returns the highest-level recognized Joliet root, when present.
    pub const fn joliet_root(&self) -> Option<IsoRoot> {
        self.joliet
    }

    /// Returns the preferred root, choosing Joliet over the primary namespace.
    pub const fn preferred_root(&self) -> IsoRoot {
        match self.joliet {
            Some(root) => root,
            None => self.primary,
        }
    }

    /// Iterates over the primary root followed by the optional Joliet root.
    pub fn roots(&self) -> impl Iterator<Item = IsoRoot> {
        [Some(self.primary), self.joliet].into_iter().flatten()
    }

    /// Selects a root by namespace.
    pub fn root(&self, namespace: IsoNamespace) -> Option<IsoRoot> {
        match namespace {
            IsoNamespace::Primary => Some(self.primary),
            IsoNamespace::Joliet(level) => self
                .joliet
                .filter(|root| root.namespace == IsoNamespace::Joliet(level)),
        }
    }
}

io_transform! {
impl<R: Read + Seek> IsoReader<R> {
    /// Opens an ISO image without allocating.
    pub async fn open(mut source: R) -> io::Result<Self> {
        let mut primary = None;
        let mut joliet = None;
        let mut sector = FIRST_DESCRIPTOR;
        loop {
            source.seek(SeekFrom::Start(sector.checked_mul(DESCRIPTOR_SIZE).ok_or_else(overflow)?))
                .await.map_err(io::Error::erase)?;
            let mut bytes = [0_u8; DESCRIPTOR_SIZE as usize];
            source.read_exact(&mut bytes).await?;
            let header = VolumeDescriptorHeader::from_bytes(&bytes[..7]);
            if !header.is_valid() {
                return Err(invalid("invalid volume descriptor header"));
            }
            let ty = VolumeDescriptorType::from_u8(header.descriptor_type);
            if ty == VolumeDescriptorType::VolumeSetTerminator { break; }
            match VolumeDescriptor::new(bytes) {
                VolumeDescriptor::Primary(descriptor) => primary = Some(root_from_primary(&descriptor)?),
                VolumeDescriptor::Supplementary(descriptor) => {
                    if let Some(root) = root_from_joliet(&descriptor)?
                        && joliet.map(|old: IsoRoot| joliet_level(old) < joliet_level(root)).unwrap_or(true)
                    { joliet = Some(root); }
                }
                _ => {}
            }
            sector = sector.checked_add(1).ok_or_else(overflow)?;
        }
        let primary = primary.ok_or_else(|| invalid("primary volume descriptor not found"))?;
        Ok(Self { source, primary, joliet })
    }

    /// Opens an allocation-free directory reader.
    pub fn open_dir(&mut self, root: IsoRoot) -> IsoDirReader<'_, R> {
        IsoDirReader { image: self, directory: root.into(), offset: 0 }
    }

    /// Finds an entry using the preferred namespace.
    pub async fn find_path(&mut self, path: &str) -> io::Result<Option<IsoDirEntry>> {
        self.find_path_in(self.preferred_root(), path).await
    }

    /// Finds an entry in an explicitly selected directory tree.
    pub async fn find_path_in(&mut self, root: IsoRoot, path: &str) -> io::Result<Option<IsoDirEntry>> {
        let mut location = DirectoryLocation::from(root);
        let mut components = hadris_path::VPath::with_separators(
            path, hadris_path::Separators::SlashOrBackslash
        ).components().filter_map(|component| match component {
            hadris_path::Component::Root | hadris_path::Component::Current => None,
            other => Some(other),
        }).peekable();

        while let Some(component) = components.next() {
            let hadris_path::Component::Normal(name) = component else {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "parent path components are not supported"));
            };
            let mut offset = 0;
            let mut found = None;
            while let Some(entry) = self.next_entry(location, &mut offset).await? {
                let flags = FileFlags::from_bits_retain(entry.record.header().flags);
                if !entry.record.is_special()
                    && !flags.contains(FileFlags::ASSOCIATED_FILE)
                    && entry.name().matches(name)
                { found = Some(entry); break; }
            }
            let Some(entry) = found else { return Ok(None); };
            if components.peek().is_none() { return Ok(Some(entry)); }
            if !entry.is_directory() { return Ok(None); }
            location = entry_directory(&entry)?;
        }
        Ok(None)
    }

    /// Opens a caller-buffered stream for a file entry.
    pub fn open_file<'a>(&'a mut self, entry: &IsoDirEntry) -> io::Result<IsoFileReader<'a, R>> {
        validate_file(entry)?;
        Ok(IsoFileReader {
            image: self,
            entry: *entry,
            current_record: entry.record,
            current_extent_start: 0,
            continuation_cursor: entry.continuation_offset,
            position: 0,
        })
    }

    async fn next_entry(&mut self, directory: DirectoryLocation, offset: &mut u32) -> io::Result<Option<IsoDirEntry>> {
        let Some(record) = self.next_raw(directory, offset).await? else { return Ok(None); };
        let first_flags = FileFlags::from_bits_retain(record.header().flags);
        let continuation_offset = first_flags.contains(FileFlags::NOT_FINAL).then_some(*offset);
        let mut total_size = record.header().data_len.read() as u64;
        if continuation_offset.is_some() {
            loop {
                let continuation = self.next_raw(directory, offset).await?
                    .ok_or_else(|| invalid("truncated multi-extent file"))?;
                validate_continuation(&record, &continuation)?;
                total_size = total_size.checked_add(continuation.header().data_len.read() as u64)
                    .ok_or_else(overflow)?;
                if !FileFlags::from_bits_retain(continuation.header().flags).contains(FileFlags::NOT_FINAL) { break; }
            }
        }
        Ok(Some(IsoDirEntry { record, directory, continuation_offset, total_size }))
    }

    async fn next_raw(&mut self, directory: DirectoryLocation, offset: &mut u32) -> io::Result<Option<DirectoryRecord>> {
        let block = directory.block_size as u32;
        while *offset < directory.size {
            let base = byte_offset(directory.extent, directory.block_size)?;
            let absolute = base.checked_add(*offset as u64).ok_or_else(overflow)?;
            self.source.seek(SeekFrom::Start(absolute)).await.map_err(io::Error::erase)?;
            let mut len = [0_u8; 1];
            self.source.read_exact(&mut len).await?;
            if len[0] == 0 {
                let next = ((*offset / block) + 1).checked_mul(block).ok_or_else(overflow)?;
                if next <= *offset { return Err(invalid("directory offset did not advance")); }
                *offset = next.min(directory.size);
                continue;
            }
            let len = len[0] as usize;
            if len < MIN_RECORD_SIZE || (*offset % block) as usize + len > block as usize {
                return Err(invalid("invalid directory record length"));
            }
            let mut header_bytes = [0_u8; core::mem::size_of::<DirectoryRecordHeader>()];
            header_bytes[0] = len as u8;
            self.source.read_exact(&mut header_bytes[1..]).await?;
            let header: DirectoryRecordHeader = bytemuck::pod_read_unaligned(&header_bytes);
            let name_end = MIN_RECORD_SIZE - 1 + header.file_identifier_len as usize;
            if name_end > len { return Err(invalid("directory identifier exceeds record")); }
            self.source.seek(SeekFrom::Start(absolute)).await.map_err(io::Error::erase)?;
            let record = DirectoryRecord::parse(&mut self.source).await?;
            *offset = offset.checked_add(len as u32).ok_or_else(overflow)?;
            return Ok(Some(record));
        }
        Ok(None)
    }

}
} // io_transform!

/// Streaming allocation-free directory reader.
pub struct IsoDirReader<'a, R> {
    image: &'a mut IsoReader<R>,
    directory: DirectoryLocation,
    offset: u32,
}

io_transform! {
impl<R: Read + Seek> IsoDirReader<'_, R> {
    /// Reads the next logical directory entry.
    pub async fn next_entry(&mut self) -> io::Result<Option<IsoDirEntry>> {
        self.image.next_entry(self.directory, &mut self.offset).await
    }
}
} // io_transform!

sync_only! {
impl<R: Read + Seek> Iterator for IsoDirReader<'_, R> {
    type Item = io::Result<IsoDirEntry>;
    fn next(&mut self) -> Option<Self::Item> { self.next_entry().transpose() }
}
}

/// Caller-buffered file stream backed by an [`IsoReader`].
pub struct IsoFileReader<'a, R> {
    image: &'a mut IsoReader<R>,
    entry: IsoDirEntry,
    current_record: DirectoryRecord,
    current_extent_start: u64,
    continuation_cursor: Option<u32>,
    position: u64,
}

impl<R> IsoFileReader<'_, R> {
    /// Returns the complete logical file length.
    pub const fn len(&self) -> u64 {
        self.entry.total_size
    }

    /// Returns whether the file contains no bytes.
    pub const fn is_empty(&self) -> bool {
        self.entry.total_size == 0
    }

    /// Returns the current logical read position.
    pub const fn position(&self) -> u64 {
        self.position
    }
}

io_transform! {
impl<R: Read + Seek> IsoFileReader<'_, R> {
    /// Reads the next chunk into a caller-provided buffer.
    pub async fn read_chunk(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.entry.total_size || output.is_empty() {
            return Ok(0);
        }

        let wanted = output
            .len()
            .min((self.entry.total_size - self.position) as usize);
        let mut written = 0;
        while written < wanted {
            let extent_len = self.current_record.header().data_len.read() as u64;
            let within = self.position.checked_sub(self.current_extent_start)
                .ok_or_else(|| invalid("file reader extent state is invalid"))?;
            if within < extent_len {
                let available = (extent_len - within) as usize;
                let take = available.min(wanted - written);
                let header = self.current_record.header();
                let xar = header.extended_attr_record as u64
                    * self.entry.directory.block_size as u64;
                let absolute = byte_offset(header.extent.read(), self.entry.directory.block_size)?
                    .checked_add(xar)
                    .and_then(|value| value.checked_add(within))
                    .ok_or_else(overflow)?;
                self.image.source.seek(SeekFrom::Start(absolute)).await
                    .map_err(io::Error::erase)?;
                self.image.source.read_exact(&mut output[written..written + take]).await?;
                written += take;
                self.position += take as u64;
            }

            if written == wanted {
                break;
            }
            self.advance_extent(extent_len).await?;
        }
        Ok(written)
    }

    async fn advance_extent(&mut self, extent_len: u64) -> io::Result<()> {
        self.current_extent_start = self.current_extent_start
            .checked_add(extent_len)
            .ok_or_else(overflow)?;
        let Some(ref mut cursor) = self.continuation_cursor else {
            return Err(invalid("file extent chain ended before its declared size"));
        };
        let next = self.image.next_raw(self.entry.directory, cursor).await?
            .ok_or_else(|| invalid("truncated multi-extent file"))?;
        validate_continuation(&self.entry.record, &next)?;
        if !FileFlags::from_bits_retain(next.header().flags).contains(FileFlags::NOT_FINAL) {
            self.continuation_cursor = None;
        }
        self.current_record = next;
        Ok(())
    }
}
} // io_transform!

fn root_from_primary(descriptor: &PrimaryVolumeDescriptor) -> io::Result<IsoRoot> {
    make_root(
        IsoNamespace::Primary,
        descriptor.logical_block_size.read(),
        descriptor.dir_record.header.extent.read(),
        descriptor.dir_record.header.data_len.read(),
        descriptor.dir_record.header.extended_attr_record,
    )
}

fn root_from_joliet(descriptor: &SupplementaryVolumeDescriptor) -> io::Result<Option<IsoRoot>> {
    let Some(level) = JolietLevel::from_escape_sequence(&descriptor.escape_sequences) else {
        return Ok(None);
    };
    Ok(Some(make_root(
        IsoNamespace::Joliet(level),
        descriptor.logical_block_size.read(),
        descriptor.dir_record.header.extent.read(),
        descriptor.dir_record.header.data_len.read(),
        descriptor.dir_record.header.extended_attr_record,
    )?))
}

fn joliet_level(root: IsoRoot) -> JolietLevel {
    match root.namespace {
        IsoNamespace::Joliet(level) => level,
        IsoNamespace::Primary => unreachable!("primary root used as Joliet root"),
    }
}

fn make_root(
    namespace: IsoNamespace,
    block_size: u16,
    extent: u32,
    size: u32,
    xar_blocks: u8,
) -> io::Result<IsoRoot> {
    if block_size < 512 || !block_size.is_power_of_two() {
        return Err(invalid("invalid logical block size"));
    }
    let extent = extent.checked_add(xar_blocks as u32).ok_or_else(overflow)?;
    byte_offset(extent, block_size)?;
    Ok(IsoRoot {
        namespace,
        extent,
        size,
        block_size,
    })
}

fn entry_directory(entry: &IsoDirEntry) -> io::Result<DirectoryLocation> {
    if !entry.is_directory() {
        return Err(invalid("entry is not a directory"));
    }
    Ok(DirectoryLocation {
        extent: entry
            .record
            .header()
            .extent
            .read()
            .checked_add(entry.record.header().extended_attr_record as u32)
            .ok_or_else(overflow)?,
        size: entry.record.header().data_len.read(),
        block_size: entry.directory.block_size,
        namespace: entry.directory.namespace,
    })
}

fn validate_file(entry: &IsoDirEntry) -> io::Result<()> {
    if entry.is_directory() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "entry is a directory",
        ));
    }
    let header = entry.record.header();
    if header.file_unit_size != 0 || header.interleave_gap_size != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "interleaved files are unsupported",
        ));
    }
    if header.volume_sequence_number.read() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "multi-volume files are unsupported",
        ));
    }
    Ok(())
}

fn validate_continuation(first: &DirectoryRecord, next: &DirectoryRecord) -> io::Result<()> {
    if first.name() != next.name()
        || next.is_directory()
        || FileFlags::from_bits_retain(next.header().flags).contains(FileFlags::ASSOCIATED_FILE)
    {
        return Err(invalid("invalid multi-extent continuation"));
    }
    Ok(())
}

fn byte_offset(extent: u32, block_size: u16) -> io::Result<u64> {
    (extent as u64)
        .checked_mul(block_size as u64)
        .ok_or_else(overflow)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn overflow() -> io::Error {
    invalid("ISO offset overflow")
}
