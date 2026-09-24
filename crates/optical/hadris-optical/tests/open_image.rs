//! Opening ISO 9660, UDF and bridge images through `OpenOpticalImage`.

mod common;

use common::{PAYLOAD, device, image_of, populated_tree};
use hadris_fs::ErrorKind;
use hadris_fs::sync::DriverExt;
use hadris_optical::sync::OpenOpticalImage;
use hadris_optical::{Detail, OpenPolicy, OpticalFormat};

#[test]
fn opens_single_format_images_and_gives_the_device_back() {
    for (iso, udf, expected) in [
        (true, false, OpticalFormat::Iso9660),
        (false, true, OpticalFormat::Udf),
    ] {
        let bytes = image_of(iso, udf, &populated_tree());
        let len = bytes.len();
        let mut opened =
            OpenOpticalImage::open(device(bytes, 2048), OpenPolicy::default()).unwrap();
        assert_eq!(opened.format(), expected);
        assert_eq!(opened.as_iso().is_some(), iso);
        assert_eq!(opened.as_udf().is_some(), udf);
        assert_eq!(opened.read_to_vec("/DOCS/README.TXT").unwrap(), PAYLOAD);
        assert!(!opened.capabilities().is_writable());
        hadris_fs::sync::contract::check_read_only(&mut opened).unwrap();
        assert_eq!(opened.into_inner().into_inner().len(), len);
    }
}

#[test]
fn bridge_images_open_as_either_filesystem() {
    let bytes = image_of(true, true, &populated_tree());
    let mut opened = OpenOpticalImage::open(device(bytes, 2048), OpenPolicy::PreferUdf).unwrap();
    assert_eq!(opened.format(), OpticalFormat::Udf);
    assert_eq!(
        opened.read_to_vec("/DOCS/R\u{e9}sum\u{e9}.txt").unwrap(),
        PAYLOAD
    );
    let udf = opened.into_udf().map_err(|_| ()).unwrap();

    let mut opened = OpenOpticalImage::open(udf.into_inner(), OpenPolicy::PreferIso9660).unwrap();
    assert_eq!(opened.format(), OpticalFormat::Iso9660);
    assert_eq!(opened.read_to_vec("/DOCS/README.TXT").unwrap(), PAYLOAD);
    let view = opened.into_iso().map_err(|_| ()).unwrap();
    assert!(!view.into_inner().into_inner().is_empty());
}

#[test]
fn exact_requests_are_checked_and_give_the_device_back() {
    let bytes = image_of(true, false, &populated_tree());
    let len = bytes.len();
    let err = OpenOpticalImage::open(device(bytes, 2048), OpenPolicy::Udf)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(
        err.error().detail(),
        Some(Detail::FormatUnavailable(OpticalFormat::Udf))
    );
    assert_eq!(err.into_device().into_inner().len(), len);
}

#[test]
fn malformed_images_use_category_errors() {
    let err = OpenOpticalImage::open(device(vec![0xA5; 64 * 2048], 2048), OpenPolicy::default())
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.error().detail(), Some(Detail::UnknownFormat));

    let mut corrupt = vec![0u8; 18 * 2048];
    corrupt[16 * 2048] = 1;
    corrupt[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
    corrupt[16 * 2048 + 6] = 1;
    let (error, dev) = OpenOpticalImage::open(device(corrupt.clone(), 2048), OpenPolicy::Iso9660)
        .map(|_| ())
        .unwrap_err()
        .into_parts();
    assert_eq!(error.detail(), Some(Detail::Mount(OpticalFormat::Iso9660)));
    assert_eq!(error.kind(), ErrorKind::Corrupt);
    assert_eq!(dev.into_inner(), corrupt);
    let io: std::io::Error = error.into();
    assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn smaller_device_blocks_open_too() {
    let bytes = image_of(true, true, &populated_tree());
    for policy in [OpenPolicy::Udf, OpenPolicy::Iso9660] {
        let mut opened = OpenOpticalImage::open(device(bytes.clone(), 512), policy).unwrap();
        assert_eq!(opened.read_to_vec("/DOCS/README.TXT").unwrap(), PAYLOAD);
    }
}
