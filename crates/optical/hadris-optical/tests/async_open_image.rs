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

fn populated_tree() -> hadris_optical::cd::FileTree {
    use hadris_optical::cd::{Directory, FileEntry, FileTree};

    let mut nested = Directory::new("DOCS");
    nested.add_file(FileEntry::from_buffer("README.TXT", PAYLOAD.to_vec()));
    nested.add_file(FileEntry::from_buffer("Résumé.txt", PAYLOAD.to_vec()));
    let mut tree = FileTree::new();
    tree.add_dir(nested);
    tree
}

fn create_cd_image(options: hadris_optical::cd::OpticalImageOptions) -> Vec<u8> {
    let mut image = hadris_io::StdIo::new(std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]));
    hadris_optical::cd::OpticalImageWriter::new(&mut image, options)
        .finish(populated_tree())
        .unwrap();
    image.into_inner().into_inner()
}

#[test]
fn asynchronously_opens_and_recovers_an_iso_source() {
    let bytes = create_cd_image(hadris_optical::cd::OpticalImageOptions::default().iso_only());

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
    use hadris_optical::udf::sync::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

    let mut image = hadris_io::StdIo::new(std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]));
    let mut root = SimpleDir::root();
    let mut docs = SimpleDir::new("DOCS");
    docs.add_file(SimpleFile::new("README.TXT", PAYLOAD.to_vec()));
    root.add_dir(docs);
    UdfWriter::create(&mut image, &root, UdfWriteOptions::default()).unwrap();
    let bytes = image.into_inner().into_inner();

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Udf,
        )
        .await
        .unwrap();
        assert_eq!(opened.format(), hadris_optical::OpticalFormat::Udf);
        let udf = opened.as_udf().unwrap();
        let root = udf.root_dir().await.unwrap();
        let docs = root.find("DOCS").unwrap();
        let nested = udf.read_directory(&docs.icb).await.unwrap();
        let readme = nested.find("README.TXT").unwrap();
        assert_eq!(udf.read_file(readme).await.unwrap(), PAYLOAD);
        let _ = opened.into_inner();
    });
}

#[test]
fn asynchronously_traverses_a_bridge_under_both_policies() {
    let bytes = create_cd_image(hadris_optical::cd::OpticalImageOptions::default());

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Udf,
        )
        .await
        .unwrap();
        let udf = opened.as_udf().unwrap();
        let root = udf.root_dir().await.unwrap();
        let docs = root.find("DOCS").unwrap();
        let nested = udf.read_directory(&docs.icb).await.unwrap();
        assert_eq!(
            udf.read_file(nested.find("README.TXT").unwrap())
                .await
                .unwrap(),
            PAYLOAD
        );
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
