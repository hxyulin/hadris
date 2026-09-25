//! Mode-independent encoding and decoding of exFAT on-disk values.

use hadris_fs::{DateTime, ErrorKind};

use super::layout::{
    self as raw, BootSector, ENTRY_SIZE, MAX_NAME_UNITS, NAME_UNITS_PER_ENTRY, UTC_OFFSET_VALID,
};
use crate::date;

/// One directory entry.
pub type RawEntry = [u8; ENTRY_SIZE];

/// The most entries a File entry set can have.
pub const MAX_SET: usize = 19;

/// Where the volume's structures are, from a boot sector [`parse_boot`]
/// accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    sector_shift: u8,
    cluster_shift: u8,
    volume_len: u64,
    /// Byte offset of the active FAT.
    fat_start: u64,
    /// Byte offset of the second FAT of a TexFAT volume that is not active.
    mirror_fat: Option<u64>,
    /// The Allocation Bitmap entries use this BitmapIdentifier.
    active: u8,
    heap_start: u64,
    cluster_count: u32,
    root: u32,
    serial: u32,
    flags: u16,
    percent_in_use: u8,
}

impl Geometry {
    /// `BytesPerSectorShift`: log2 of the sector size.
    pub const fn sector_shift(&self) -> u8 {
        self.sector_shift
    }

    /// Log2 of the cluster size.
    pub const fn cluster_shift(&self) -> u8 {
        self.cluster_shift
    }

    /// The volume's length in bytes.
    pub const fn volume_len(&self) -> u64 {
        self.volume_len
    }

    /// Byte offset of the active FAT.
    pub const fn fat_start(&self) -> u64 {
        self.fat_start
    }

    /// Byte offset of the second FAT of a TexFAT volume, the one that is
    /// not active.
    pub const fn mirror_fat(&self) -> Option<u64> {
        self.mirror_fat
    }

    /// The `BitmapIdentifier` of the active Allocation Bitmap: 1 when
    /// `VolumeFlags` names the second FAT active, else 0.
    pub const fn active(&self) -> u8 {
        self.active
    }

    /// Byte offset of the cluster heap.
    pub const fn heap_start(&self) -> u64 {
        self.heap_start
    }

    /// `ClusterCount`.
    pub const fn cluster_count(&self) -> u32 {
        self.cluster_count
    }

    /// `FirstClusterOfRootDirectory`.
    pub const fn root(&self) -> u32 {
        self.root
    }

    /// `VolumeSerialNumber`.
    pub const fn volume_serial(&self) -> u32 {
        self.serial
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn set_volume_serial(&mut self, serial: u32) {
        self.serial = serial;
    }

    /// `VolumeFlags` as the boot sector holds them.
    pub const fn flags(&self) -> u16 {
        self.flags
    }

    /// `PercentInUse` as the boot sector holds it.
    pub const fn percent_in_use(&self) -> u8 {
        self.percent_in_use
    }

    /// Bytes per sector.
    pub const fn sector_size(&self) -> u64 {
        1 << self.sector_shift
    }

    /// Bytes per cluster.
    pub const fn cluster_size(&self) -> u64 {
        1 << self.cluster_shift
    }

    /// The highest cluster number.
    pub const fn max_cluster(&self) -> u32 {
        self.cluster_count + 1
    }

    /// Whether `cluster` is a heap cluster.
    pub const fn is_cluster(&self, cluster: u32) -> bool {
        cluster >= raw::FIRST_CLUSTER && cluster <= self.max_cluster()
    }

    /// Byte offset of a heap cluster.
    pub fn cluster_offset(&self, cluster: u32) -> Option<u64> {
        self.is_cluster(cluster).then(|| {
            self.heap_start + (((cluster - raw::FIRST_CLUSTER) as u64) << self.cluster_shift)
        })
    }

    /// The heap cluster holding byte `offset`.
    pub fn cluster_of(&self, offset: u64) -> Option<u32> {
        let rel = offset.checked_sub(self.heap_start)? >> self.cluster_shift;
        let cluster = u32::try_from(rel).ok()?.checked_add(raw::FIRST_CLUSTER)?;
        self.is_cluster(cluster).then_some(cluster)
    }
}

/// Checks a boot sector and returns its geometry, or the name of the first
/// field that is wrong.
pub fn parse_boot(boot: &BootSector) -> Result<Geometry, &'static str> {
    if boot.file_system_name != raw::FILE_SYSTEM_NAME {
        return Err("FileSystemName");
    }
    if boot.boot_signature.get() != raw::BOOT_SIGNATURE {
        return Err("BootSignature");
    }
    if boot.must_be_zero.iter().any(|&byte| byte != 0) {
        return Err("MustBeZero");
    }
    if boot.file_system_revision.get() >> 8 != 1 {
        return Err("FileSystemRevision");
    }
    let sector_shift = boot.bytes_per_sector_shift;
    if !(9..=12).contains(&sector_shift) {
        return Err("BytesPerSectorShift");
    }
    if boot.sectors_per_cluster_shift > 25 - sector_shift {
        return Err("SectorsPerClusterShift");
    }
    let fats = boot.number_of_fats as u64;
    if fats != 1 && fats != 2 {
        return Err("NumberOfFats");
    }
    let sector = 1u64 << sector_shift;
    let volume_len = boot.volume_length.get();
    let fat_offset = boot.fat_offset.get() as u64;
    let fat_length = boot.fat_length.get() as u64;
    let heap_offset = boot.cluster_heap_offset.get() as u64;
    let count = boot.cluster_count.get();
    if volume_len < (1 << 20) >> sector_shift || volume_len.checked_mul(sector).is_none() {
        return Err("VolumeLength");
    }
    if fat_offset < 24 || fat_offset + fat_length * fats > heap_offset {
        return Err("FatOffset");
    }
    if (count as u64 + 2) * 4 > fat_length * sector {
        return Err("FatLength");
    }
    if heap_offset > volume_len {
        return Err("ClusterHeapOffset");
    }
    let cluster_shift = sector_shift + boot.sectors_per_cluster_shift;
    if count == 0
        || count > raw::MAX_CLUSTER_COUNT
        || count as u64 > (volume_len - heap_offset) >> boot.sectors_per_cluster_shift
    {
        return Err("ClusterCount");
    }
    let flags = boot.volume_flags.get();
    let active = (fats == 2 && flags & raw::VOLUME_ACTIVE_FAT != 0) as u64;
    let fat_at = |index: u64| (fat_offset + fat_length * index) * sector;
    let geometry = Geometry {
        sector_shift,
        cluster_shift,
        volume_len: volume_len * sector,
        fat_start: fat_at(active),
        mirror_fat: (fats == 2).then(|| fat_at(1 - active)),
        active: active as u8,
        heap_start: heap_offset * sector,
        cluster_count: count,
        root: boot.first_cluster_of_root_directory.get(),
        serial: boot.volume_serial_number.get(),
        flags,
        percent_in_use: boot.percent_in_use,
    };
    if !geometry.is_cluster(geometry.root) {
        return Err("FirstClusterOfRootDirectory");
    }
    Ok(geometry)
}

/// Adds `bytes` of sector `index` of a boot region to a boot checksum.
pub fn boot_checksum(mut sum: u32, index: u64, bytes: &[u8]) -> u32 {
    for (at, &byte) in bytes.iter().enumerate() {
        if index == 0 && raw::CHECKSUM_SKIPPED.contains(&at) {
            continue;
        }
        sum = sum.rotate_right(1).wrapping_add(byte as u32);
    }
    sum
}

/// Adds `bytes` to an up-case table checksum.
pub fn table_checksum(mut sum: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        sum = sum.rotate_right(1).wrapping_add(byte as u32);
    }
    sum
}

/// The `SetChecksum` of an entry set.
pub fn set_checksum(entries: &[RawEntry]) -> u16 {
    let mut sum = 0u16;
    for (index, entry) in entries.iter().enumerate() {
        for (at, &byte) in entry.iter().enumerate() {
            if index == 0 && (at == 2 || at == 3) {
                continue;
            }
            sum = sum.rotate_right(1).wrapping_add(byte as u16);
        }
    }
    sum
}

/// Stores the checksum of `entries` in its primary entry.
pub fn seal(entries: &mut [RawEntry]) {
    let sum = set_checksum(entries);
    entries[0][2..4].copy_from_slice(&sum.to_le_bytes());
}

/// The `NameHash` of up-cased code units.
pub fn name_hash(upcased: &[u16]) -> u16 {
    upcased.iter().fold(0, |hash, &unit| hash_unit(hash, unit))
}

/// Adds one up-cased code unit to a `NameHash` that started at 0.
pub fn hash_unit(mut hash: u16, unit: u16) -> u16 {
    for byte in unit.to_le_bytes() {
        hash = hash.rotate_right(1).wrapping_add(byte as u16);
    }
    hash
}

/// Whether `unit` may appear in a file name or label.
pub fn valid_unit(unit: u16) -> bool {
    unit >= 0x20
        && !matches!(
            unit,
            0x22 | 0x2A | 0x2F | 0x3A | 0x3C | 0x3E | 0x3F | 0x5C | 0x7C
        )
}

/// A name in UTF-16 code units, at most 255 of them.
#[derive(Clone, Copy)]
pub struct NameUnits {
    units: [u16; MAX_NAME_UNITS],
    len: usize,
}

impl Default for NameUnits {
    fn default() -> Self {
        Self::new()
    }
}

impl NameUnits {
    /// The empty name.
    pub const fn new() -> Self {
        Self {
            units: [0; MAX_NAME_UNITS],
            len: 0,
        }
    }

    /// Appends `unit`; `false` when the name already holds 255.
    pub fn push(&mut self, unit: u16) -> bool {
        if self.len == MAX_NAME_UNITS {
            return false;
        }
        self.units[self.len] = unit;
        self.len += 1;
        true
    }

    /// The number of code units.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the name is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The code units, to change in place.
    pub fn as_mut_slice(&mut self) -> &mut [u16] {
        &mut self.units[..self.len]
    }

    /// The code units.
    pub fn as_slice(&self) -> &[u16] {
        &self.units[..self.len]
    }

    /// Encodes a name for a new entry. Fails with `NameTooLong` above 255
    /// units, and with `InvalidInput` for an empty name, `.`, `..`, a
    /// character exFAT forbids, or a trailing dot or space.
    pub fn encode(text: &str) -> Result<Self, ErrorKind> {
        if text.is_empty() || text == "." || text == ".." || text.ends_with(['.', ' ']) {
            return Err(ErrorKind::InvalidInput);
        }
        let mut name = Self::new();
        for unit in text.encode_utf16() {
            if !valid_unit(unit) {
                return Err(ErrorKind::InvalidInput);
            }
            if name.len == MAX_NAME_UNITS {
                return Err(ErrorKind::NameTooLong);
            }
            name.units[name.len] = unit;
            name.len += 1;
        }
        Ok(name)
    }

    /// Converts a lookup query; `None` when it cannot name an entry.
    pub fn query(text: &str) -> Option<Self> {
        let mut name = Self::new();
        for unit in text.encode_utf16() {
            if name.len == MAX_NAME_UNITS {
                return None;
            }
            name.units[name.len] = unit;
            name.len += 1;
        }
        (name.len > 0).then_some(name)
    }

    /// File Name entries the name needs.
    pub fn entries(&self) -> usize {
        self.len.div_ceil(NAME_UNITS_PER_ENTRY)
    }
}

/// Encodes a time as `(Timestamp, 10msIncrement, UtcOffset)`. A zone-less
/// time is stored with no valid offset; an offset that is not a whole
/// number of quarter hours in range is stored as UTC.
pub fn encode_time(time: DateTime) -> (u32, u8, u8) {
    let (time, offset) = match time.utc_offset_minutes() {
        None => (time, 0),
        Some(minutes) if minutes % 15 == 0 && (-64 * 15..=63 * 15).contains(&minutes) => {
            (time, UTC_OFFSET_VALID | ((minutes / 15) as i8 as u8 & 0x7F))
        }
        Some(_) => (
            time.with_utc_offset_minutes(Some(0)).unwrap_or(time),
            UTC_OFFSET_VALID,
        ),
    };
    let (packed_date, packed_time, tenths) = date::encode(time, None);
    (
        (packed_date as u32) << 16 | packed_time as u32,
        tenths,
        offset,
    )
}

/// Decodes a stored time; `None` when its fields are out of range. A time
/// with no valid offset is read as local time in `zone`, minutes east of
/// UTC, as [`date::decode`] reads it.
pub fn decode_time(stamp: u32, increment: u8, offset: u8, zone: Option<i16>) -> Option<DateTime> {
    if offset & UTC_OFFSET_VALID == 0 {
        return date::decode((stamp >> 16) as u16, stamp as u16, increment, zone);
    }
    let local = date::decode((stamp >> 16) as u16, stamp as u16, increment, None)?;
    let minutes = (((offset << 1) as i8) >> 1) as i16 * 15;
    DateTime::new(
        local.unix_seconds() - minutes as i64 * 60,
        local.nanoseconds(),
    )
    .ok()?
    .with_utc_offset_minutes(Some(minutes))
    .ok()
}

/// Where the decoding of one 256-unit page of the up-case table starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageStart {
    /// The table unit to continue from.
    unit: u32,
    /// Identity mappings left from a run that began in an earlier page.
    run: u16,
}

/// Decodes a compressed or plain up-case table one unit at a time.
#[derive(Debug, Clone, Copy, Default)]
pub struct UpcaseDecoder {
    /// The next code point to map.
    code: u32,
    /// The last unit was an identity-run marker.
    marker: bool,
    /// Identity mappings left in the current run.
    run: u32,
    /// The index of the next table unit.
    next: u32,
}

impl UpcaseDecoder {
    /// The state that continues at `start`, the start of code point `code`.
    pub const fn resume(start: PageStart, code: u32) -> Self {
        Self {
            code,
            marker: false,
            run: start.run as u32,
            next: start.unit,
        }
    }

    /// Takes the table unit at index `next` and passes each mapping it
    /// completes to `emit`, with the state that resumes at that mapping.
    pub fn feed(&mut self, unit: u16, emit: &mut impl FnMut(u32, u16, PageStart)) {
        let index = self.next;
        self.next += 1;
        if self.marker {
            self.marker = false;
            self.run = unit as u32;
            self.drain(emit);
            return;
        }
        if self.code > 0xFFFF {
            return;
        }
        if unit == 0xFFFF && self.code != 0xFFFF {
            self.marker = true;
            return;
        }
        emit(
            self.code,
            unit,
            PageStart {
                unit: index,
                run: 0,
            },
        );
        self.code += 1;
    }

    /// The next code point to map.
    pub const fn code(&self) -> u32 {
        self.code
    }

    /// The index of the next table unit to feed.
    pub const fn next(&self) -> u32 {
        self.next
    }

    /// Emits the identity mappings left in the current run.
    pub fn drain(&mut self, emit: &mut impl FnMut(u32, u16, PageStart)) {
        while self.run > 0 && self.code <= 0xFFFF {
            let start = PageStart {
                unit: self.next,
                run: self.run as u16,
            };
            emit(self.code, self.code as u16, start);
            self.code += 1;
            self.run -= 1;
        }
        self.run = 0;
    }
}

/// The mandatory mapping of the first 128 code points.
pub const fn mandatory_upcase(code: u16) -> u16 {
    if code >= b'a' as u16 && code <= b'z' as u16 {
        code - 0x20
    } else {
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hadris_fs::{CivilDate, CivilTime};

    fn decode_table(table: &[u8]) -> std::vec::Vec<u16> {
        let mut out: std::vec::Vec<u16> = (0..=0xFFFF).collect();
        let mut decoder = UpcaseDecoder::default();
        for pair in table.chunks_exact(2) {
            decoder.feed(
                u16::from_le_bytes([pair[0], pair[1]]),
                &mut |code, upper, _| {
                    out[code as usize] = upper;
                },
            );
        }
        out
    }

    #[test]
    fn recommended_table_decodes() {
        let table = decode_table(raw::RECOMMENDED_UPCASE_TABLE);
        assert_eq!(
            table_checksum(0, raw::RECOMMENDED_UPCASE_TABLE),
            raw::RECOMMENDED_UPCASE_CHECKSUM
        );
        assert_eq!(table[b'a' as usize], b'A' as u16);
        assert_eq!(table[0xE9], 0xC9);
        assert_eq!(table[0x3B1], 0x391);
        assert_eq!(table[0x430], 0x410);
        assert_eq!(table[0xFF41], 0xFF21);
        assert_eq!(table[0xFFFF], 0xFFFF);
        for code in 0..128u16 {
            assert_eq!(table[code as usize], mandatory_upcase(code));
        }
    }

    #[test]
    fn decoding_resumes_at_any_code_point() {
        let full = decode_table(raw::RECOMMENDED_UPCASE_TABLE);
        let mut starts = std::vec![PageStart::default(); 0x10000];
        let mut decoder = UpcaseDecoder::default();
        for pair in raw::RECOMMENDED_UPCASE_TABLE.chunks_exact(2) {
            decoder.feed(
                u16::from_le_bytes([pair[0], pair[1]]),
                &mut |code, _, start| {
                    starts[code as usize] = start;
                },
            );
        }
        let units: std::vec::Vec<u16> = raw::RECOMMENDED_UPCASE_TABLE
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        for code in [0x61u32, 0x100, 0x3B1, 0x1000, 0x8000, 0xFF41, 0xFFFF] {
            let mut decoder = UpcaseDecoder::resume(starts[code as usize], code);
            let mut got = None;
            let mut emit = |at: u32, upper: u16, _| {
                if at == code {
                    got = Some(upper);
                }
            };
            decoder.drain(&mut emit);
            while decoder.code <= code && (decoder.next as usize) < units.len() {
                decoder.feed(units[decoder.next as usize], &mut emit);
            }
            assert_eq!(got, Some(full[code as usize]), "{code:#x}");
        }
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(NameUnits::encode("a.txt").unwrap().len, 5);
        assert_eq!(NameUnits::encode("\u{1F600}").unwrap().len, 2);
        for bad in [
            "", ".", "..", "a:b", "a*", "tail.", "tail ", "a\u{1}b", "a\\b",
        ] {
            assert_eq!(
                NameUnits::encode(bad).err(),
                Some(ErrorKind::InvalidInput),
                "{bad:?}"
            );
        }
        let long: std::string::String = "e".repeat(256);
        assert_eq!(NameUnits::encode(&long).err(), Some(ErrorKind::NameTooLong));
        assert_eq!(NameUnits::encode(&long[..255]).unwrap().entries(), 17);
    }

    #[test]
    fn name_hash_matches_the_specification() {
        let units: std::vec::Vec<u16> = "FILE.TXT".encode_utf16().collect();
        let mut expected = 0u16;
        for unit in &units {
            for byte in unit.to_le_bytes() {
                expected = (if expected & 1 != 0 { 0x8000u16 } else { 0 })
                    .wrapping_add(expected >> 1)
                    .wrapping_add(byte as u16);
            }
        }
        assert_eq!(name_hash(&units), expected);
    }

    #[test]
    fn times_round_trip_with_offsets() {
        let civil = |h| {
            DateTime::from_civil(
                CivilDate::new(2024, 2, 29).unwrap(),
                CivilTime::new(h, 45, 31).unwrap(),
                Some(-150),
            )
            .unwrap()
            .with_nanoseconds(560_000_000)
            .unwrap()
        };
        let time = civil(13);
        let (stamp, increment, offset) = encode_time(time);
        assert_eq!(offset, UTC_OFFSET_VALID | ((-10i8) as u8 & 0x7F));
        assert_eq!(increment, 156);
        assert_eq!(decode_time(stamp, increment, offset, None), Some(time));
        let odd = time.with_utc_offset_minutes(Some(7)).unwrap();
        let (stamp, increment, offset) = encode_time(odd);
        assert_eq!(offset, UTC_OFFSET_VALID);
        let back = decode_time(stamp, increment, offset, None).unwrap();
        assert_eq!(back.unix_seconds(), odd.unix_seconds());
        let plain = DateTime::from_unix_seconds(1_000_000_000).unwrap();
        let (stamp, increment, offset) = encode_time(plain);
        assert_eq!(offset, 0);
        assert_eq!(decode_time(stamp, increment, offset, None), Some(plain));
        assert_eq!(decode_time(0, 0, 0, None), None);
        let zoned = decode_time(stamp, increment, 0, Some(60)).unwrap();
        assert_eq!(zoned.unix_seconds(), plain.unix_seconds() - 3600);
        assert_eq!(zoned.utc_offset_minutes(), Some(60));
    }

    #[test]
    fn checksums_skip_their_own_fields() {
        let mut entries = [[0u8; ENTRY_SIZE]; 3];
        entries[0][0] = raw::ENTRY_FILE;
        entries[0][1] = 2;
        entries[1][0] = raw::ENTRY_STREAM;
        entries[2][0] = raw::ENTRY_NAME;
        seal(&mut entries);
        let sum = u16::from_le_bytes([entries[0][2], entries[0][3]]);
        entries[0][2] ^= 0xFF;
        assert_eq!(set_checksum(&entries), sum);
        let mut sector = [0u8; 512];
        sector[106] = 0xFF;
        sector[112] = 0x33;
        assert_eq!(boot_checksum(0, 0, &sector), 0);
        assert_ne!(boot_checksum(0, 1, &sector), 0);
    }
}
