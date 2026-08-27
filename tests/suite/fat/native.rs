//! The host platform's FAT formatter, checker, and kernel driver.

use hadris_tests::fat::hadris::{self, HadrisFatAdapter};
use hadris_tests::fat::scenarios::curated_operations;
use hadris_tests::fat::{
    FAT_CASES, FORMAT, FatAdapter, Operation, apply_operations, clear_mutable_attrs,
    compare_snapshot, fatfs, format_trace, native, spec,
};
use hadris_tests::harness::{NATIVE_MOUNT_ENV, Workspace, native_mount_enabled};

fn run_native_tools_matrix(operations: &[Operation]) -> Result<(), String> {
    native::tools()?;
    for case in FAT_CASES {
        let workspace = Workspace::new(FORMAT, &format!("{}-native-", case.name))?;

        let hadris_image = workspace.path.join("hadris.img");
        hadris::format(&hadris_image, case)?;
        let expected =
            apply_operations(&mut HadrisFatAdapter::new(hadris_image.clone()), operations)?;
        native::fsck(&hadris_image)?;
        compare_snapshot(
            &format!("{} native checker input", case.name),
            &expected,
            &fatfs::snapshot(&hadris_image)?,
        )?;

        let native_image = workspace.path.join("native.img");
        native::format(&native_image, case)?;
        let expected =
            apply_operations(&mut HadrisFatAdapter::new(native_image.clone()), operations)?;
        native::fsck(&native_image)?;
        let oracle = spec::snapshot(&native_image, case.bits)?;
        compare_snapshot(
            &format!("{} specification oracle reading native format", case.name),
            &expected,
            &oracle,
        )?;
        compare_snapshot(
            &format!("{} rust-fatfs reading native format", case.name),
            &expected,
            &fatfs::snapshot(&native_image)?,
        )?;
    }
    Ok(())
}

fn run_native_mount_matrix() -> Result<(), String> {
    if !native_mount_enabled() {
        eprintln!("set {NATIVE_MOUNT_ENV}=1 to enable native kernel mount tests");
        return Ok(());
    }
    for case in FAT_CASES {
        let workspace = Workspace::new(FORMAT, &format!("{}-mount-", case.name))?;
        let image = workspace.path.join("native-mount.img");
        hadris::format(&image, case)?;
        let base = vec![
            Operation::CreateDir {
                path: "/Hadris Source".into(),
            },
            Operation::CreateFile {
                path: "/Hadris Source/Original.txt".into(),
                data: b"written by Hadris".to_vec(),
            },
        ];
        let mut expected = apply_operations(&mut HadrisFatAdapter::new(image.clone()), &base)?;
        let mount = native::mount(&image, &workspace.path.join("mount"))?;
        let original = mount.path().join("Hadris Source/Original.txt");
        if std::fs::read(&original).map_err(|error| error.to_string())? != b"written by Hadris" {
            return Err(format!(
                "{} native mount read incorrect contents",
                case.name
            ));
        }
        let native_dir = mount.path().join("Kernel Directory");
        std::fs::create_dir(&native_dir).map_err(|error| error.to_string())?;
        let temporary = mount.path().join("Temporary.txt");
        std::fs::write(&temporary, b"delete me").map_err(|error| error.to_string())?;
        let renamed = native_dir.join("Renamed by Kernel.txt");
        std::fs::write(&renamed, format!("native-{}", case.name))
            .map_err(|error| error.to_string())?;
        std::fs::remove_file(temporary).map_err(|error| error.to_string())?;
        mount.unmount()?;

        expected.apply(&Operation::CreateDir {
            path: "/Kernel Directory".into(),
        })?;
        expected.apply(&Operation::CreateFile {
            path: "/Kernel Directory/Renamed by Kernel.txt".into(),
            data: format!("native-{}", case.name).into_bytes(),
        })?;
        native::fsck(&image)?;
        let mut oracle = spec::snapshot(&image, case.bits)?;
        clear_mutable_attrs(&mut expected);
        clear_mutable_attrs(&mut oracle);
        native::remove_native_metadata(&mut oracle);
        compare_snapshot(
            &format!("{} specification oracle reading native mount", case.name),
            &expected,
            &oracle,
        )?;
        let mut hadris = HadrisFatAdapter::new(image).snapshot()?;
        clear_mutable_attrs(&mut hadris);
        native::remove_native_metadata(&mut hadris);
        compare_snapshot(
            &format!("{} Hadris reading native mount", case.name),
            &expected,
            &hadris,
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "requires native Linux or macOS FAT formatter and checker"]
fn native_platform_tools() {
    let operations = curated_operations();
    run_native_tools_matrix(&operations)
        .unwrap_or_else(|error| panic!("{error}\ntrace:\n{}", format_trace(&operations)));
}

#[test]
#[ignore = "requires HADRIS_TESTS_NATIVE_MOUNT=1 and native mount privileges"]
fn native_mount_roundtrip() {
    run_native_mount_matrix().unwrap();
}
