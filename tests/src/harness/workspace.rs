use std::fs;
use std::path::PathBuf;

use tempfile::{Builder as TempBuilder, TempDir};

/// Set to retain images and peer artifacts under the report directory.
pub const KEEP_ENV: &str = "HADRIS_TESTS_KEEP";
/// Overrides the report root, which defaults to `tests/target/reports`.
pub const REPORT_DIR_ENV: &str = "HADRIS_TESTS_REPORT_DIR";

pub fn report_dir(format: &str) -> Result<PathBuf, String> {
    let base = match std::env::var_os(REPORT_DIR_ENV) {
        Some(directory) => PathBuf::from(directory),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("reports"),
    };
    let directory = base.join(format);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

/// Prints a report and writes it to `<report dir>/<format>/<name>`.
pub fn write_report(format: &str, name: &str, report: &str) -> Result<(), String> {
    eprintln!("{report}");
    fs::write(report_dir(format)?.join(name), format!("{report}\n"))
        .map_err(|error| error.to_string())
}

/// A scratch directory that is deleted on drop unless [`KEEP_ENV`] is set.
pub struct Workspace {
    _temp: Option<TempDir>,
    pub path: PathBuf,
}

impl Workspace {
    pub fn new(format: &str, prefix: &str) -> Result<Self, String> {
        if std::env::var_os(KEEP_ENV).is_some() {
            let root = report_dir(format)?.join("artifacts");
            fs::create_dir_all(&root).map_err(|error| error.to_string())?;
            let temp = TempBuilder::new()
                .prefix(prefix)
                .tempdir_in(root)
                .map_err(|error| error.to_string())?;
            let path = temp.keep();
            eprintln!("retaining {format} artifacts at {}", path.display());
            Ok(Self { _temp: None, path })
        } else {
            let temp = TempBuilder::new()
                .prefix(prefix)
                .tempdir()
                .map_err(|error| error.to_string())?;
            let path = temp.path().to_path_buf();
            Ok(Self {
                _temp: Some(temp),
                path,
            })
        }
    }
}
