//! Errors print their whole cause chain for every format.

#[test]
fn missing_image_errors_include_the_os_cause() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("missing.img");
    let prefix = format!("Error: cannot open {}: ", image.display());
    let mut messages = Vec::new();
    for format in ["fat", "iso", "udf"] {
        for command in ["ls", "info"] {
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
                .args([format, command])
                .arg(&image)
                .output()
                .unwrap();
            assert!(!output.status.success());
            let stderr = String::from_utf8(output.stderr).unwrap();
            let cause = stderr.strip_prefix(&prefix).unwrap_or_else(|| {
                panic!("{format} {command}: {stderr}");
            });
            assert!(!cause.trim().is_empty(), "{format} {command}: {stderr}");
            messages.push(stderr);
        }
    }
    messages.dedup();
    assert_eq!(messages.len(), 1, "{messages:?}");
}
