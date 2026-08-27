//! Hadris FAT writer and reader against the raw-image specification oracle.

use hadris_tests::fat::hadris::{self, HadrisFatAdapter};
use hadris_tests::fat::scenarios::{edge_case_scenarios, specification_scenarios};
use hadris_tests::fat::{
    FAT_CASES, FORMAT, FatAdapter, Operation, apply_operations, compare_snapshot, format_trace,
    spec,
};
use hadris_tests::harness::Workspace;

pub fn run_spec_matrix(operations: &[Operation]) -> Result<(), String> {
    for case in FAT_CASES {
        let workspace = Workspace::new(FORMAT, &format!("{}-spec-", case.name))?;
        let image = workspace.path.join(format!("{}-hadris.img", case.name));
        hadris::format(&image, case)?;
        let expected = apply_operations(&mut HadrisFatAdapter::new(image.clone()), operations)?;
        let oracle = spec::snapshot(&image, case.bits)?;
        compare_snapshot(
            &format!("{} FAT specification oracle", case.name),
            &expected,
            &oracle,
        )?;
        let hadris = HadrisFatAdapter::new(image).snapshot()?;
        compare_snapshot(&format!("{} Hadris reader", case.name), &expected, &hadris)?;
    }
    Ok(())
}

#[test]
#[ignore = "manual FAT specification conformance suite"]
fn fat_spec_conformance() {
    for (scenario, operations) in specification_scenarios() {
        run_spec_matrix(&operations).unwrap_or_else(|error| {
            panic!("{scenario}: {error}\ntrace:\n{}", format_trace(&operations))
        });
    }
}

#[test]
fn fat_edge_cases_match_spec() {
    for (scenario, operations) in edge_case_scenarios() {
        run_spec_matrix(&operations).unwrap_or_else(|error| {
            panic!("{scenario}: {error}\ntrace:\n{}", format_trace(&operations))
        });
    }
}
