#![cfg(all(feature = "open", feature = "async", feature = "sync", feature = "cd"))]

use hadris_fs::r#async::DriverExt;

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

struct ThreadWaker(std::thread::Thread);
impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

const PAYLOAD: &[u8] = b"async optical traversal";

fn populated_tree() -> hadris_fs::tree::Tree {
    use hadris_fs::tree::{Content, Tree};

    let mut tree = Tree::new();
    tree.add_file("DOCS/README.TXT", Content::bytes(PAYLOAD))
        .unwrap();
    tree.add_file("DOCS/R\u{e9}sum\u{e9}.txt", Content::bytes(PAYLOAD))
        .unwrap();
    tree
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
fn asynchronously_opens_and_recovers_an_iso_source() {
    let bytes = image_of(true, false, &populated_tree());

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Iso9660,
        )
        .await
        .unwrap();
        assert_eq!(opened.format(), hadris_optical::OpticalFormat::Iso9660);
        let mut opened = opened;
        let iso = opened.as_iso9660_mut().unwrap();
        let mut view = iso.view(hadris_optical::iso::Namespace::Preferred).unwrap();
        assert_eq!(
            view.read_to_vec("/DOCS//README.TXT").await.unwrap(),
            PAYLOAD
        );
        assert_eq!(view.read_to_vec("DOCS/Résumé.txt").await.unwrap(), PAYLOAD);
        assert!(!view.exists("DOCS/MISSING.TXT").await.unwrap());
        assert_eq!(
            view.read_to_vec("DOCS/README.TXT/CHILD")
                .await
                .unwrap_err()
                .kind(),
            hadris_fs::ErrorKind::NotADirectory
        );
        let root = view.root();
        let docs = view
            .lookup(root, hadris_fs::Name::new("DOCS").unwrap())
            .await
            .unwrap();
        assert!(view.node_metadata(docs).await.unwrap().file_type().is_dir());
        let _ = opened.into_inner();
    });
}

#[test]
fn asynchronously_opens_and_recovers_a_udf_source() {
    let bytes = image_of(false, true, &populated_tree());

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let mut opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Udf,
        )
        .await
        .unwrap();
        assert_eq!(opened.format(), hadris_optical::OpticalFormat::Udf);
        let udf = opened.as_udf_mut().unwrap();
        assert_eq!(udf.read_to_vec("/DOCS/README.TXT").await.unwrap(), PAYLOAD);
        let _ = opened.into_inner();
    });
}

#[test]
fn asynchronously_traverses_a_bridge_under_both_policies() {
    let bytes = image_of(true, true, &populated_tree());

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Udf,
        )
        .await
        .unwrap();
        let mut opened = opened;
        let udf = opened.as_udf_mut().unwrap();
        assert_eq!(udf.read_to_vec("/DOCS/README.TXT").await.unwrap(), PAYLOAD);
        drop(opened);

        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Iso9660,
        )
        .await
        .unwrap();
        let mut opened = opened;
        let iso = opened.as_iso9660_mut().unwrap();
        let mut view = iso.view(hadris_optical::iso::Namespace::Preferred).unwrap();
        assert_eq!(view.read_to_vec("/DOCS/README.TXT").await.unwrap(), PAYLOAD);
        let _ = opened.into_inner();
    });
}

#[test]
fn async_malformed_optical_inputs_use_category_errors() {
    use hadris_io::SeekFrom;
    use hadris_io::legacy::r#async::Seek;

    block_on(async {
        let unknown = [0xA5_u8; 4096];
        let mut source = hadris_io::Cursor::new(&unknown);
        source.seek(SeekFrom::Start(29)).await.unwrap();
        assert!(matches!(
            hadris_optical::r#async::OpenOpticalImage::open(
                &mut source,
                hadris_optical::OpenPolicy::default(),
            )
            .await,
            Err(hadris_optical::Error::UnknownFormat)
        ));
        assert_eq!(source.stream_position().await.unwrap(), 29);

        let mut corrupt_iso = vec![0_u8; 18 * 2048];
        corrupt_iso[16 * 2048] = 1;
        corrupt_iso[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
        corrupt_iso[16 * 2048 + 6] = 1;
        assert!(matches!(
            hadris_optical::r#async::OpenOpticalImage::open(
                &mut hadris_io::Cursor::new(corrupt_iso.as_slice()),
                hadris_optical::OpenPolicy::Iso9660,
            )
            .await,
            Err(hadris_optical::Error::Iso(_))
        ));
    });
}
