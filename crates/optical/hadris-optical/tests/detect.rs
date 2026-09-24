//! Detection of ISO 9660, UDF and bridge images in every mode and on
//! every block size.

mod common;

use common::{SECTOR, block_on, device, image_of, image_with};
use hadris_optical::detect::{UdfVrs, sync::detect};

fn cases() -> [(Vec<u8>, bool, Option<UdfVrs>); 4] {
    [
        (image_with(&[(16, b"CD001")]), true, None),
        (
            image_with(&[(16, b"BEA01"), (17, b"NSR02"), (18, b"TEA01")]),
            false,
            Some(UdfVrs::Nsr02),
        ),
        (
            image_with(&[
                (16, b"CD001"),
                (17, b"BEA01"),
                (18, b"NSR03"),
                (19, b"TEA01"),
            ]),
            true,
            Some(UdfVrs::Nsr03),
        ),
        (
            image_with(&[(16, b"BEA01"), (18, b"NSR03"), (20, b"TEA01")]),
            false,
            Some(UdfVrs::Nsr03),
        ),
    ]
}

#[test]
fn every_block_size_distinguishes_iso_udf_and_bridge() {
    for block in [512, 1024, 2048, 4096] {
        for (image, iso, udf) in cases() {
            let formats = detect(&mut device(image, block)).unwrap().unwrap();
            assert_eq!(formats.has_iso9660(), iso, "{block}");
            assert_eq!(formats.udf(), udf, "{block}");
            assert_eq!(formats.is_bridge(), iso && udf.is_some());
        }
    }
}

#[test]
fn images_from_the_writers_are_detected() {
    for (iso, udf) in [(true, false), (false, true), (true, true)] {
        let bytes = image_of(iso, udf, &hadris_fs::tree::Tree::new());
        let formats = detect(&mut device(bytes, 2048)).unwrap().unwrap();
        assert_eq!(formats.has_iso9660(), iso);
        assert_eq!(formats.udf().is_some(), udf);
    }
}

#[test]
fn small_blank_and_huge_block_devices_are_not_optical() {
    assert_eq!(
        detect(&mut device(vec![0u8; 20 * SECTOR], 2048)).unwrap(),
        None
    );
    assert_eq!(
        detect(&mut device(vec![0u8; 8 * SECTOR], 512)).unwrap(),
        None
    );
    let image = image_with(&[(16, b"CD001")]);
    assert_eq!(detect(&mut device(image, 8192)).unwrap(), None);
    let image = image_with(&[(16, b"BEA01"), (17, b"NSR02")]);
    assert_eq!(detect(&mut device(image, 2048)).unwrap(), None);
}

#[test]
fn async_modes_detect_bridges() {
    let image = image_with(&[
        (16, b"CD001"),
        (17, b"BEA01"),
        (18, b"NSR03"),
        (19, b"TEA01"),
    ]);
    block_on(async {
        let formats = hadris_optical::detect::r#async::detect(&mut device(image.clone(), 512))
            .await
            .unwrap()
            .unwrap();
        assert!(formats.is_bridge());
        let formats = hadris_optical::detect::r#async::detect(&mut device(image, 4096))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(formats.udf(), Some(UdfVrs::Nsr03));
    });
}
