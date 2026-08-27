//! Format-agnostic test infrastructure shared by every suite.

pub mod command;
pub mod mount;
pub mod path;
pub mod qemu;
pub mod rng;
pub mod scorecard;
pub mod tree;
pub mod workspace;

pub use command::{command_value, program_available, require_or_skip, run_command};
pub use mount::NativeMount;
pub use path::{join_path, normalize_path, path_depth, split_parent};
pub use rng::Rng;
pub use scorecard::Scorecard;
pub use tree::{EntryData, fnv1a};
pub use workspace::{Workspace, report_dir, write_report};

/// Environment variable that opts into privileged native kernel mounts.
pub const NATIVE_MOUNT_ENV: &str = "HADRIS_TESTS_NATIVE_MOUNT";

pub fn native_mount_enabled() -> bool {
    std::env::var(NATIVE_MOUNT_ENV).as_deref() == Ok("1")
}

/// Runs a peer implementation and converts a panic into an ordinary error so
/// one crashing peer does not abort the whole measurement.
pub fn catch_panic<T>(
    peer: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic payload");
        format!("{peer} panicked: {message}")
    })?
}
