//! Bidirectional accuracy of independent FAT implementations: rust-fatfs and
//! GNU mtools with dosfstools.

use std::path::Path;

use hadris_tests::fat::fatfs::{self, FatfsAdapter};
use hadris_tests::fat::hadris::{self, HadrisFatAdapter};
use hadris_tests::fat::limits::{
    Checks, Oracle, exercise_data_region, exercise_root_directory, large_extent_operations,
};
use hadris_tests::fat::mtools::{self, MtoolsFatAdapter};
use hadris_tests::fat::scenarios::{
    RejectionScenario, dot_entry_operations, interoperability_scenarios, rejection_scenarios,
};
use hadris_tests::fat::{
    FAT_CASES, FORMAT, FatAdapter, FatCase, FsState, Operation, apply_operations,
    apply_operations_without_attrs, apply_rejection, clear_mutable_attrs, compare_snapshot,
    format_trace, spec, summarize_operation,
};
use hadris_tests::harness::{Scorecard, Workspace, catch_panic, write_report};

const FATFS_READS: &str = "rust-fatfs reading Hadris";
const FATFS_WRITES: &str = "rust-fatfs writing spec-valid images";
const FATFS_REJECTS: &str = "rust-fatfs rejecting invalid operations";
const FATFS_LIMITS: &str = "rust-fatfs at volume limits";
const MTOOLS_READS: &str = "mtools reading Hadris";
const MTOOLS_WRITES: &str = "mtools writing spec-valid images";
const MTOOLS_REJECTS: &str = "mtools rejecting invalid operations";
const MTOOLS_LIMITS: &str = "mtools at volume limits";
const FSCK_HADRIS: &str = "fsck.fat accepting Hadris images";
const FSCK_MTOOLS: &str = "fsck.fat accepting mtools images";

fn fatfs_scorecard() -> Scorecard {
    Scorecard::new(fatfs::NAME).headline(&[FATFS_READS, FATFS_WRITES, FATFS_REJECTS, FATFS_LIMITS])
}

fn mtools_scorecard() -> Scorecard {
    Scorecard::new(mtools::NAME).headline(&[
        MTOOLS_READS,
        MTOOLS_WRITES,
        MTOOLS_REJECTS,
        MTOOLS_LIMITS,
    ])
}

/// A spec-valid peer image must also be readable by Hadris; a mismatch there
/// is a Hadris failure, not a peer measurement.
fn hadris_reads(image: &Path, case: FatCase, ignore_attrs: bool) -> Result<(), String> {
    let mut oracle = spec::snapshot(image, case.bits)?;
    let mut hadris = HadrisFatAdapter::new(image.to_path_buf()).snapshot()?;
    if ignore_attrs {
        clear_mutable_attrs(&mut oracle);
        clear_mutable_attrs(&mut hadris);
    }
    compare_snapshot(
        &format!("{} Hadris reading a spec-valid peer image", case.name),
        &oracle,
        &hadris,
    )
}

type OpenAdapter<'a> = &'a dyn Fn(&Path, &Workspace) -> Result<Box<dyn FatAdapter>, String>;
type Exercise<'a> =
    &'a dyn Fn(&mut dyn FatAdapter, Checks<'_>, &mut Oracle<'_>) -> Result<(), String>;

/// The three limit exercises, each on a freshly formatted image.
fn limit_exercises(
    case: FatCase,
    ignore_attrs: bool,
    format: &dyn Fn(&Path) -> Result<(), String>,
    open: OpenAdapter<'_>,
) -> Vec<(&'static str, Result<(), String>)> {
    let exercises: [(&'static str, Exercise<'_>); 3] = [
        ("root-directory-capacity", &exercise_root_directory),
        ("data-region-exhaustion", &exercise_data_region),
        ("large-extents", &|adapter, checks, oracle| {
            let operations = large_extent_operations(checks.geometry);
            let mut expected = FsState::empty();
            for (index, operation) in operations.iter().enumerate() {
                adapter.apply(operation).map_err(|error| {
                    format!(
                        "operation {index} failed: {error}\ntrace:\n{}",
                        format_trace(&operations[..=index])
                    )
                })?;
                expected.apply(operation)?;
            }
            let mut actual = oracle()?;
            if checks.ignore_attrs {
                clear_mutable_attrs(&mut expected);
                clear_mutable_attrs(&mut actual);
            }
            compare_snapshot("large extents", &expected, &actual)
        }),
    ];
    exercises
        .into_iter()
        .map(|(name, exercise)| {
            let outcome = (|| {
                let workspace = Workspace::new(FORMAT, &format!("{}-{name}-", case.name))?;
                let image = workspace.path.join("peer.img");
                format(&image)?;
                let geometry = spec::geometry(&image)?;
                let free = || spec::free_clusters(&image);
                let checks = Checks {
                    ignore_attrs,
                    geometry,
                    free_clusters: &free,
                };
                let mut adapter = open(&image, &workspace)?;
                let mut oracle = || spec::snapshot(&image, case.bits);
                exercise(adapter.as_mut(), checks, &mut oracle)?;
                hadris_reads(&image, case, ignore_attrs)
            })();
            (name, outcome)
        })
        .collect()
}

fn measure_fatfs_rejection(
    case: FatCase,
    scenario: &RejectionScenario,
    scorecard: &mut Scorecard,
) -> Result<(), String> {
    let workspace = Workspace::new(FORMAT, &format!("{}-{}-fatfs-", case.name, scenario.name))?;
    let image = workspace.path.join("fatfs.img");
    let outcome = catch_panic(fatfs::NAME, || {
        fatfs::format(&image, case)?;
        let mut expected = apply_rejection(&mut FatfsAdapter::new(image.clone()), scenario)?;
        let mut oracle = spec::snapshot(&image, case.bits)?;
        clear_mutable_attrs(&mut expected);
        clear_mutable_attrs(&mut oracle);
        compare_snapshot("image after the rejected operation", &expected, &oracle)
    });
    scorecard.record(
        FATFS_REJECTS,
        format!("{} {} rust-fatfs", case.name, scenario.name),
        outcome,
    );
    Ok(())
}

fn measure_fatfs_limits(case: FatCase, scorecard: &mut Scorecard) {
    for (name, outcome) in limit_exercises(
        case,
        true,
        &|image| fatfs::format(image, case),
        &|image, _| Ok(Box::new(FatfsAdapter::new(image.to_path_buf()))),
    ) {
        scorecard.record(
            FATFS_LIMITS,
            format!("{} {name} rust-fatfs", case.name),
            catch_panic(fatfs::NAME, || outcome),
        );
    }
}

fn measure_mtools_rejection(
    case: FatCase,
    scenario: &RejectionScenario,
    scorecard: &mut Scorecard,
) -> Result<(), String> {
    let workspace = Workspace::new(FORMAT, &format!("{}-{}-mtools-", case.name, scenario.name))?;
    let image = workspace.path.join("mtools.img");
    let outcome = mtools::format(&image, case).and_then(|()| {
        let mut adapter = MtoolsFatAdapter::new(image.clone(), &workspace.path)?;
        let expected = apply_rejection(&mut adapter, scenario)?;
        let oracle = spec::snapshot(&image, case.bits)?;
        compare_snapshot("image after the rejected operation", &expected, &oracle)?;
        mtools::fsck(&image)
    });
    scorecard.record(
        MTOOLS_REJECTS,
        format!("{} {} mtools", case.name, scenario.name),
        outcome,
    );
    Ok(())
}

fn measure_mtools_limits(case: FatCase, scorecard: &mut Scorecard) {
    for (name, outcome) in limit_exercises(
        case,
        false,
        &|image| mtools::format(image, case),
        &|image, workspace| {
            Ok(Box::new(MtoolsFatAdapter::new(
                image.to_path_buf(),
                &workspace.path,
            )?))
        },
    ) {
        scorecard.record(
            MTOOLS_LIMITS,
            format!("{} {name} mtools", case.name),
            outcome,
        );
    }
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
    let context = format!("{} {scenario}", case.name);
    match hadris_image(
        &workspace,
        case,
        operations,
        "Hadris image before rust-fatfs measurement",
    ) {
        Ok((hadris_image, expected)) => {
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
        }
        Err(error) => scorecard.details.push(format!(
            "{context} read measurement skipped, Hadris could not produce the reference image: {error}"
        )),
    }

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
    let context = format!("{} {scenario}", case.name);
    match hadris_image(
        &hadris_workspace,
        case,
        operations,
        "Hadris image before external measurement",
    ) {
        Ok((hadris_image, expected)) => {
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
        }
        Err(error) => scorecard.details.push(format!(
            "{context} read measurement skipped, Hadris could not produce the reference image: {error}"
        )),
    }

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
    for scenario in rejection_scenarios() {
        for case in FAT_CASES {
            measure_fatfs_rejection(case, &scenario, &mut scorecard).unwrap();
        }
    }
    for case in FAT_CASES {
        measure_fatfs_limits(case, &mut scorecard);
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
fn external_tools_interoperate_with_hadris() {
    if !mtools::require_tools() {
        return;
    }
    let operations = hadris_tests::fat::scenarios::curated_operations();
    let mut scorecard = mtools_scorecard();
    for case in FAT_CASES {
        measure_mtools(case, "curated", &operations, &mut scorecard).unwrap_or_else(|error| {
            panic!(
                "{} curated: {error}\ntrace:\n{}",
                case.name,
                format_trace(&operations)
            )
        });
    }
    scorecard
        .require_all(&[
            (MTOOLS_READS, FAT_CASES.len()),
            (FSCK_HADRIS, FAT_CASES.len()),
        ])
        .unwrap();
}

#[test]
#[ignore = "manual mtools and dosfstools accuracy suite"]
fn mtools_accuracy_report() {
    assert!(mtools::require_tools());
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
    for scenario in rejection_scenarios() {
        for case in FAT_CASES {
            measure_mtools_rejection(case, &scenario, &mut scorecard).unwrap();
        }
    }
    for case in FAT_CASES {
        measure_mtools_limits(case, &mut scorecard);
    }
    write_report(FORMAT, "mtools-accuracy.txt", &scorecard.report()).unwrap();
}
