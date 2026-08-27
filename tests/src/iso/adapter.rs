use std::path::Path;

use super::model::IsoState;

/// An implementation that can author an ISO 9660 image from a semantic tree.
pub trait IsoProducer {
    fn name(&self) -> String;

    /// Writes `state` to `image`. `workspace` is a scratch directory the
    /// producer may use for source trees or intermediate files.
    fn produce(&self, state: &IsoState, workspace: &Path, image: &Path) -> Result<(), String>;
}

/// An implementation that can read an ISO 9660 image back into the model.
pub trait IsoConsumer {
    fn name(&self) -> String;

    /// Reads `image`. `workspace` is a scratch directory the consumer may use
    /// for extraction targets or mountpoints.
    fn snapshot(&self, image: &Path, workspace: &Path) -> Result<IsoState, String>;
}
