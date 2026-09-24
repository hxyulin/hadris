//! Hadris at the edges of an exFAT volume and on operations it must
//! refuse.

use std::path::{Path, PathBuf};

use hadris_tests::exfat::generic::{self, HadrisExFatAdapter};
use hadris_tests::exfat::limits::exercise_directory_growth;
use hadris_tests::exfat::scenarios::exfat_rejection_scenarios;
use hadris_tests::exfat::{EXFAT_CASES, ExFatCase, FORMAT, spec};
use hadris_tests::fat::limits::{Checks, Oracle, exercise_data_region, large_extent_operations};
use hadris_tests::fat::{FatAdapter, FsState, apply_rejection, compare_snapshot, format_trace};
use hadris_tests::harness::Workspace;

fn hadris_image(case: ExFatCase, topic: &str) -> Result<(Workspace, PathBuf), String> {
    let workspace = Workspace::new(FORMAT, &format!("{}-{topic}-", case.name))?;
    let image = workspace.path.join("hadris.img");
    generic::format(&image, case)?;
    Ok((workspace, image))
}

fn verify(image: &Path, case: ExFatCase, expected: &FsState) -> Result<(), String> {
    let oracle = spec::snapshot(image)?;
    compare_snapshot(
        &format!("{} exFAT specification oracle", case.name),
        expected,
        &oracle,
    )?;
    let hadris = HadrisExFatAdapter::new(image.to_path_buf()).snapshot()?;
    compare_snapshot(&format!("{} Hadris reader", case.name), expected, &hadris)
}

fn report(failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} failures:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn hadris_rejects_invalid_operations() {
    let mut failures = Vec::new();
    for scenario in exfat_rejection_scenarios() {
        for case in EXFAT_CASES {
            let outcome = hadris_image(case, "reject").and_then(|(_workspace, image)| {
                let expected =
                    apply_rejection(&mut HadrisExFatAdapter::new(image.clone()), &scenario)?;
                verify(&image, case, &expected)
            });
            if let Err(error) = outcome {
                failures.push(format!(
                    "{} {}: {error}\nsetup:\n{}",
                    case.name,
                    scenario.name,
                    format_trace(&scenario.setup)
                ));
            }
        }
    }
    report(failures);
}

type Exercise = fn(&mut dyn FatAdapter, Checks<'_>, &mut Oracle<'_>) -> Result<(), String>;

fn run_exercise(case: ExFatCase, topic: &str, exercise: Exercise) -> Result<(), String> {
    let (_workspace, image) = hadris_image(case, topic)?;
    let geometry = spec::geometry(&image)?;
    let free = || spec::free_clusters(&image);
    let checks = Checks {
        ignore_attrs: false,
        geometry,
        free_clusters: &free,
    };
    let mut adapter = HadrisExFatAdapter::new(image.clone());
    let mut oracle = || spec::snapshot(&image);
    exercise(&mut adapter, checks, &mut oracle)
}

#[test]
fn hadris_data_region_exhaustion() {
    let mut failures = Vec::new();
    for case in EXFAT_CASES {
        if let Err(error) = run_exercise(case, "full", exercise_data_region) {
            failures.push(format!("{}: {error}", case.name));
        }
    }
    report(failures);
}

#[test]
fn hadris_directory_growth_to_a_full_volume() {
    let case = ExFatCase {
        name: "exfat-1m",
        size: 1024 * 1024,
        cluster: 4096,
    };
    if let Err(error) = run_exercise(case, "grow", exercise_directory_growth) {
        panic!("{}: {error}", case.name);
    }
}

#[test]
fn hadris_large_extents() {
    let mut failures = Vec::new();
    for case in EXFAT_CASES {
        let outcome = hadris_image(case, "extent").and_then(|(_workspace, image)| {
            let operations = large_extent_operations(spec::geometry(&image)?);
            let mut adapter = HadrisExFatAdapter::new(image.clone());
            let mut expected = FsState::empty();
            for (index, operation) in operations.iter().enumerate() {
                adapter.apply(operation).map_err(|error| {
                    format!(
                        "operation {index} failed: {error}\ntrace:\n{}",
                        format_trace(&operations[..=index])
                    )
                })?;
                expected.apply(operation)?;
                verify(&image, case, &expected).map_err(|error| {
                    format!(
                        "after operation {index}: {error}\ntrace:\n{}",
                        format_trace(&operations[..=index])
                    )
                })?;
            }
            Ok(())
        });
        if let Err(error) = outcome {
            failures.push(format!("{}: {error}", case.name));
        }
    }
    report(failures);
}
