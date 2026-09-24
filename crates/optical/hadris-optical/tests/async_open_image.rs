//! `OpenOpticalImage` in the asynchronous modes.

mod common;

use common::{PAYLOAD, block_on, device, image_of, populated_tree};
use hadris_fs::ErrorKind;
use hadris_optical::{OpenPolicy, OpticalFormat};

#[test]
fn async_mode_opens_each_filesystem_of_a_bridge() {
    use common::asynch::get;
    use hadris_optical::r#async::OpenOpticalImage;

    let bytes = image_of(true, true, &populated_tree());
    block_on(async {
        let mut opened = OpenOpticalImage::open(device(bytes, 2048), OpenPolicy::Udf)
            .await
            .unwrap();
        assert_eq!(opened.format(), OpticalFormat::Udf);
        assert_eq!(get(&mut opened, "/DOCS/README.TXT").await.unwrap(), PAYLOAD);
        hadris_fs::r#async::contract::check_read_only(&mut opened)
            .await
            .unwrap();

        let dev = opened.into_inner();
        let mut opened = OpenOpticalImage::open(dev, OpenPolicy::Iso9660)
            .await
            .unwrap();
        assert_eq!(opened.format(), OpticalFormat::Iso9660);
        assert_eq!(
            get(&mut opened, "DOCS/MISSING.TXT")
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            get(&mut opened, "DOCS/README.TXT/CHILD")
                .await
                .unwrap_err()
                .kind(),
            hadris_fs::ErrorKind::NotADirectory
        );
        hadris_fs::r#async::contract::check_read_only(&mut opened)
            .await
            .unwrap();
    });
}

#[test]
fn async_mode_shares_an_opened_image() {
    use common::asynch::get;
    use hadris_fs::r#async::Volume;
    use hadris_optical::r#async::OpenOpticalImage;

    let bytes = image_of(true, false, &populated_tree());
    block_on(async {
        let opened = OpenOpticalImage::open(device(bytes, 2048), OpenPolicy::default())
            .await
            .unwrap();
        let vol = Volume::new(opened);
        assert_eq!(
            get(&mut *vol.lock().await, "/DOCS/README.TXT")
                .await
                .unwrap(),
            PAYLOAD
        );
        hadris_fs::r#async::contract::check_read_only(&mut *vol.lock().await)
            .await
            .unwrap();

        let err = OpenOpticalImage::open(device(vec![0u8; 64 * 2048], 2048), OpenPolicy::default())
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotRecognized);
        assert_eq!(err.error().detail(), None);
    });
}
