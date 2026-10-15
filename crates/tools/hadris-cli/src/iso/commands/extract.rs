use anyhow::Result;
use hadris_iso::sync::IsoFs;

use super::super::args::ExtractArgs;

use super::view_for;
use crate::vfs::{Extraction, Wording};

/// Extract files from an ISO image. The root is merged into the output
/// directory; any other path lands at `<output>/<name>`.
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut iso = args.locate.open(&args.input)?;
    let from = args.path.as_deref().unwrap_or("/");
    let (namespace, plan) = {
        let mut view = view_for(&mut iso, from)?;
        let plan = Extraction::plan(&mut view, from, &args.output, Wording::Optical)?;
        (view.namespace(), plan)
    };
    if args.verbose {
        println!(
            "Extracting {from} from the {namespace:?} tree to {}",
            plan.destination.display()
        );
    }
    let fs = IsoFs::mount_namespace(iso, hadris_fs::MountOptions::new(), namespace)
        .map_err(|err| err.into_parts().0)?;
    plan.run(fs)?;
    println!("Extracted {from} to {}", plan.destination.display());
    Ok(())
}
