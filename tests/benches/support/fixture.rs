use std::collections::BTreeMap;

use hadris_fs::{Content, Node, Tree};
use hadris_tests::harness::EntryData;

pub const PAYLOAD: &str = "PAYLOAD.BIN";

pub struct Fixture {
    #[allow(dead_code)]
    pub tree: Tree,
    #[allow(dead_code)]
    pub payload: Vec<u8>,
    pub entries: BTreeMap<String, EntryData>,
}

impl Fixture {
    pub fn new() -> Self {
        let payload: Vec<u8> = (0..131072).map(|i| (i * 17 + i / 251) as u8).collect();
        let mut entries = BTreeMap::new();
        let files = std::env::var("HADRIS_TESTS_PERF_FILES")
            .map_or(32, |text| text.parse::<usize>().unwrap());
        assert!(files > 0 && files <= 100_000);
        for index in 0..files {
            entries.insert(
                format!("/F{index:07}.TXT"),
                EntryData::File(b"fixture".to_vec()),
            );
        }
        entries.insert(format!("/{PAYLOAD}"), EntryData::File(payload.clone()));
        let mut tree = Tree::new();
        for (path, entry) in &entries {
            if let EntryData::File(bytes) = entry {
                tree.insert(path, Node::file(Content::bytes(bytes.clone())))
                    .unwrap();
            }
        }
        Self {
            tree,
            payload,
            entries,
        }
    }
}
