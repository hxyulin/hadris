//! Bidirectional accuracy of independent FAT implementations: rust-fatfs and
//! GNU mtools with dosfstools.

use hadris_tests::fat::fatfs::{self, FatfsAdapter};
use hadris_tests::fat::hadris::{self, HadrisFatAdapter};
use hadris_tests::fat::mtools::{self, MtoolsFatAdapter};
use hadris_tests::fat::scenarios::{dot_entry_operations, interoperability_scenarios};
use hadris_tests::fat::{
    FAT_CASES, FORMAT, FatAdapter, FatCase, FsState, Operation, apply_operations,
    apply_operations_without_attrs, clear_mutable_attrs, compare_snapshot, format_trace, spec,
    summarize_operation,
};
use hadris_tests::harness::{Scorecard, Workspace, catch_panic, write_report};

const FATFS_READS: &str = "rust-fatfs reading Hadris";
const FATFS_WRITES: &str = "rust-fatfs writing spec-valid images";
const MTOOLS_READS: &str = "mtools reading Hadris";
const MTOOLS_WRITES: &str = "mtools writing spec-valid images";
const FSCK_HADRIS: &str = "fsck.fat accepting Hadris images";
const FSCK_MTOOLS: &str = "fsck.fat accepting mtools images";

fn fatfs_scorecard() -> Scorecard {
    Scorecard::new(fatfs::NAME).headline(&[FATFS_READS, FATFS_WRITES])
}

fn mtools_scorecard() -> Scorecard {
    Scorecard::new(mtools::NAME).headline(&[MTOOLS_READS, MTOOLS_WRITES])
}

/// Prepares a spec-valid Hadris image for `operations` and returns the model
/// state a peer reader must reproduce.
fn hadris_image(
    workspace: &Workspace,
    case: FatCase,
    operations: &[Operation],
    context: &str,
) -> Result<(std::path::PathBuf, FsState), String> {
    let image = workspace.path.join("hadris.img");
    hadris::format(&image, case)?;
    let expected = apply_operations(&mut HadrisFatAdapter::new(image.clone()), operations)?;
    let oracle = spec::snapshot(&image, case.bits)?;
    compare_snapshot(context, &expected, &oracle)?;
    Ok((image, expected))
}

fn measure_fatfs(
    case: FatCase,
    scenario: &str,
    operations: &[Operation],
    scorecard: &mut Scorecard,
) -> Result<(), String> {
    let workspace = Workspace::new(FORMAT, &format!("{}-{scenario}-fatfs-", case.name))?;
    let (hadris_image, expected) = hadris_image(
        &workspace,
        case,
        operations,
        "Hadris image before rust-fatfs measurement",
    )?;
    let context = format!("{} {scenario}", case.name);

    scorecard.record(
        FATFS_READS,
        format!("{context} rust-fatfs read mismatch"),
        catch_panic(fatfs::NAME, || {
            fatfs::snapshot(&hadris_image).and_then(|snapshot| {
                compare_snapshot(
                    &format!("{} rust-fatfs reading Hadris", case.name),
                    &expected,
                    &snapshot,
                )
            })
        }),
    );

    let fatfs_image = workspace.path.join("fatfs.img");
    let written = catch_panic(fatfs::NAME, || {
        fatfs::format(&fatfs_image, case)?;
        let mut expected = apply_operations_without_attrs(
            &mut FatfsAdapter::new(fatfs_image.clone()),
            operations,
        )?;
        let mut oracle = spec::snapshot(&fatfs_image, case.bits)?;
        clear_mutable_attrs(&mut expected);
        clear_mutable_attrs(&mut oracle);
        compare_snapshot(
            &format!("{} FAT specification oracle reading rust-fatfs", case.name),
            &expected,
            &oracle,
        )?;
        let mut hadris = HadrisFatAdapter::new(fatfs_image.clone()).snapshot()?;
        clear_mutable_attrs(&mut hadris);
        compare_snapshot(
            &format!("{} Hadris reading rust-fatfs", case.name),
            &expected,
            &hadris,
        )
    });
    scorecard.record(
        FATFS_WRITES,
        format!("{context} rust-fatfs write mismatch"),
        written,
    );
    Ok(())
}

fn measure_mtools(
    case: FatCase,
    scenario: &str,
    operations: &[Operation],
    scorecard: &mut Scorecard,
) -> Result<(), String> {
    let hadris_workspace = Workspace::new(FORMAT, &format!("{}-{scenario}-hadris-", case.name))?;
    let (hadris_image, expected) = hadris_image(
        &hadris_workspace,
        case,
        operations,
        "Hadris image before external measurement",
    )?;
    let context = format!("{} {scenario}", case.name);

    scorecard.record(
        MTOOLS_READS,
        format!("{context} mtools read mismatch"),
        MtoolsFatAdapter::new(hadris_image.clone(), &hadris_workspace.path)
            .and_then(|mut adapter| adapter.snapshot())
            .and_then(|snapshot| compare_snapshot("mtools reader", &expected, &snapshot)),
    );
    scorecard.record(
        FSCK_HADRIS,
        format!("{context} fsck rejected Hadris image"),
        mtools::fsck(&hadris_image),
    );

    scorecard.attempt(MTOOLS_WRITES);
    let mtools_workspace = Workspace::new(FORMAT, &format!("{}-{scenario}-mtools-", case.name))?;
    let mtools_image = mtools_workspace.path.join("mtools.img");
    if let Err(error) = mtools::format(&mtools_image, case) {
        scorecard.command_failure(format!("{context} mkfs.fat failed: {error}"));
        return Ok(());
    }
    let mut adapter = MtoolsFatAdapter::new(mtools_image.clone(), &mtools_workspace.path)?;
    let mut written_model = FsState::empty();
    for (index, operation) in operations.iter().enumerate() {
        if let Err(error) = adapter.apply(operation) {
            scorecard.command_failure(format!(
                "{context} mtools operation {index} failed ({}): {error}\ntrace:\n{}",
                summarize_operation(operation),
                format_trace(&operations[..=index])
            ));
            return Ok(());
        }
        written_model.apply(operation)?;
    }
    match spec::snapshot(&mtools_image, case.bits)
        .and_then(|snapshot| compare_snapshot("mtools writer", &written_model, &snapshot))
    {
        Ok(()) => {
            scorecard.pass(MTOOLS_WRITES);
            let hadris = HadrisFatAdapter::new(mtools_image.clone()).snapshot()?;
            compare_snapshot(
                "Hadris reading spec-valid mtools image",
                &written_model,
                &hadris,
            )?;
        }
        Err(error) => scorecard
            .details
            .push(format!("{context} mtools write mismatch: {error}")),
    }
    scorecard.attempt(FSCK_MTOOLS);
    if mtools::fsck(&mtools_image).is_ok() {
        scorecard.pass(FSCK_MTOOLS);
    }
    Ok(())
}

#[test]
#[ignore = "manual bidirectional rust-fatfs accuracy suite"]
fn fatfs_accuracy_report() {
    let mut scorecard = fatfs_scorecard();
    for (scenario, operations) in interoperability_scenarios() {
        for case in FAT_CASES {
            measure_fatfs(case, &scenario, &operations, &mut scorecard).unwrap_or_else(|error| {
                panic!(
                    "{} {scenario}: {error}\ntrace:\n{}",
                    case.name,
                    format_trace(&operations)
                )
            });
        }
    }
    write_report(FORMAT, "fatfs-accuracy.txt", &scorecard.report()).unwrap();
}

#[test]
fn fatfs_dot_entries_match_spec() {
    let operations = dot_entry_operations();
    let mut scorecard = fatfs_scorecard();
    for case in FAT_CASES {
        measure_fatfs(case, "dot-entries", &operations, &mut scorecard).unwrap();
    }
    assert!(scorecard.all_passed(FATFS_WRITES), "{}", scorecard.report());
}

#[test]
#[ignore = "requires mtools and dosfstools; run through nix develop"]
fn mtools_accuracy_report() {
    mtools::require_tools().unwrap();
    let mut scorecard = mtools_scorecard();
    for (scenario, operations) in interoperability_scenarios() {
        for case in FAT_CASES {
            measure_mtools(case, &scenario, &operations, &mut scorecard).unwrap_or_else(|error| {
                panic!(
                    "{} {scenario}: {error}\ntrace:\n{}",
                    case.name,
                    format_trace(&operations)
                )
            });
        }
    }
    write_report(FORMAT, "mtools-accuracy.txt", &scorecard.report()).unwrap();
}
