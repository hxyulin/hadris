//! VFAT long file names: checksums, entry packing and assembly.

/// Set in the sequence byte of the first LFN entry on disk, which holds the
/// last part of the name.
pub(crate) const LAST_ENTRY: u8 = 0x40;
/// Bits of the sequence byte that hold the 1-based entry number.
pub(crate) const SEQUENCE_MASK: u8 = 0x3F;
/// UTF-16 code units stored per LFN entry.
pub(crate) const UNITS_PER_ENTRY: usize = 13;
/// The longest name in UTF-16 code units.
pub(crate) const MAX_UNITS: usize = 255;
/// The most LFN entries one name can use.
pub(crate) const MAX_ENTRIES: usize = 20;

/// The checksum of an 11-byte short name that ties LFN entries to it.
pub(crate) const fn checksum(short: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    let mut i = 0;
    while i < short.len() {
        sum = sum.rotate_right(1).wrapping_add(short[i]);
        i += 1;
    }
    sum
}

/// The code units of one LFN entry and how many of them precede the
/// `0x0000` terminator or `0xFFFF` filler.
pub(crate) fn unpack(
    name1: &[u8; 10],
    name2: &[u8; 12],
    name3: &[u8; 4],
) -> ([u16; UNITS_PER_ENTRY], usize) {
    let mut units = [0u16; UNITS_PER_ENTRY];
    let bytes = name1
        .chunks_exact(2)
        .chain(name2.chunks_exact(2))
        .chain(name3.chunks_exact(2));
    for (unit, pair) in units.iter_mut().zip(bytes) {
        *unit = u16::from_le_bytes([pair[0], pair[1]]);
    }
    let len = units
        .iter()
        .position(|&unit| unit == 0x0000 || unit == 0xFFFF)
        .unwrap_or(UNITS_PER_ENTRY);
    (units, len)
}

/// Splits 13 code units into the three name fields of an LFN entry.
pub(crate) fn pack(units: &[u16; UNITS_PER_ENTRY]) -> ([u8; 10], [u8; 12], [u8; 4]) {
    let mut name1 = [0u8; 10];
    let mut name2 = [0u8; 12];
    let mut name3 = [0u8; 4];
    let fields = name1
        .chunks_exact_mut(2)
        .chain(name2.chunks_exact_mut(2))
        .chain(name3.chunks_exact_mut(2));
    for (pair, unit) in fields.zip(units) {
        pair.copy_from_slice(&unit.to_le_bytes());
    }
    (name1, name2, name3)
}

/// A name encoded as the LFN entries that store it.
pub(crate) struct Encoded {
    units: [u16; MAX_ENTRIES * UNITS_PER_ENTRY],
    entries: usize,
}

impl Encoded {
    /// Encodes `name` as UTF-16, followed by a `0x0000` terminator and
    /// `0xFFFF` filler when it does not fill its last entry. `None` when the
    /// name is empty or longer than [`MAX_UNITS`].
    pub(crate) fn new(name: &str) -> Option<Self> {
        let mut units = [0u16; MAX_ENTRIES * UNITS_PER_ENTRY];
        let mut len = 0;
        for unit in name.encode_utf16() {
            if len >= MAX_UNITS {
                return None;
            }
            units[len] = unit;
            len += 1;
        }
        let entries = len.div_ceil(UNITS_PER_ENTRY);
        if entries == 0 {
            return None;
        }
        let capacity = entries * UNITS_PER_ENTRY;
        if len < capacity {
            units[len] = 0x0000;
            units[len + 1..capacity].fill(0xFFFF);
        }
        Some(Self { units, entries })
    }

    /// Number of LFN entries.
    pub(crate) fn entries(&self) -> usize {
        self.entries
    }

    /// The sequence byte and code units of the entry at `index` in disk
    /// order: index 0 comes first and carries [`LAST_ENTRY`].
    pub(crate) fn entry(&self, index: usize) -> (u8, [u16; UNITS_PER_ENTRY]) {
        let number = self.entries - index;
        let sequence = if index == 0 {
            number as u8 | LAST_ENTRY
        } else {
            number as u8
        };
        let start = (number - 1) * UNITS_PER_ENTRY;
        let mut units = [0u16; UNITS_PER_ENTRY];
        units.copy_from_slice(&self.units[start..start + UNITS_PER_ENTRY]);
        (sequence, units)
    }
}

/// Collects LFN entries read in disk order into a name.
///
/// A sequence that skips a number, changes checksum, has more than
/// [`MAX_ENTRIES`] entries or [`MAX_UNITS`] code units, or does not match
/// the short entry that follows it yields no name.
pub(crate) struct Assembler {
    units: [u16; MAX_UNITS],
    len: usize,
    checksum: u8,
    expected: u8,
    building: bool,
}

impl Default for Assembler {
    fn default() -> Self {
        Self::new()
    }
}

impl Assembler {
    pub(crate) const fn new() -> Self {
        Self {
            units: [0; MAX_UNITS],
            len: 0,
            checksum: 0,
            expected: 0,
            building: false,
        }
    }

    /// Drops any partial name, for example after a deleted or foreign entry.
    pub(crate) fn reset(&mut self) {
        self.len = 0;
        self.checksum = 0;
        self.expected = 0;
        self.building = false;
    }

    /// Feeds one LFN entry. An entry with [`LAST_ENTRY`] starts a new name.
    pub(crate) fn push(
        &mut self,
        sequence: u8,
        checksum: u8,
        name1: &[u8; 10],
        name2: &[u8; 12],
        name3: &[u8; 4],
    ) {
        if sequence & LAST_ENTRY != 0 {
            self.reset();
            let count = sequence & SEQUENCE_MASK;
            if count == 0 || count as usize > MAX_ENTRIES {
                return;
            }
            self.building = true;
            self.checksum = checksum;
            self.expected = count;
        }
        if !self.building {
            return;
        }
        let number = sequence & SEQUENCE_MASK;
        if number == 0 || number != self.expected || checksum != self.checksum {
            self.reset();
            return;
        }
        let (units, len) = unpack(name1, name2, name3);
        if self.len + len > MAX_UNITS {
            self.reset();
            return;
        }
        self.units.copy_within(0..self.len, len);
        self.units[..len].copy_from_slice(&units[..len]);
        self.len += len;
        self.expected -= 1;
    }

    /// Ends the name at a short entry whose name has checksum `short`, and
    /// returns its code units if the sequence was complete and matches.
    pub(crate) fn finish(&mut self, short: u8) -> Option<&[u16]> {
        let complete = self.building && self.expected == 0 && self.checksum == short;
        let len = self.len;
        self.reset();
        if complete {
            Some(&self.units[..len])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(assembler: &mut Assembler, encoded: &Encoded, sum: u8) {
        for index in 0..encoded.entries() {
            let (sequence, units) = encoded.entry(index);
            let (name1, name2, name3) = pack(&units);
            assembler.push(sequence, sum, &name1, &name2, &name3);
        }
    }

    fn assemble(name: &str) -> Option<std::vec::Vec<u16>> {
        let encoded = Encoded::new(name)?;
        let mut assembler = Assembler::new();
        feed(&mut assembler, &encoded, 0x5A);
        assembler.finish(0x5A).map(<[u16]>::to_vec)
    }

    #[test]
    fn checksum_matches_reference() {
        assert_eq!(checksum(b"FOO     BAR"), 0x53);
    }

    #[test]
    fn pack_and_unpack_round_trip() {
        let units: [u16; 13] = core::array::from_fn(|i| 0x41 + i as u16);
        let (name1, name2, name3) = pack(&units);
        assert_eq!(unpack(&name1, &name2, &name3), (units, 13));
        let mut short = units;
        short[4] = 0;
        short[5..].fill(0xFFFF);
        let (name1, name2, name3) = pack(&short);
        assert_eq!(unpack(&name1, &name2, &name3).1, 4);
    }

    #[test]
    fn encoded_orders_entries_last_first() {
        let encoded = Encoded::new("abcdefghijklmnop").unwrap();
        assert_eq!(encoded.entries(), 2);
        let (first, units) = encoded.entry(0);
        assert_eq!(first, 2 | LAST_ENTRY);
        assert_eq!(&units[..4], &[b'n' as u16, b'o' as u16, b'p' as u16, 0]);
        assert!(units[4..].iter().all(|&unit| unit == 0xFFFF));
        assert_eq!(encoded.entry(1).0, 1);
    }

    #[test]
    fn encoded_rejects_empty_and_overlong() {
        assert!(Encoded::new("").is_none());
        let max: std::string::String = core::iter::repeat_n('x', MAX_UNITS).collect();
        assert_eq!(Encoded::new(&max).unwrap().entries(), MAX_ENTRIES);
        let over: std::string::String = core::iter::repeat_n('x', MAX_UNITS + 1).collect();
        assert!(Encoded::new(&over).is_none());
        let exact = Encoded::new("abcdefghijklm").unwrap();
        assert_eq!(exact.entries(), 1);
        assert_eq!(exact.entry(0).1[12], b'm' as u16);
    }

    #[test]
    fn assembler_round_trips_names() {
        for name in [
            "a",
            "abcdefghijklm",
            "long file name.txt",
            "\u{1F600} smile",
        ] {
            let expected: std::vec::Vec<u16> = name.encode_utf16().collect();
            assert_eq!(assemble(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn assembler_rejects_overlong_runs() {
        let mut units = [0xFFFFu16; UNITS_PER_ENTRY];
        units[0] = b'a' as u16;
        units[1] = 0;
        let (name1, name2, name3) = pack(&units);
        let mut assembler = Assembler::new();
        for number in (1..=21u8).rev() {
            let sequence = if number == 21 {
                number | LAST_ENTRY
            } else {
                number
            };
            assembler.push(sequence, 7, &name1, &name2, &name3);
        }
        assert!(assembler.finish(7).is_none());

        let (name1, name2, name3) = pack(&[b'b' as u16; UNITS_PER_ENTRY]);
        for number in (1..=MAX_ENTRIES as u8).rev() {
            let sequence = if number == MAX_ENTRIES as u8 {
                number | LAST_ENTRY
            } else {
                number
            };
            assembler.push(sequence, 7, &name1, &name2, &name3);
        }
        assert!(assembler.finish(7).is_none());

        let max: std::string::String = core::iter::repeat_n('x', MAX_UNITS).collect();
        assert_eq!(assemble(&max).map(|units| units.len()), Some(MAX_UNITS));
    }

    #[test]
    fn assembler_rejects_broken_sequences() {
        let encoded = Encoded::new("abcdefghijklmnop").unwrap();
        let mut assembler = Assembler::new();
        feed(&mut assembler, &encoded, 1);
        assert!(assembler.finish(2).is_none());

        let (sequence, units) = encoded.entry(1);
        let (name1, name2, name3) = pack(&units);
        assembler.push(sequence, 1, &name1, &name2, &name3);
        assert!(assembler.finish(1).is_none());

        assembler.push(LAST_ENTRY, 1, &name1, &name2, &name3);
        assert!(assembler.finish(1).is_none());

        let (sequence, units) = encoded.entry(0);
        let (name1, name2, name3) = pack(&units);
        assembler.push(sequence, 1, &name1, &name2, &name3);
        assert!(assembler.finish(1).is_none());
    }
}
