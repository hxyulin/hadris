use hadris_fs::{PathError, Report, Tree};

use super::storage::BlockDevice;
use crate::options::CdOptions;

io_transform! {

/// Writes `tree` as a hybrid ISO 9660 and UDF image on `out`, as `opts`
/// says, and returns the report [`plan`](crate::plan) returns.
///
/// This is `hadris_udf`'s `write_bridge` with the two option sets of
/// `opts`: the ISO 9660 image is written first, then the UDF structures
/// pointing at its file extents. Errors of either writer keep their kind,
/// detail code and path.
pub async fn write<D: BlockDevice>(out: D, tree: &Tree, opts: &CdOptions) -> Result<Report, PathError> {
    super::udf::write_bridge(out, tree, opts.iso(), opts.udf()).await
}

}
