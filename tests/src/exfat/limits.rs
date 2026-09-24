//! exFAT's own limit exercise: a directory that grows until the volume has
//! no cluster left for it. The data-region and long-extent exercises of the
//! FAT suite apply unchanged; exFAT has no fixed root directory.

use crate::fat::adapter::FatAdapter;
use crate::fat::limits::{Checks, Oracle};
use crate::fat::model::{FsState, Operation, compare_snapshot, summarize_operation};

/// A name that takes the most File Name entries a set can hold.
fn long_name(index: usize) -> String {
    format!("{index:05}{}", "n".repeat(250))
}

/// Fills one directory with empty files whose names take nineteen entries
/// each, so the directory grows cluster by cluster until the volume is out
/// of space, then checks the image, frees entries in the middle and fills
/// the holes again.
pub fn exercise_directory_growth(
    adapter: &mut dyn FatAdapter,
    checks: Checks<'_>,
    oracle: &mut Oracle<'_>,
) -> Result<(), String> {
    let mut expected = FsState::empty();
    let dir = Operation::CreateDir {
        path: "/Grown".into(),
    };
    adapter.apply(&dir)?;
    expected.apply(&dir)?;
    let limit = checks.geometry.cluster_count as usize * checks.geometry.cluster_size / 608 + 2;
    let mut created = 0;
    loop {
        if created > limit {
            return Err(format!(
                "the directory never ran out of space after {created} files"
            ));
        }
        let operation = Operation::CreateFile {
            path: format!("/Grown/{}", long_name(created)),
            data: Vec::new(),
        };
        match adapter.apply(&operation) {
            Ok(()) => {
                expected.apply(&operation)?;
                created += 1;
            }
            Err(_) => break,
        }
    }
    if created < 2 {
        return Err(format!("only {created} files fit"));
    }
    compare_snapshot("after filling the directory", &expected, &oracle()?)?;
    let free = (checks.free_clusters)()?;
    if free != 0 {
        return Err(format!(
            "the directory stopped growing with {free} clusters free"
        ));
    }
    for index in [created / 3, created / 2] {
        let operation = Operation::Delete {
            path: format!("/Grown/{}", long_name(index)),
        };
        adapter
            .apply(&operation)
            .map_err(|error| format!("{} failed: {error}", summarize_operation(&operation)))?;
        expected.apply(&operation)?;
    }
    for index in [created, created + 1] {
        let operation = Operation::CreateFile {
            path: format!("/Grown/{}", long_name(index)),
            data: Vec::new(),
        };
        adapter
            .apply(&operation)
            .map_err(|error| format!("freed entries were not reused: {error}"))?;
        expected.apply(&operation)?;
    }
    compare_snapshot("after reusing freed entries", &expected, &oracle()?)
}
