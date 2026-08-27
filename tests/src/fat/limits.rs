//! Exercises at the edges of a volume: fixed root directories, data-region
//! exhaustion, and extents long enough to span many FAT sectors.
//!
//! Every exercise drives an adapter through the shared operation model and
//! checks the image through a caller-supplied oracle after each phase, so it
//! scores Hadris and the peers identically.

use super::adapter::{FatAdapter, clear_mutable_attrs};
use super::model::{FsState, Operation, compare_snapshot, summarize_operation};
use super::scenarios::payload;
use super::spec::ImageGeometry;
use crate::harness::tree::EntryData;

/// Reads the image under test through the specification oracle.
pub type Oracle<'a> = dyn FnMut() -> Result<FsState, String> + 'a;

/// How an exercise compares the model against the oracle.
#[derive(Clone, Copy)]
pub struct Checks<'a> {
    /// Peers that cannot set attributes are compared with attributes cleared.
    pub ignore_attrs: bool,
    pub geometry: ImageGeometry,
    pub free_clusters: &'a dyn Fn() -> Result<u32, String>,
}

impl Checks<'_> {
    fn compare(&self, context: &str, expected: &FsState, actual: FsState) -> Result<(), String> {
        let mut expected = expected.clone();
        let mut actual = actual;
        if self.ignore_attrs {
            clear_mutable_attrs(&mut expected);
            clear_mutable_attrs(&mut actual);
        }
        compare_snapshot(context, &expected, &actual)
    }
}

fn create(path: String, data: Vec<u8>) -> Operation {
    Operation::CreateFile { path, data }
}

fn step(
    adapter: &mut dyn FatAdapter,
    expected: &mut FsState,
    operation: Operation,
) -> Result<(), String> {
    adapter
        .apply(&operation)
        .map_err(|error| format!("{} failed: {error}", summarize_operation(&operation)))?;
    expected.apply(&operation)
}

fn expect_rejection(
    adapter: &mut dyn FatAdapter,
    operation: Operation,
    context: &str,
) -> Result<(), String> {
    match adapter.apply(&operation) {
        Ok(()) => Err(format!(
            "{context}: accepted {}",
            summarize_operation(&operation)
        )),
        Err(_) => Ok(()),
    }
}

/// Fills a fixed FAT12/16 root directory to the last slot with 8.3 names,
/// then checks that overflow, a long name that does not fit the single free
/// slot, and a final 8.3 name are handled without touching other entries.
/// FAT32 roots grow like any directory and are covered by the scenarios.
pub fn exercise_root_directory(
    adapter: &mut dyn FatAdapter,
    checks: Checks<'_>,
    oracle: &mut Oracle<'_>,
) -> Result<(), String> {
    let root_entries = checks.geometry.root_entry_count;
    if root_entries == 0 {
        return Ok(());
    }
    let capacity = root_entries - 1;
    let mut expected = FsState::empty();
    for index in 0..capacity {
        step(
            adapter,
            &mut expected,
            create(format!("/R{index:05}.TXT"), vec![index as u8]),
        )
        .map_err(|error| {
            format!("root directory holds {index} of {capacity} 8.3 entries: {error}")
        })?;
    }
    expect_rejection(
        adapter,
        create("/OVERFLOW.TXT".into(), b"one too many".to_vec()),
        "full root directory",
    )?;
    checks.compare("after filling the root directory", &expected, oracle()?)?;

    step(
        adapter,
        &mut expected,
        Operation::Delete {
            path: "/R00000.TXT".into(),
        },
    )?;
    expect_rejection(
        adapter,
        create("/Two Slot Name.txt".into(), b"needs a long name".to_vec()),
        "root directory with one free slot",
    )?;
    checks.compare(
        "after rejecting a long name in the last root slot",
        &expected,
        oracle()?,
    )?;

    step(
        adapter,
        &mut expected,
        create("/R99999.TXT".into(), b"last slot".to_vec()),
    )?;
    expect_rejection(
        adapter,
        Operation::CreateDir {
            path: "/OVERFLOW".into(),
        },
        "refilled root directory",
    )?;
    checks.compare("after refilling the last root slot", &expected, oracle()?)
}

/// Writes files until the data region is exhausted, then checks that the
/// failure left a consistent image, that every cluster can be allocated, and
/// that deleted space is reusable.
pub fn exercise_data_region(
    adapter: &mut dyn FatAdapter,
    checks: Checks<'_>,
    oracle: &mut Oracle<'_>,
) -> Result<(), String> {
    let geometry = checks.geometry;
    let clusters_per_chunk = (geometry.cluster_count as usize / 48).max(65);
    let chunk = clusters_per_chunk * geometry.cluster_size + 1;
    let mut expected = FsState::empty();
    step(
        adapter,
        &mut expected,
        Operation::CreateDir {
            path: "/Fill".into(),
        },
    )?;

    let failed = fill_until_rejected(adapter, &mut expected, geometry.cluster_count, |index| {
        create(
            format!("/Fill/FILL{index:04}.BIN"),
            payload(chunk, index as u8),
        )
    })?;
    settle_partial(
        adapter,
        checks,
        oracle,
        &mut expected,
        &failed,
        "after exhausting the data region",
    )?;
    let free = (checks.free_clusters)()?;
    if free as usize > clusters_per_chunk + 1 {
        return Err(format!(
            "rejected a {chunk}-byte file while {free} clusters were free"
        ));
    }

    let failed = fill_until_rejected(
        adapter,
        &mut expected,
        clusters_per_chunk as u32 + 4,
        |index| create(format!("/ONE{index:04}.BIN"), vec![index as u8]),
    )?;
    settle_partial(
        adapter,
        checks,
        oracle,
        &mut expected,
        &failed,
        "after allocating the last clusters",
    )?;
    let free = (checks.free_clusters)()?;
    if free != 0 {
        return Err(format!(
            "{free} clusters can never be allocated by single-cluster files"
        ));
    }

    for path in ["/Fill/FILL0000.BIN", "/Fill/FILL0001.BIN"] {
        step(
            adapter,
            &mut expected,
            Operation::Delete { path: path.into() },
        )?;
    }
    step(
        adapter,
        &mut expected,
        create("/Fill/AGAIN.BIN".into(), payload(chunk, 0xfe)),
    )
    .map_err(|error| format!("space freed by deletion was not reusable: {error}"))?;
    checks.compare("after reusing freed clusters", &expected, oracle()?)
}

fn fill_until_rejected(
    adapter: &mut dyn FatAdapter,
    expected: &mut FsState,
    limit: u32,
    make: impl Fn(usize) -> Operation,
) -> Result<(String, Vec<u8>), String> {
    for index in 0..=limit as usize {
        let operation = make(index);
        match adapter.apply(&operation) {
            Ok(()) => expected.apply(&operation)?,
            Err(_) => {
                let Operation::CreateFile { path, data } = operation else {
                    unreachable!();
                };
                return Ok((path, data));
            }
        }
    }
    Err(format!("never ran out of space after {limit} files"))
}

/// The file whose creation failed may be absent or hold a prefix of its
/// intended contents; either way the rest of the image must be intact. A
/// surviving partial file is deleted so the model stays in step.
fn settle_partial(
    adapter: &mut dyn FatAdapter,
    checks: Checks<'_>,
    oracle: &mut Oracle<'_>,
    expected: &mut FsState,
    (path, data): &(String, Vec<u8>),
    context: &str,
) -> Result<(), String> {
    let actual = oracle()?;
    let mut tolerant = expected.clone();
    match actual.entries.get(path) {
        None => {}
        Some(entry) => match &entry.data {
            EntryData::File(contents) if data.starts_with(contents) => {
                tolerant.entries.insert(path.clone(), entry.clone());
            }
            EntryData::File(contents) => {
                return Err(format!(
                    "{context}: partial file {path} holds {} bytes that are not a prefix of its data",
                    contents.len()
                ));
            }
            EntryData::Directory => {
                return Err(format!("{context}: partial file {path} became a directory"));
            }
        },
    }
    checks.compare(context, &tolerant, actual)?;
    if tolerant.entries.contains_key(path) {
        adapter
            .apply(&Operation::Delete { path: path.clone() })
            .map_err(|error| format!("{context}: deleting partial file {path} failed: {error}"))?;
    }
    Ok(())
}

/// One file covering most of the data region, so its chain crosses every
/// FAT sector boundary and, on FAT32 volumes with enough clusters, pushes
/// later allocations above cluster 65535 where the high half of the first
/// cluster field matters.
pub fn large_extent_operations(geometry: ImageGeometry) -> Vec<Operation> {
    let cluster_size = geometry.cluster_size;
    let big = geometry.cluster_count as usize * 3 / 5 * cluster_size + 7;
    vec![
        create("/big extent.bin".into(), payload(big, 0xa1)),
        create("/after.txt".into(), b"allocated after the extent".to_vec()),
        Operation::CreateDir {
            path: "/After Dir".into(),
        },
        create(
            "/After Dir/inner.txt".into(),
            payload(cluster_size + 1, 0xa4),
        ),
        Operation::TruncateFile {
            path: "/big extent.bin".into(),
            len: big / 3,
        },
        Operation::CreateDir {
            path: "/Low Dir".into(),
        },
        Operation::AppendFile {
            path: "/big extent.bin".into(),
            data: payload(cluster_size * 5 + 3, 0xa2),
        },
        Operation::Rename {
            from: "/After Dir".into(),
            to: "/Low Dir/After Dir Moved".into(),
        },
        Operation::Rename {
            from: "/Low Dir/After Dir Moved/inner.txt".into(),
            to: "/inner moved up.txt".into(),
        },
        Operation::TruncateFile {
            path: "/big extent.bin".into(),
            len: 0,
        },
        Operation::AppendFile {
            path: "/big extent.bin".into(),
            data: payload(big / 2, 0xa3),
        },
        Operation::ReplaceFile {
            path: "/after.txt".into(),
            data: payload(cluster_size * 2, 0xa5),
        },
        Operation::Delete {
            path: "/big extent.bin".into(),
        },
    ]
}
