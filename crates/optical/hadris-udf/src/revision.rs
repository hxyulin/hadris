use core::fmt;

/// A UDF revision, as the domain identifier records it: `0x0102` for 1.02.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UdfRevision(u16);

impl UdfRevision {
    /// UDF 1.02: DVD-ROM.
    pub const V1_02: Self = Self(0x0102);
    /// UDF 1.50: packet writing and DVD-RAM.
    pub const V1_50: Self = Self(0x0150);
    /// UDF 2.00.
    pub const V2_00: Self = Self(0x0200);
    /// UDF 2.01: DVD-RW and DVD+RW.
    pub const V2_01: Self = Self(0x0201);
    /// UDF 2.50: Blu-ray. The writer refuses it: it needs a metadata
    /// partition.
    pub const V2_50: Self = Self(0x0250);
    /// UDF 2.60: Blu-ray pseudo-overwrite. The writer refuses it: it needs
    /// a metadata partition.
    pub const V2_60: Self = Self(0x0260);

    /// A revision from its binary-coded value.
    pub const fn from_raw(value: u16) -> Self {
        Self(value)
    }

    /// The binary-coded value.
    pub const fn to_raw(self) -> u16 {
        self.0
    }

    /// The major version.
    pub const fn major(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// The minor version, binary-coded: `0x02` for 1.02.
    pub const fn minor(self) -> u8 {
        self.0 as u8
    }

    /// Whether volumes of this revision use ECMA-167 3rd edition
    /// structures: NSR03 and descriptor version 3.
    pub const fn is_nsr03(self) -> bool {
        self.0 >= 0x0200
    }
}

impl fmt::Display for UdfRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02x}", self.major(), self.minor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_print_as_decimal_versions() {
        assert_eq!(UdfRevision::V2_01.major(), 2);
        assert_eq!(UdfRevision::V2_01.minor(), 1);
        assert_eq!(format!("{}", UdfRevision::V1_02), "1.02");
        assert_eq!(format!("{}", UdfRevision::V2_50), "2.50");
        assert!(UdfRevision::V2_00.is_nsr03() && !UdfRevision::V1_50.is_nsr03());
    }
}
