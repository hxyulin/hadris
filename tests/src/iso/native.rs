//! The host platform's own ISO producer and kernel ISO 9660 reader.

use std::path::Path;

use super::adapter::{IsoConsumer, IsoProducer};
use super::model::{IsoState, strip_path_versions};
use crate::harness::mount::NativeMount;
use crate::harness::run_command;

/// macOS `hdiutil makehybrid`; every other platform has no built-in producer.
pub struct Hdiutil;

impl IsoProducer for Hdiutil {
    fn name(&self) -> String {
        "macOS hdiutil".to_string()
    }

    fn produce(&self, state: &IsoState, workspace: &Path, image: &Path) -> Result<(), String> {
        if !cfg!(target_os = "macos") {
            return Err("the native platform has no built-in ISO producer".to_string());
        }
        let source = workspace.join("hdiutil-source");
        state.write_host(&source)?;
        run_command(
            "hdiutil",
            vec![
                "makehybrid".into(),
                "-o".into(),
                image.as_os_str().into(),
                source.as_os_str().into(),
                "-iso".into(),
                "-iso-volume-name".into(),
                state.volume_id.as_str().into(),
                "-ov".into(),
            ],
        )
        .map(|_| ())
    }
}

/// The kernel ISO 9660 driver: Linux `mount`, macOS `hdiutil attach`, or
/// Windows `Mount-DiskImage`.
pub struct NativeReader;

impl IsoConsumer for NativeReader {
    fn name(&self) -> String {
        format!("{} native ISO reader", std::env::consts::OS)
    }

    fn snapshot(&self, image: &Path, workspace: &Path) -> Result<IsoState, String> {
        let mountpoint = workspace.join("mount");
        let mount = if cfg!(target_os = "linux") {
            NativeMount::linux(image, &mountpoint, None, "loop,ro,map=off")?
        } else if cfg!(target_os = "macos") {
            NativeMount::macos(image, &mountpoint, &["-readonly", "-nobrowse"])?
        } else if cfg!(target_os = "windows") {
            NativeMount::windows(image)?
        } else {
            return Err(
                "native ISO mounting is supported on Linux, macOS, and Windows".to_string(),
            );
        };
        let state = IsoState::from_host(mount.path()).map(strip_path_versions);
        let unmount = mount.unmount();
        match (state, unmount) {
            (Ok(state), Ok(())) => Ok(state),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
}
