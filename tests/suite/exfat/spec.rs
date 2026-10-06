//! Hadris exFAT writer and reader against the raw-image specification
//! oracle, on the shared FAT traces, the exFAT scenarios and the seeded
//! traces.

use hadris_tests::exfat::generic::{self, HadrisExFatAdapter};
use hadris_tests::exfat::scenarios::exfat_scenarios;
use hadris_tests::exfat::{EXFAT_CASES, FORMAT, spec};
use hadris_tests::fat::scenarios::{edge_case_scenarios, specification_scenarios};
use hadris_tests::fat::{FatAdapter, Operation, apply_operations, compare_snapshot, format_trace};
use hadris_tests::harness::Workspace;

pub fn run_spec_matrix(operations: &[Operation]) -> Result<(), String> {
    for case in EXFAT_CASES {
        let workspace = Workspace::new(FORMAT, &format!("{}-spec-", case.name))?;
        let image = workspace.path.join(format!("{}-hadris.img", case.name));
        generic::format(&image, case)?;
        let expected = apply_operations(&mut HadrisExFatAdapter::new(image.clone()), operations)?;
        let oracle = spec::snapshot(&image)?;
        compare_snapshot(
            &format!("{} exFAT specification oracle", case.name),
            &expected,
            &oracle,
        )?;
        let hadris = HadrisExFatAdapter::new(image).snapshot()?;
        compare_snapshot(&format!("{} Hadris reader", case.name), &expected, &hadris)?;
    }
    Ok(())
}

fn report_failures(scenarios: Vec<(String, Vec<Operation>)>) {
    let failures: Vec<String> = scenarios
        .into_iter()
        .filter_map(|(scenario, operations)| {
            run_spec_matrix(&operations)
                .err()
                .map(|error| format!("{scenario}: {error}\ntrace:\n{}", format_trace(&operations)))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} scenarios failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn exfat_spec_conformance() {
    report_failures(specification_scenarios());
}

#[test]
fn exfat_edge_cases_match_spec() {
    report_failures(edge_case_scenarios());
}

#[test]
fn exfat_scenarios_match_spec() {
    report_failures(exfat_scenarios());
}
