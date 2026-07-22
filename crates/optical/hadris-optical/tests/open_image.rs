#![cfg(all(feature = "open", feature = "sync", feature = "cd"))]

use hadris_optical::{OpenPolicy, OpticalFormat, sync::OpenOpticalImage};

fn create_image(options: hadris_optical::cd::OpticalImageOptions) -> std::io::Cursor<Vec<u8>> {
    let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    hadris_optical::cd::OpticalImageWriter::new(
        hadris_io::sync::Borrowed::new(&mut image),
        options,
    )
    .finish(hadris_optical::cd::FileTree::new())
    .unwrap();
    image
}

fn create_udf_image() -> std::io::Cursor<Vec<u8>> {
    use hadris_optical::udf::sync::write::{SimpleDir, UdfWriteOptions, UdfWriter};

    let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    UdfWriter::create(
        hadris_io::sync::Borrowed::new(&mut image),
        &SimpleDir::root(),
        UdfWriteOptions::default(),
    )
    .unwrap();
    image
}

#[test]
fn opens_single_format_images_and_recovers_source() {
    let cases = [
        (
            create_image(hadris_optical::cd::OpticalImageOptions::default().iso_only()),
            OpticalFormat::Iso9660,
        ),
        (create_udf_image(), OpticalFormat::Udf),
    ];
    for (mut source, expected) in cases {
        let opened = OpenOpticalImage::open(&mut source, OpenPolicy::default()).unwrap();
        assert_eq!(opened.format(), expected);
        let source = opened.into_inner();
        assert!(!source.get_ref().is_empty());
    }
}

#[test]
fn exact_requests_are_checked() {
    let mut iso = create_image(hadris_optical::cd::OpticalImageOptions::default().iso_only());
    let error = match OpenOpticalImage::open(&mut iso, OpenPolicy::Udf) {
        Ok(_) => panic!("ISO-only image unexpectedly opened as UDF"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        hadris_optical::Error::RequestedFormatUnavailable(OpticalFormat::Udf)
    ));
}

#[test]
fn bridge_image_opens_as_either_filesystem() {
    let mut source = create_image(hadris_optical::cd::OpticalImageOptions::default());
    let opened = OpenOpticalImage::open(&mut source, OpenPolicy::Udf).unwrap();
    assert_eq!(opened.format(), OpticalFormat::Udf);
    let source = opened.into_inner();

    let opened = OpenOpticalImage::open(source, OpenPolicy::Iso9660).unwrap();
    assert_eq!(opened.format(), OpticalFormat::Iso9660);
    let _ = opened.into_inner();
}
