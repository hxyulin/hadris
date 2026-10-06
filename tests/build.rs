use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    let output = Command::new(std::env::var_os("RUSTC").unwrap())
        .arg("-Vv")
        .output()
        .expect("query benchmark compiler");
    assert!(output.status.success());
    let version = String::from_utf8(output.stdout)
        .unwrap()
        .replace('\n', "; ");
    println!("cargo:rustc-env=HADRIS_TESTS_COMPILER={version}");
    println!(
        "cargo:rustc-env=HADRIS_TESTS_PROFILE={}",
        std::env::var("PROFILE").unwrap()
    );
}
