//! The host platform's ISO producer and kernel ISO 9660 reader.

use hadris_tests::harness::{NATIVE_MOUNT_ENV, Workspace, native_mount_enabled, write_report};
use hadris_tests::iso::native::{Hdiutil, NativeReader};
use hadris_tests::iso::{FORMAT, IsoConsumer, IsoProducer, conformance_scenarios, measure};

#[test]
#[ignore = "manual macOS hdiutil producer accuracy suite"]
fn native_iso_producer_accuracy_report() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut scorecard = measure::scorecard(&Hdiutil.name());
    for (scenario, expected) in conformance_scenarios() {
        let workspace = Workspace::new(FORMAT, &format!("{scenario}-hdiutil-"))?;
        measure::producer(
            &mut scorecard,
            scenario,
            &expected,
            &Hdiutil,
            &workspace.path,
        )?;
    }
    write_report(FORMAT, "macos-hdiutil-accuracy.txt", &scorecard.report())
}

#[test]
#[ignore = "manual native ISO reader accuracy suite"]
fn native_iso_reader_accuracy_report() -> Result<(), String> {
    if !native_mount_enabled() {
        return Err(format!("{NATIVE_MOUNT_ENV}=1 is required"));
    }
    let platform = std::env::consts::OS;
    let mut scorecard = measure::scorecard(&NativeReader.name());
    for (scenario, expected) in conformance_scenarios() {
        let workspace = Workspace::new(FORMAT, &format!("{scenario}-{platform}-native-"))?;
        measure::consumer(
            &mut scorecard,
            scenario,
            &expected,
            &NativeReader,
            &workspace.path,
        )?;
    }
    write_report(
        FORMAT,
        &format!("native-{platform}-accuracy.txt"),
        &scorecard.report(),
    )
}
