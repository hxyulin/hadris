//! Qualification against native exFAT tools: exfatprogs `mkfs.exfat` and
//! `fsck.exfat`, macOS `newfs_exfat` and `fsck_exfat`, and the macOS kernel
//! driver.

use std::path::Path;

use hadris_tests::exfat::generic::{self, HadrisExFatAdapter};
use hadris_tests::exfat::native::{self, Exfatprogs};
use hadris_tests::exfat::scenarios::exfat_scenarios;
use hadris_tests::exfat::{EXFAT_CASES, FORMAT, spec};
use hadris_tests::fat::scenarios::{curated_operations, interoperability_scenarios};
use hadris_tests::fat::{
    FatAdapter, Operation, apply_operations, clear_mutable_attrs, compare_snapshot, format_trace,
};
use hadris_tests::harness::command::REQUIRE_TOOLS_ENV;
use hadris_tests::harness::{NATIVE_MOUNT_ENV, Workspace, native_mount_enabled};

/// The native checkers present, or `None` when there are none and the
/// test should be skipped.
fn checkers() -> Option<(Option<Exfatprogs>, bool)> {
    let exfatprogs = Exfatprogs::find();
    let macos = native::macos_available();
    if exfatprogs.is_none() && !macos {
        if std::env::var_os(REQUIRE_TOOLS_ENV).is_some() {
            panic!("no native exFAT checker is available");
        }
        eprintln!("skipping: no native exFAT checker is available");
        return None;
    }
    Some((exfatprogs, macos))
}

fn fsck_all(image: &Path, exfatprogs: Option<Exfatprogs>, macos: bool) -> Result<(), String> {
    if let Some(tools) = exfatprogs {
        tools
            .fsck(image)
            .map_err(|error| format!("fsck.exfat: {error}"))?;
    }
    let sector_shift = std::fs::read(image).map_err(|error| error.to_string())?[108];
    if macos && sector_shift == 9 {
        native::fsck_macos(image).map_err(|error| format!("fsck_exfat: {error}"))?;
    }
    Ok(())
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
fn native_checkers_accept_hadris_images() {
    let Some((exfatprogs, macos)) = checkers() else {
        return;
    };
    let mut scenarios = interoperability_scenarios();
    scenarios.extend(exfat_scenarios());
    let mut failures = Vec::new();
    for (name, operations) in scenarios {
        for case in EXFAT_CASES {
            let outcome = (|| {
                let workspace = Workspace::new(FORMAT, &format!("{}-fsck-", case.name))?;
                let image = workspace.path.join("hadris.img");
                generic::format(&image, case)?;
                apply_operations(&mut HadrisExFatAdapter::new(image.clone()), &operations)?;
                fsck_all(&image, exfatprogs, macos)
            })();
            if let Err(error) = outcome {
                failures.push(format!("{name} {}: {error}", case.name));
            }
        }
    }
    report(failures);
}

fn hadris_writes(
    image: &Path,
    operations: &[Operation],
    exfatprogs: Option<Exfatprogs>,
    macos: bool,
) -> Result<(), String> {
    let mut formatted = spec::snapshot(image)?;
    formatted.label = hadris_tests::fat::LABEL.into();
    let expected = apply_operations(
        &mut HadrisExFatAdapter::new(image.to_path_buf()),
        operations,
    )?;
    fsck_all(image, exfatprogs, macos)?;
    compare_snapshot("specification oracle", &expected, &spec::snapshot(image)?)?;
    compare_snapshot(
        "Hadris reader",
        &expected,
        &HadrisExFatAdapter::new(image.to_path_buf()).snapshot()?,
    )
}

#[test]
fn hadris_writes_natively_formatted_volumes() {
    let Some((exfatprogs, macos)) = checkers() else {
        return;
    };
    let operations = curated_operations();
    let mut failures = Vec::new();
    if let Some(tools) = exfatprogs {
        for case in EXFAT_CASES {
            let outcome = (|| {
                let workspace = Workspace::new(FORMAT, &format!("{}-mkfs-", case.name))?;
                let image = workspace.path.join("mkfs.img");
                tools.format(&image, case)?;
                hadris_writes(&image, &operations, exfatprogs, macos)
            })();
            if let Err(error) = outcome {
                failures.push(format!("mkfs.exfat {}: {error}", case.name));
            }
        }
    }
    if macos {
        for size in [8u64 << 20, 64 << 20] {
            let outcome = (|| {
                let workspace = Workspace::new(FORMAT, "newfs-")?;
                let image = workspace.path.join("newfs.img");
                native::newfs(&image, size)?;
                hadris_writes(&image, &operations, exfatprogs, macos)
            })();
            if let Err(error) = outcome {
                failures.push(format!("newfs_exfat {size}: {error}"));
            }
        }
    }
    if !failures.is_empty() {
        panic!(
            "{}\ntrace:\n{}",
            failures.join("\n\n"),
            format_trace(&operations)
        );
    }
}

fn run_native_mount() -> Result<(), String> {
    for case in EXFAT_CASES {
        let workspace = Workspace::new(FORMAT, &format!("{}-mount-", case.name))?;
        let image = workspace.path.join("native-mount.img");
        generic::format(&image, case)?;
        let mut expected = apply_operations(
            &mut HadrisExFatAdapter::new(image.clone()),
            &curated_operations(),
        )?;
        let mount = native::mount(&image, &workspace.path.join("mount"))?;
        for (path, entry) in &expected.entries {
            let host = mount.path().join(path.trim_start_matches('/'));
            if let hadris_tests::harness::EntryData::File(data) = &entry.data {
                let read = std::fs::read(&host).map_err(|error| format!("{path}: {error}"))?;
                if &read != data {
                    return Err(format!("{} kernel read {path} differently", case.name));
                }
            }
        }
        let dir = mount.path().join("Kernel Directory");
        std::fs::create_dir(&dir).map_err(|error| error.to_string())?;
        std::fs::write(
            dir.join("Written by Kernel.txt"),
            format!("native-{}", case.name),
        )
        .map_err(|error| error.to_string())?;
        std::fs::remove_file(mount.path().join("README.TXT")).map_err(|error| error.to_string())?;
        mount.unmount()?;
        expected.apply(&Operation::CreateDir {
            path: "/Kernel Directory".into(),
        })?;
        expected.apply(&Operation::CreateFile {
            path: "/Kernel Directory/Written by Kernel.txt".into(),
            data: format!("native-{}", case.name).into_bytes(),
        })?;
        expected.apply(&Operation::Delete {
            path: "/README.TXT".into(),
        })?;
        clear_mutable_attrs(&mut expected);
        for (what, mut actual) in [
            ("specification oracle", spec::snapshot(&image)?),
            (
                "Hadris reader",
                HadrisExFatAdapter::new(image.clone()).snapshot()?,
            ),
        ] {
            clear_mutable_attrs(&mut actual);
            actual.entries.retain(|path, _| {
                !path
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.starts_with("._"))
            });
            compare_snapshot(
                &format!("{} {what} after the kernel", case.name),
                &expected,
                &actual,
            )?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires HADRIS_TESTS_NATIVE_MOUNT=1 and macOS"]
fn native_mount_roundtrip() {
    if !native_mount_enabled() {
        eprintln!("set {NATIVE_MOUNT_ENV}=1 to enable native kernel mount tests");
        return;
    }
    run_native_mount().unwrap();
}
