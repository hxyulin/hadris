//! Every read command against one image built by `create`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hadris-fat"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout_of(args: &[&str]) -> String {
    let output = run(args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Host names in `dir`, as the host stores them.
fn host_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

struct Fixture {
    temp: tempfile::TempDir,
    image: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(source.join("Sub/deep")).unwrap();
        std::fs::write(source.join("top.txt"), b"top").unwrap();
        std::fs::write(source.join("b"), b"b").unwrap();
        std::fs::write(source.join("a"), b"a").unwrap();
        std::fs::write(source.join("Sub/Inner.TXT"), b"inner").unwrap();
        std::fs::write(source.join("Sub/deep/data.bin"), vec![7u8; 10_000]).unwrap();
        let image = temp.path().join("disk.img");
        let output = run(&["create", text(&source), "--output", text(&image)]);
        assert!(output.status.success(), "{output:?}");
        Self { temp, image }
    }

    fn image(&self) -> &str {
        text(&self.image)
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
}

#[test]
fn listing_commands() {
    let fx = Fixture::new();

    assert_eq!(
        stdout_of(&["ls", fx.image()]),
        "Sub/\na\nb\ntop.txt\n",
        "create imports in name order"
    );
    let long = stdout_of(&["ls", "--long", fx.image(), "/sub"]);
    assert!(long.contains("<DIR>  deep"), "{long}");
    assert!(long.contains("5  Inner.TXT"), "{long}");

    let tree = stdout_of(&["tree", fx.image()]);
    assert!(tree.starts_with("/\n"), "{tree}");
    assert!(tree.contains("├── Sub/\n"), "{tree}");
    assert!(tree.contains("│   └── deep/\n"), "{tree}");
    assert!(tree.contains("│       └── data.bin\n"), "{tree}");
    assert!(tree.contains("└── top.txt\n"), "{tree}");
    let shallow = stdout_of(&["tree", "--depth", "1", fx.image()]);
    assert!(
        shallow.contains("Sub/") && !shallow.contains("Inner.TXT"),
        "{shallow}"
    );

    let output = run(&["ls", fx.image(), "/missing"]);
    assert!(!output.status.success());
}

#[test]
fn report_commands() {
    let fx = Fixture::new();

    let info = stdout_of(&["info", fx.image()]);
    assert!(info.contains("Volume Label:    HADRIS"), "{info}");
    let stat = stdout_of(&["stat", fx.image()]);
    assert!(stat.contains("Files:             5"), "{stat}");
    let verify = stdout_of(&["verify", "--verbose", fx.image()]);
    assert!(verify.contains("Result: PASS"), "{verify}");

    let chain = stdout_of(&["chain", fx.image(), "/Sub/deep/data.bin"]);
    assert!(chain.contains("File size: 10000 bytes"), "{chain}");
    assert!(chain.contains("Fragments: 1"), "{chain}");
    let small = stdout_of(&["chain", fx.image(), "/top.txt"]);
    assert!(small.contains("Chain length: 1 clusters"), "{small}");

    let fragmentation = stdout_of(&["fragmentation", fx.image()]);
    assert!(
        fragmentation.contains("Total Files:             5"),
        "{fragmentation}"
    );
}

#[test]
fn extract_uses_the_stored_name() {
    let fx = Fixture::new();

    let out = fx.dir("file");
    stdout_of(&[
        "extract",
        fx.image(),
        "-o",
        text(&out),
        "-p",
        "/SUB/inner.txt",
    ]);
    assert_eq!(host_names(&out), ["Inner.TXT"]);
    assert_eq!(std::fs::read(out.join("Inner.TXT")).unwrap(), b"inner");

    let out = fx.dir("dir");
    stdout_of(&["extract", fx.image(), "-o", text(&out), "-p", "/sub/"]);
    assert_eq!(host_names(&out), ["Sub"]);
    assert_eq!(host_names(&out.join("Sub")), ["Inner.TXT", "deep"]);
    assert_eq!(
        std::fs::read(out.join("Sub/deep/data.bin")).unwrap(),
        vec![7u8; 10_000]
    );
}

#[test]
fn extract_stays_inside_the_output_directory() {
    let fx = Fixture::new();
    let nested = fx.dir("nested");
    let out = nested.join("out");

    stdout_of(&["extract", fx.image(), "-o", text(&out), "-p", "/Sub/.."]);
    assert_eq!(host_names(&nested), ["out"]);
    assert_eq!(host_names(&out), ["Sub", "a", "b", "top.txt"]);

    let out = fx.dir("dotted");
    stdout_of(&[
        "extract",
        fx.image(),
        "-o",
        text(&out),
        "-p",
        "/Sub/deep/../.",
    ]);
    assert_eq!(host_names(&out), ["Sub"]);

    let output = run(&["extract", fx.image(), "-o", text(&out), "-p", "/missing"]);
    assert!(!output.status.success());
}
