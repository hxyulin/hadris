mod common;

use hadris_fs::{Content, Node, Tree};
use hadris_iso::{IsoOptions, Namespace};
use hadris_storage::MemDevice;

fn versioned(namespace: Namespace, order: [usize; 3], split: bool) -> MemDevice<Vec<u8>> {
    let mut tree = Tree::new();
    for version in 1..=3 {
        tree.insert(
            format!("V{version}.TXT"),
            Node::file(Content::bytes([b'0' + version as u8])),
        )
        .unwrap();
    }
    let mut image = common::image(&tree, &IsoOptions::new().with_joliet().with_iso1999());
    let start = {
        use hadris_fs::sync::FileSystem;
        let fs = hadris_iso::sync::IsoFs::mount_namespace(
            &mut image,
            hadris_fs::MountOptions::new(),
            namespace,
        )
        .unwrap();
        fs.root().get() as usize
    };
    let mut records = Vec::new();
    let mut at = start;
    while image.get_ref()[at] != 0 {
        let len = image.get_ref()[at] as usize;
        records.push(image.get_ref()[at..at + len].to_vec());
        at += len;
    }
    assert_eq!(records.len(), 5);
    let mut selected = records[..2].to_vec();
    for version in order {
        let mut record = records[version + 1].clone();
        let name = format!("A0.TXT;{version}");
        let bytes = if namespace == Namespace::Joliet {
            name.encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>()
        } else {
            name.into_bytes()
        };
        let mut encoded = hadris_iso::raw::DirectoryRecord::new(&bytes, &[])
            .unwrap()
            .as_bytes()
            .to_vec();
        let len = encoded.len();
        encoded[..33].copy_from_slice(&record[..33]);
        encoded[0] = len as u8;
        encoded[32] = bytes.len() as u8;
        record = encoded;
        if split && version == 3 {
            let mut first = record.clone();
            first[25] |= 0x80;
            selected.push(first);
        }
        selected.push(record);
    }
    image.get_mut()[start..start + 2048].fill(0);
    at = start;
    for record in selected {
        image.get_mut()[at..at + record.len()].copy_from_slice(&record);
        at += record.len();
    }
    image
}

macro_rules! cases {
    ($mode:ident, $test:ident, $run:ident) => {
        #[test]
        fn $test() {
            $run!(async {
                #[allow(unused_imports)]
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{DirCursor, ErrorKind, MountOptions, Name, OpenMode};
                for namespace in [Namespace::Primary, Namespace::Joliet] {
                    for order in [[3, 2, 1], [1, 2, 3], [2, 1, 3]] {
                        for split in [false, true] {
                            let image = versioned(namespace, order, split);
                            let mut fs = hadris_iso::$mode::IsoFs::mount_namespace(
                                image,
                                MountOptions::new(),
                                namespace,
                            )
                            .await
                            .unwrap();
                            let root = fs.root();
                            for (query, version) in [
                                ("A0.TXT", 3),
                                ("A0.TXT;1", 1),
                                ("A0.TXT;2", 2),
                                ("A0.TXT;3", 3),
                            ] {
                                let node = fs.lookup(root, Name::new(query)).await.unwrap();
                                fs.open(node, OpenMode::Read).await.unwrap();
                                let mut data = [0; 2];
                                let expected = if split && version == 3 {
                                    vec![b'3'; 2]
                                } else {
                                    vec![b'0' + version]
                                };
                                let mut len = 0;
                                while len < expected.len() {
                                    let n = fs
                                        .read(node, len as u64, &mut data[len..expected.len()])
                                        .await
                                        .unwrap();
                                    assert!(n > 0);
                                    len += n;
                                }
                                assert_eq!(
                                    fs.stat(node).await.unwrap().len(),
                                    expected.len() as u64
                                );
                                assert_eq!(
                                    &data[..len],
                                    expected,
                                    "{namespace:?} {order:?} {query}"
                                );
                                fs.close(node).await.unwrap();
                            }
                            assert_eq!(
                                fs.lookup(root, Name::new("A0.TXT;4"))
                                    .await
                                    .unwrap_err()
                                    .kind(),
                                ErrorKind::NotFound
                            );
                            let entry = fs.readdir(root, DirCursor::START).await.unwrap().unwrap();
                            assert_eq!(entry.name().as_bytes(), b"A0.TXT");
                            assert_eq!(entry.metadata().len(), if split { 2 } else { 1 });
                            assert_eq!(
                                entry.node(),
                                fs.lookup(root, Name::new("A0.TXT")).await.unwrap()
                            );
                            assert!(
                                fs.readdir(root, entry.next_cursor())
                                    .await
                                    .unwrap()
                                    .is_none()
                            );
                        }
                    }
                }
            });
        }
    };
}
macro_rules! sync_case { ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) }; }
#[cfg(feature = "async")]
macro_rules! async_case {
    ($body:expr) => {
        common::block_on($body)
    };
}
cases!(sync, versions_select_and_list_the_highest_sync, sync_case);
#[cfg(feature = "async")]
cases!(
    r#async,
    versions_select_and_list_the_highest_async,
    async_case
);

#[test]
fn rock_ridge_names_with_semicolons_are_literal() {
    use common::Paths;
    let mut tree = Tree::new();
    tree.insert("notes;2", Node::file(Content::bytes("literal")))
        .unwrap();
    let image = common::image(&tree, &IsoOptions::new().with_rock_ridge());
    let mut fs = hadris_iso::sync::IsoFs::mount(image, hadris_fs::MountOptions::new()).unwrap();
    assert_eq!(fs.read_to_vec("/notes;2").unwrap(), b"literal");
    assert!(!fs.exists("/notes").unwrap());
}
