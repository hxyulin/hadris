//! The `read_only` form of `impl_fs_driver!` keeps the write defaults.

#![cfg(feature = "sync")]

use core::convert::Infallible;

use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats, Metadata, Name,
    NameBuf, NewNode, NodeId, SetMetadata,
};

struct Empty;

impl Empty {
    fn capabilities(&self) -> Capabilities {
        Capabilities::new()
    }

    fn root(&self) -> NodeId {
        NodeId::new(1)
    }

    fn lookup(&mut self, _: NodeId, _: &Name) -> FsResult<NodeId, Infallible> {
        Err(ErrorKind::NotFound.into())
    }

    fn node_metadata(&mut self, _: NodeId) -> FsResult<Metadata, Infallible> {
        Ok(Metadata::new(FileType::Dir))
    }

    fn read_dir_entry(
        &mut self,
        _: NodeId,
        _: &mut DirCursor,
        _: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, Infallible> {
        Ok(None)
    }

    fn read_at(&mut self, _: NodeId, _: u64, _: &mut [u8]) -> FsResult<usize, Infallible> {
        Ok(0)
    }

    fn stats(&mut self) -> FsResult<FsStats, Infallible> {
        Ok(FsStats::new(0, 0, 512))
    }

    fn forget(&mut self, _: NodeId) {}
}

hadris_fs::impl_fs_driver!(sync, impl[] Empty, error = Infallible, read_only);

#[test]
fn read_only_form_keeps_write_defaults() {
    let mut fs = Empty;
    let root = FsDriver::root(&fs);
    let err = FsDriver::create(
        &mut fs,
        root,
        Name::new("x").unwrap(),
        NewNode::File,
        &SetMetadata::new(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ReadOnly);
    assert_eq!(
        FsDriver::parent(&mut fs, root).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        fs.write_file("/x", b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
}
