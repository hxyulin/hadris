#![cfg(all(feature = "open", feature = "sync", feature = "cd"))]

use hadris_io::StdIo;
use hadris_optical::{OpenPolicy, OpticalFormat, sync::OpenOpticalImage};

fn create_image(iso: bool, udf: bool) -> StdIo<std::io::Cursor<Vec<u8>>> {
    StdIo::new(std::io::Cursor::new(image_of(
        iso,
        udf,
        &hadris_fs::tree::Tree::new(),
    )))
}

/// An empty image with the ISO 9660 tree, the UDF volume, or both.
fn image_of(iso: bool, udf: bool, tree: &hadris_fs::tree::Tree) -> Vec<u8> {
    use hadris_storage::{BlockSize, MemDevice};
    let block = BlockSize::new(2048).unwrap();
    let size = 4 * 1024 * 1024;
    let mut dev = MemDevice::new(vec![0_u8; size], block);
    match (iso, udf) {
        (true, false) => {
            hadris_optical::iso::sync::write(
                &mut dev,
                tree,
                hadris_optical::cd::CdOptions::default().iso(),
            )
            .unwrap();
        }
        (false, true) => {
            hadris_optical::udf::sync::write(
                &mut dev,
                tree,
                &hadris_optical::udf::UdfOptions::default(),
            )
            .unwrap();
        }
        _ => {
            hadris_optical::cd::sync::write(
                &mut dev,
                tree,
                &hadris_optical::cd::CdOptions::default(),
            )
            .unwrap();
        }
    }
    dev.into_inner()
}

#[test]
fn opens_single_format_images_and_recovers_source() {
    let cases = [
        (create_image(true, false), OpticalFormat::Iso9660),
        (create_image(false, true), OpticalFormat::Udf),
    ];
    for (mut source, expected) in cases {
        let opened = OpenOpticalImage::open(&mut source, OpenPolicy::default()).unwrap();
        assert_eq!(opened.format(), expected);
        let source = opened.into_inner();
        assert!(!source.get_ref().get_ref().is_empty());
    }
}

#[test]
fn exact_requests_are_checked() {
    let mut iso = create_image(true, false);
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
    let mut source = create_image(true, true);
    let opened = OpenOpticalImage::open(&mut source, OpenPolicy::Udf).unwrap();
    assert_eq!(opened.format(), OpticalFormat::Udf);
    let source = opened.into_inner();

    let opened = OpenOpticalImage::open(source, OpenPolicy::Iso9660).unwrap();
    assert_eq!(opened.format(), OpticalFormat::Iso9660);
    let _ = opened.into_inner();
}
