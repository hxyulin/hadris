//! Bidirectional accuracy of external ISO tools: xorriso/libisofs and mkisofs.

use hadris_tests::harness::{Workspace, write_report};
use hadris_tests::iso::mkisofs::Mkisofs;
use hadris_tests::iso::xorriso::{self, Xorriso};
use hadris_tests::iso::{FORMAT, IsoProducer, conformance_scenarios, measure};

#[test]
#[ignore = "manual xorriso and mkisofs accuracy suite; run through nix develop"]
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
