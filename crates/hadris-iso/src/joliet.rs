pub static ESCAPE_SEQUNCES: [[u8; 3]; 3] = [*b"%/@", *b"%/C", *b"%/E"];

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
