//! Flags and helpers every format's commands share.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use hadris_fs::host::{self, OnError, TreeOptions};
use hadris_fs::{Clock, DateTime, SystemClock, Tree};
use hadris_udf::UdfRevision;

use crate::output::Output;

/// Where a `create` command writes its image or archive.
#[derive(Debug, Clone, clap::Args)]
pub struct Target {
    /// Output path; an existing file or device is refused without --force
    #[arg(short, long)]
    pub output: PathBuf,
    /// Replace an existing output file, or write over an existing device
    #[arg(short, long)]
    pub force: bool,
}

impl Target {
    /// Starts writing the output under the shared overwrite rule.
    pub fn create(&self) -> Result<(std::fs::File, Output)> {
        Output::create(&self.output, self.force)
            .with_context(|| format!("cannot create {}", self.output.display()))
    }
}

/// Reads the host directory `source` into a tree. Entries that cannot be
/// read are skipped with a warning.
pub fn read_source(source: &Path) -> Result<Tree> {
    let meta = std::fs::metadata(source)
        .with_context(|| format!("cannot read source {}", source.display()))?;
    if !meta.is_dir() {
        bail!("source is not a directory: {}", source.display());
    }
    let (tree, skipped) = host::read_tree(source, &TreeOptions::new().with_on_error(OnError::Skip))
        .with_context(|| format!("cannot read source {}", source.display()))?;
    for err in skipped {
        eprintln!("warning: skipped {err}");
    }
    Ok(tree)
}

/// The time new images are dated with: `SOURCE_DATE_EPOCH` when set, the
/// system time otherwise.
pub fn build_time() -> Result<DateTime> {
    Ok(host::source_date_epoch()?.unwrap_or_else(|| SystemClock.now()))
}

/// A UDF revision given as `1.02`, `1.50`, `2.00`, `2.01`, `2.50` or `2.60`.
#[derive(Debug, Clone, Copy)]
pub struct RevisionArg(pub UdfRevision);

impl FromStr for RevisionArg {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let revision = match value {
            "1.02" => UdfRevision::V1_02,
            "1.50" => UdfRevision::V1_50,
            "2.00" => UdfRevision::V2_00,
            "2.01" => UdfRevision::V2_01,
            "2.50" => UdfRevision::V2_50,
            "2.60" => UdfRevision::V2_60,
            _ => return Err("expected 1.02, 1.50, 2.00, 2.01, 2.50 or 2.60"),
        };
        Ok(Self(revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_parse_only_known_values() {
        assert_eq!("1.02".parse::<RevisionArg>().unwrap().0, UdfRevision::V1_02);
        assert_eq!("2.60".parse::<RevisionArg>().unwrap().0, UdfRevision::V2_60);
        for input in ["9.99", "1.03", "2.5", "abc", ""] {
            assert!(input.parse::<RevisionArg>().is_err(), "{input:?}");
        }
    }
}
