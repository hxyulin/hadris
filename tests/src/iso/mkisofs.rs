//! The original cdrtools `mkisofs`, or its `genisoimage` fork, as a producer.

use std::path::Path;

use super::adapter::IsoProducer;
use super::model::IsoState;
use crate::harness::command::{program_available, run_command};

pub const CANDIDATES: [&str; 2] = ["mkisofs", "genisoimage"];

pub struct Mkisofs {
    program: &'static str,
}

impl Mkisofs {
    /// Returns the first installed candidate, if any.
    pub fn detect() -> Option<Self> {
        CANDIDATES
            .into_iter()
            .find(|program| program_available(program, "-version"))
            .map(|program| Self { program })
    }
}

impl IsoProducer for Mkisofs {
    fn name(&self) -> String {
        self.program.to_string()
    }

    fn produce(&self, state: &IsoState, workspace: &Path, image: &Path) -> Result<(), String> {
        let source = workspace.join(format!("{}-source", self.program));
        state.write_host(&source)?;
        run_command(
            self.program,
            vec![
                "-iso-level".into(),
                "1".into(),
                "-V".into(),
                state.volume_id.as_str().into(),
                "-o".into(),
                image.as_os_str().into(),
                source.as_os_str().into(),
            ],
        )
        .map(|_| ())
    }
}
