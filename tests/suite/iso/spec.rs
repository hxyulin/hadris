//! Hadris ISO writer and reader against the raw ECMA-119 oracle.

use hadris_tests::iso::{SECTOR_SIZE, conformance_scenarios, hadris, spec};

#[test]
fn hadris_iso_matches_ecma_119_oracle() {
    for (scenario, expected) in conformance_scenarios() {
        let bytes = hadris::write(&expected).unwrap();
        hadris::verify_image(&format!("{scenario} Hadris writer"), bytes, &expected).unwrap();
    }
}

#[test]
fn oracle_rejects_structural_corruption() {
    type Corrupt = fn(&mut [u8]);

    let expected = conformance_scenarios().remove(0).1;
    let image = hadris::write(&expected).unwrap();
    let cases: [(&str, Corrupt); 3] = [
        ("volume endian mismatch", |bytes| {
            bytes[16 * SECTOR_SIZE + 84] ^= 1
        }),
        ("path table endian mismatch", |bytes| {
            bytes[16 * SECTOR_SIZE + 148] ^= 1
        }),
        ("invalid root record", |bytes| {
            bytes[16 * SECTOR_SIZE + 156 + 25] = 0
        }),
    ];
    for (name, corrupt) in cases {
        let mut damaged = image.clone();
        corrupt(&mut damaged);
        assert!(spec::snapshot(&damaged).is_err(), "oracle accepted {name}");
    }
}
