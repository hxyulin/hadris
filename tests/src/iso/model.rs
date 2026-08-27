use std::collections::BTreeMap;
use std::path::Path;

use super::VOLUME_ID;
use crate::harness::tree::{self, EntryData};

/// The semantic content of a write-once ISO 9660 volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoState {
    pub volume_id: String,
    pub entries: BTreeMap<String, EntryData>,
}

impl IsoState {
    /// Reads an extracted or mounted image from the host filesystem. Host
    /// trees carry no volume identifier, so compare them with
    /// [`compare_entries`].
    pub fn from_host(root: &Path) -> Result<Self, String> {
        Ok(Self {
            volume_id: String::new(),
            entries: tree::snapshot_host(root)?,
        })
    }

    /// Materializes the tree as a source directory for external producers.
    pub fn write_host(&self, root: &Path) -> Result<(), String> {
        tree::write_host_tree(root, &self.entries)
    }
}

pub fn compare_state(label: &str, expected: &IsoState, actual: &IsoState) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let mut differences = Vec::new();
    if expected.volume_id != actual.volume_id {
        differences.push(format!(
            "volume ID: expected {:?}, actual {:?}",
            expected.volume_id, actual.volume_id
        ));
    }
    differences.extend(tree::differences(
        &expected.entries,
        &actual.entries,
        EntryData::summary,
    ));
    Err(format!(
        "{label} semantic mismatch:\n{}",
        differences.join("\n")
    ))
}

pub fn compare_entries(label: &str, expected: &IsoState, actual: &IsoState) -> Result<(), String> {
    let differences = tree::differences(&expected.entries, &actual.entries, EntryData::summary);
    if differences.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{label} semantic mismatch:\n{}",
            differences.join("\n")
        ))
    }
}

/// Removes a trailing `;N` file version from one identifier.
pub fn strip_version(name: &str) -> &str {
    match name.rsplit_once(';') {
        Some((base, version))
            if !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            base
        }
        _ => name,
    }
}

/// Normalizes readers that expose file versions in their path names.
pub fn strip_path_versions(state: IsoState) -> IsoState {
    let entries = state
        .entries
        .into_iter()
        .map(|(path, data)| {
            let path = path
                .split('/')
                .map(strip_version)
                .collect::<Vec<_>>()
                .join("/");
            (path, data)
        })
        .collect();
    IsoState { entries, ..state }
}

/// The Level 1 scenarios every producer and consumer is measured against.
pub fn conformance_scenarios() -> Vec<(&'static str, IsoState)> {
    let basic = IsoState {
        volume_id: VOLUME_ID.to_string(),
        entries: BTreeMap::from([
            ("/EMPTY.BIN".to_string(), EntryData::File(Vec::new())),
            (
                "/README.TXT".to_string(),
                EntryData::File(b"Hadris ISO conformance\n".to_vec()),
            ),
        ]),
    };
    let mut entries = BTreeMap::from([
        ("/NESTED".to_string(), EntryData::Directory),
        (
            "/NESTED/DATA.BIN".to_string(),
            EntryData::File((0..8193).map(|value| (value % 251) as u8).collect()),
        ),
        ("/MANY".to_string(), EntryData::Directory),
    ]);
    for index in 0..48 {
        entries.insert(
            format!("/MANY/F{index:02}.TXT"),
            EntryData::File(format!("record {index:02}\n").into_bytes()),
        );
    }
    vec![
        ("basic", basic),
        (
            "nested-multisector",
            IsoState {
                volume_id: VOLUME_ID.to_string(),
                entries,
            },
        ),
    ]
}
