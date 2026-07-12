#![cfg(all(feature = "open", feature = "async", feature = "sync", feature = "cd"))]

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

fn create_cd_image(options: hadris_optical::cd::CdOptions) -> Vec<u8> {
    let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    hadris_optical::cd::CdWriter::new(hadris_io::sync::Borrowed::new(&mut image), options)
        .finish(populated_tree())
        .unwrap();
    image.into_inner()
}

fn iso_name(entry: &hadris_optical::iso::r#async::read::DirEntry) -> String {
    entry
        .display_name()
        .chars()
        .filter(|character| *character != '\0')
        .collect::<String>()
        .trim_end_matches(";1")
        .to_owned()
}

#[test]
fn asynchronously_opens_and_recovers_an_iso_source() {
    let bytes = create_cd_image(hadris_optical::cd::CdOptions::default().iso_only());

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        let opened = hadris_optical::r#async::OpenOpticalImage::open(
            &mut source,
            hadris_optical::OpenPolicy::Iso9660,
        )
        .await
        .unwrap();
        assert_eq!(opened.format(), hadris_optical::OpticalFormat::Iso9660);
        let iso = opened.as_iso9660().unwrap();
        let readme = iso.find_path("/DOCS//README.TXT").await.unwrap().unwrap();
        assert!(readme.is_file());
        assert_eq!(iso.read_file(&readme).await.unwrap(), PAYLOAD);
        let unicode = iso.find_path("DOCS/Résumé.txt").await.unwrap().unwrap();
        assert_eq!(iso.read_file(&unicode).await.unwrap(), PAYLOAD);
        assert!(iso.find_path("DOCS/MISSING.TXT").await.unwrap().is_none());
        assert!(
            iso.find_path("DOCS/README.TXT/CHILD")
                .await
                .unwrap()
                .is_none()
        );
        assert!(iso.find_path("../README.TXT").await.is_err());

        let root = iso.open_dir(iso.root_dir().dir_ref());
        let entries = root.read_entries().await.unwrap();
        let docs = entries
            .iter()
            .find(|entry| entry.is_directory() && iso_name(entry).eq_ignore_ascii_case("DOCS"))
            .unwrap();
        let nested = iso.open_dir(docs.as_dir_ref(iso).await.unwrap());
        let entries = nested.read_entries().await.unwrap();
        let readme = entries
            .iter()
            .find(|entry| entry.is_file() && iso_name(entry).eq_ignore_ascii_case("README.TXT"))
            .unwrap();
        assert_eq!(iso.read_file(readme).await.unwrap(), PAYLOAD);
        let _ = opened.into_inner();
    });
}

#[test]
fn asynchronously_opens_and_recovers_a_udf_source() {
    use hadris_optical::udf::sync::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

    let mut image = std::io::Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    let mut root = SimpleDir::root();
    let mut docs = SimpleDir::new("DOCS");
    docs.add_file(SimpleFile::new("README.TXT", PAYLOAD.to_vec()));
    root.add_dir(docs);
    UdfWriter::create(
        hadris_io::sync::Borrowed::new(&mut image),
        &root,
        UdfWriteOptions::default(),
    )
    .unwrap();
    let bytes = image.into_inner();

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
    let bytes = create_cd_image(hadris_optical::cd::CdOptions::default());

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
        let iso = opened.as_iso9660().unwrap();
        let root = iso.open_dir(iso.root_dir().dir_ref());
        let entries = root.read_entries().await.unwrap();
        assert!(
            entries.iter().any(|entry| {
                entry.is_directory() && iso_name(entry).eq_ignore_ascii_case("DOCS")
            })
        );
        let _ = opened.into_inner();
    });
}

#[test]
fn async_malformed_optical_inputs_use_category_errors() {
    use hadris_io::SeekFrom;
    use hadris_io::r#async::Seek;

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
