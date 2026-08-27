//! ISO 9660 conformance model, oracle, scenarios, and implementation adapters.

pub mod adapter;
pub mod hadris;
pub mod measure;
pub mod mkisofs;
pub mod model;
pub mod native;
pub mod spec;
pub mod xorriso;

pub use adapter::{IsoConsumer, IsoProducer};
pub use model::{
    IsoState, compare_entries, compare_state, conformance_scenarios, strip_path_versions,
    strip_version,
};

/// Report directory name under the harness report root.
pub const FORMAT: &str = "iso";
pub const SECTOR_SIZE: usize = 2048;
pub const VOLUME_ID: &str = "HADRISCONF";
