/// One of the directory trees an ISO image can carry.
///
/// An image always has the primary tree. Rock Ridge adds POSIX names and
/// metadata to it, Joliet adds a second tree with UCS-2 names, and ISO
/// 9660:1999 adds an enhanced tree with long names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Namespace {
    /// The most capable tree the image has: Rock Ridge, then Joliet, then
    /// the enhanced tree, then the primary tree.
    Preferred,
    /// The primary tree with ISO 9660 names.
    Primary,
    /// The primary tree with Rock Ridge names and metadata.
    RockRidge,
    /// The Joliet tree.
    Joliet,
    /// The ISO 9660:1999 enhanced tree.
    Enhanced,
}

/// The UCS-2 level of a Joliet tree, from its escape sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum JolietLevel {
    /// UCS-2 level 1, `%/@`.
    L1,
    /// UCS-2 level 2, `%/C`.
    L2,
    /// UCS-2 level 3, `%/E`, which most producers write.
    L3,
}

impl JolietLevel {
    /// The level for the escape sequence field, if it names one.
    pub fn from_escape_sequences(escapes: &[u8; 32]) -> Option<Self> {
        match &escapes[..3] {
            b"%/@" => Some(Self::L1),
            b"%/C" => Some(Self::L2),
            b"%/E" => Some(Self::L3),
            _ => None,
        }
    }

    /// The escape sequence field for this level, with the unused bytes set
    /// to zero as ECMA-119 8.5.6 requires.
    pub fn escape_sequences(self) -> [u8; 32] {
        let mut out = [0; 32];
        let index = match self {
            Self::L1 => 0,
            Self::L2 => 1,
            Self::L3 => 2,
        };
        out[..3].copy_from_slice(&crate::raw::JOLIET_ESCAPES[index]);
        out
    }
}

/// The trees an image has, as `IsoFs::namespaces` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Namespaces {
    rock_ridge: bool,
    joliet: Option<JolietLevel>,
    enhanced: bool,
}

impl Namespaces {
    pub(crate) const fn new(rock_ridge: bool, joliet: Option<JolietLevel>, enhanced: bool) -> Self {
        Self {
            rock_ridge,
            joliet,
            enhanced,
        }
    }

    /// Whether the image has `namespace`. [`Namespace::Preferred`] and
    /// [`Namespace::Primary`] always exist.
    pub const fn contains(&self, namespace: Namespace) -> bool {
        match namespace {
            Namespace::Preferred | Namespace::Primary => true,
            Namespace::RockRidge => self.rock_ridge,
            Namespace::Joliet => self.joliet.is_some(),
            Namespace::Enhanced => self.enhanced,
        }
    }

    /// The level of the Joliet tree, if there is one.
    pub const fn joliet_level(&self) -> Option<JolietLevel> {
        self.joliet
    }

    /// The tree [`Namespace::Preferred`] picks.
    pub const fn preferred(&self) -> Namespace {
        if self.rock_ridge {
            Namespace::RockRidge
        } else if self.joliet.is_some() {
            Namespace::Joliet
        } else if self.enhanced {
            Namespace::Enhanced
        } else {
            Namespace::Primary
        }
    }

    /// Every tree the image has, most capable first.
    pub fn iter(&self) -> impl Iterator<Item = Namespace> + '_ {
        [
            Namespace::RockRidge,
            Namespace::Joliet,
            Namespace::Enhanced,
            Namespace::Primary,
        ]
        .into_iter()
        .filter(|namespace| self.contains(*namespace))
    }
}
