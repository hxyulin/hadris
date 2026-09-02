//! Bidirectional accuracy of external ISO tools: xorriso/libisofs and mkisofs.

use hadris_tests::harness::{Workspace, write_report};
use hadris_tests::iso::mkisofs::Mkisofs;
use hadris_tests::iso::xorriso::{self, Xorriso};
use hadris_tests::iso::{FORMAT, IsoProducer, conformance_scenarios, measure};

#[test]
fn external_tools_interoperate_with_hadris() -> Result<(), String> {
    if !xorriso::require() {
        return Ok(());
    }
    let mut scorecard = measure::scorecard(xorriso::NAME);
    let scenarios = conformance_scenarios();
    let expected_attempts = scenarios.len();
    for (scenario, expected) in scenarios {
        let workspace = Workspace::new(FORMAT, &format!("{scenario}-xorriso-strict-"))?;
        measure::producer(
            &mut scorecard,
            scenario,
            &expected,
            &Xorriso,
            &workspace.path,
        )?;
        measure::consumer(
            &mut scorecard,
            scenario,
            &expected,
            &Xorriso,
            &workspace.path,
        )?;
    }
    let labels = measure::labels(xorriso::NAME);
    scorecard.require_all(&[
        (&labels.reads, expected_attempts),
        (&labels.writes, expected_attempts),
        (&labels.hadris_reads, expected_attempts),
    ])
}

#[test]
#[ignore = "manual xorriso and mkisofs accuracy suite"]
fn external_iso_tool_accuracy_report() -> Result<(), String> {
    assert!(xorriso::available(), "xorriso is required");
    let mkisofs = Mkisofs::detect().expect("mkisofs or genisoimage is required");
    let mut xorriso_score = measure::scorecard(xorriso::NAME);
    let mut mkisofs_score = measure::scorecard(&mkisofs.name());
    for (scenario, expected) in conformance_scenarios() {
        let workspace = Workspace::new(FORMAT, &format!("{scenario}-iso-tools-"))?;
        measure::producer(
            &mut xorriso_score,
            scenario,
            &expected,
            &Xorriso,
            &workspace.path,
        )?;
        measure::producer(
            &mut mkisofs_score,
            scenario,
            &expected,
            &mkisofs,
            &workspace.path,
        )?;
        measure::consumer(
            &mut xorriso_score,
            scenario,
            &expected,
            &Xorriso,
            &workspace.path,
        )?;
    }
    let report = format!("{}\n\n{}", xorriso_score.report(), mkisofs_score.report());
    write_report(FORMAT, "external-tools-accuracy.txt", &report)
}
