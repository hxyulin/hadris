//! Hadris at the edges of a volume and on operations it must refuse.

use std::path::{Path, PathBuf};

use hadris_tests::fat::hadris::{self, HadrisFatAdapter};
use hadris_tests::fat::limits::{
    Checks, exercise_data_region, exercise_root_directory, large_extent_operations,
};
use hadris_tests::fat::scenarios::rejection_scenarios;
use hadris_tests::fat::{
    FAT_CASES, FORMAT, FatAdapter, FatCase, FsState, apply_rejection, compare_snapshot,
    format_trace, spec,
};
use hadris_tests::harness::Workspace;

fn hadris_image(case: FatCase, topic: &str) -> Result<(Workspace, PathBuf), String> {
    let workspace = Workspace::new(FORMAT, &format!("{}-{topic}-", case.name))?;
    let image = workspace.path.join("hadris.img");
    hadris::format(&image, case)?;
    Ok((workspace, image))
}

fn verify(image: &Path, case: FatCase, expected: &FsState) -> Result<(), String> {
    let oracle = spec::snapshot(image, case.bits)?;
    compare_snapshot(
        &format!("{} FAT specification oracle", case.name),
        expected,
        &oracle,
    )?;
    let hadris = HadrisFatAdapter::new(image.to_path_buf()).snapshot()?;
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
    for scenario in rejection_scenarios() {
        for case in FAT_CASES {
            let outcome = hadris_image(case, "reject").and_then(|(_workspace, image)| {
                let expected =
                    apply_rejection(&mut HadrisFatAdapter::new(image.clone()), &scenario)?;
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

#[test]
fn hadris_root_directory_capacity() {
    let mut failures = Vec::new();
    for case in FAT_CASES {
        let outcome = hadris_image(case, "root").and_then(|(_workspace, image)| {
            let geometry = spec::geometry(&image)?;
            let free = || spec::free_clusters(&image);
            let checks = Checks {
                ignore_attrs: false,
                geometry,
                free_clusters: &free,
            };
            let mut adapter = HadrisFatAdapter::new(image.clone());
            let mut oracle = || spec::snapshot(&image, case.bits);
            exercise_root_directory(&mut adapter, checks, &mut oracle)
        });
        if let Err(error) = outcome {
            failures.push(format!("{}: {error}", case.name));
        }
    }
    report(failures);
}

#[test]
fn hadris_data_region_exhaustion() {
    let mut failures = Vec::new();
    for case in FAT_CASES {
        let outcome = hadris_image(case, "full").and_then(|(_workspace, image)| {
            let geometry = spec::geometry(&image)?;
            let free = || spec::free_clusters(&image);
            let checks = Checks {
                ignore_attrs: false,
                geometry,
                free_clusters: &free,
            };
            let mut adapter = HadrisFatAdapter::new(image.clone());
            let mut oracle = || spec::snapshot(&image, case.bits);
            exercise_data_region(&mut adapter, checks, &mut oracle)
        });
        if let Err(error) = outcome {
            failures.push(format!("{}: {error}", case.name));
        }
    }
    report(failures);
}

#[test]
fn hadris_large_extents() {
    let mut failures = Vec::new();
    for case in FAT_CASES {
        let outcome = hadris_image(case, "extent").and_then(|(_workspace, image)| {
            let operations = large_extent_operations(spec::geometry(&image)?);
            let mut adapter = HadrisFatAdapter::new(image.clone());
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
