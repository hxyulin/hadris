pub static ESCAPE_SEQUNCES: [[u8; 3]; 3] = [*b"%/@", *b"%/C", *b"%/E"];

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JolietLevel {
    Level3 = 3,
}

impl JolietLevel {
    pub fn all() -> &'static [JolietLevel] {
        static LEVELS: [JolietLevel; 1] = [JolietLevel::Level3];
        &LEVELS
    }

    pub fn escape_sequence(self) -> [u8; 32] {
        let mut output = [0u8; 32];
        match self {
            Self::Level3 => output[0..3].copy_from_slice(b"%/E"),
        }
        output
    }
}


#[cfg(all(feature = "std", test))]
mod tests {
    use super::*;

    #[test]
    fn test_escape_sequences() {
        let level1 = b"%/@";
        assert_eq!(level1, &ESCAPE_SEQUNCES[0]);

        let level2 = b"%/C";
        assert_eq!(level2, &ESCAPE_SEQUNCES[1]);

        let level3 = b"%/E";
        assert_eq!(level3, &ESCAPE_SEQUNCES[2]);
    }
}
