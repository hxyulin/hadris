//! Every wrapper forwards every trait method (R10). The list of methods is
//! read from the trait definitions themselves, so a method added to
//! `FsDriver` or `FileSystem` fails here until every wrapper forwards it.

#![cfg(all(feature = "sync", feature = "std"))]

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use hadris_fs::sync::{AsDriver, FileSystem, FsDriver, Lexical, StdMutex, Volume, WithResolver};
use hadris_fs::{
    Capabilities, DirCursor, DirEntry, FileType, FsResult, FsStats, Metadata, Name, NameBuf,
    NewNode, NodeId, RemoveKind, RenameFlags, SetMetadata,
};

type Calls = BTreeSet<&'static str>;

/// The methods declared in `pub trait <name>` of the driver source.
fn declared(name: &str) -> Calls {
    let source = include_str!("../src/api/driver.rs");
    let start = source
        .find(&format!("pub trait {name} {{"))
        .expect("trait is defined");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("trait ends")];
    body.split("fn ")
        .skip(1)
        .filter_map(|rest| rest.split('(').next())
        .filter(|name| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(|name| &*String::from(name).leak())
        .collect()
}

#[derive(Default)]
struct Recorder {
    calls: Calls,
}

const ROOT: NodeId = NodeId::new(1);
const NODE: NodeId = NodeId::new(2);

impl FsDriver for Recorder {
    type DeviceError = std::io::Error;

    fn capabilities(&self) -> Capabilities {
        Capabilities::new().with_writable()
    }
    fn root(&self) -> NodeId {
        ROOT
    }
    fn lookup(&mut self, _: NodeId, _: &Name) -> FsResult<NodeId, Self::DeviceError> {
        self.calls.insert("lookup");
        Ok(NODE)
    }
    fn node_metadata(&mut self, _: NodeId) -> FsResult<Metadata, Self::DeviceError> {
        self.calls.insert("node_metadata");
        Ok(Metadata::new(FileType::File))
    }
    fn read_dir_entry(
        &mut self,
        _: NodeId,
        _: &mut DirCursor,
        _: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, Self::DeviceError> {
        self.calls.insert("read_dir_entry");
        Ok(None)
    }
    fn read_at(&mut self, _: NodeId, _: u64, _: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        self.calls.insert("read_at");
        Ok(0)
    }
    fn stats(&mut self) -> FsResult<FsStats, Self::DeviceError> {
        self.calls.insert("stats");
        Ok(FsStats::new(0, 0, 512))
    }
    fn forget(&mut self, _: NodeId) {
        self.calls.insert("forget");
    }
    fn open_node(&mut self, _: NodeId) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("open_node");
        Ok(())
    }
    fn close_node(&mut self, _: NodeId) {
        self.calls.insert("close_node");
    }
    fn parent(&mut self, _: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        self.calls.insert("parent");
        Ok(ROOT)
    }
    fn read_link(&mut self, _: NodeId, _: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        self.calls.insert("read_link");
        Ok(0)
    }
    fn resolve(&mut self, _: &str) -> FsResult<NodeId, Self::DeviceError> {
        self.calls.insert("resolve");
        Ok(ROOT)
    }
    fn create(
        &mut self,
        _: NodeId,
        _: &Name,
        _: NewNode<'_>,
        _: &SetMetadata,
    ) -> FsResult<NodeId, Self::DeviceError> {
        self.calls.insert("create");
        Ok(NODE)
    }
    fn remove(&mut self, _: NodeId, _: &Name, _: RemoveKind) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("remove");
        Ok(())
    }
    fn rename(
        &mut self,
        _: NodeId,
        _: &Name,
        _: NodeId,
        _: &Name,
        _: RenameFlags,
    ) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("rename");
        Ok(())
    }
    fn write_at(&mut self, _: NodeId, _: u64, _: &[u8]) -> FsResult<usize, Self::DeviceError> {
        self.calls.insert("write_at");
        Ok(0)
    }
    fn set_len(&mut self, _: NodeId, _: u64) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("set_len");
        Ok(())
    }
    fn set_metadata(&mut self, _: NodeId, _: &SetMetadata) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("set_metadata");
        Ok(())
    }
    fn sync_node(&mut self, _: NodeId) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("sync_node");
        Ok(())
    }
    fn publish_node(&mut self, _: NodeId) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("publish_node");
        Ok(())
    }
    fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
        self.calls.insert("sync");
        Ok(())
    }
}

fn name() -> &'static Name {
    Name::new("x").unwrap()
}

/// Calls every `FsDriver` method, including the synchronous ones, whose
/// calls a wrapper answers without the driver (`capabilities`, `root`).
fn every_driver_method<D: FsDriver + ?Sized>(d: &mut D) -> Calls {
    let mut direct = Calls::new();
    let _ = d.capabilities();
    let _ = d.root();
    direct.extend(["capabilities", "root"]);
    d.lookup(ROOT, name()).unwrap();
    d.node_metadata(NODE).unwrap();
    d.read_dir_entry(ROOT, &mut DirCursor::start(), &mut NameBuf::new())
        .unwrap();
    d.read_at(NODE, 0, &mut [0; 4]).unwrap();
    d.stats().unwrap();
    d.forget(NODE);
    d.open_node(NODE).unwrap();
    d.close_node(NODE);
    d.parent(NODE).unwrap();
    d.read_link(NODE, &mut [0; 4]).unwrap();
    d.resolve("/").unwrap();
    d.create(ROOT, name(), NewNode::File, &SetMetadata::new())
        .unwrap();
    d.remove(ROOT, name(), RemoveKind::Any).unwrap();
    d.rename(ROOT, name(), ROOT, name(), RenameFlags::empty())
        .unwrap();
    d.write_at(NODE, 0, b"x").unwrap();
    d.set_len(NODE, 0).unwrap();
    d.set_metadata(NODE, &SetMetadata::new()).unwrap();
    d.sync_node(NODE).unwrap();
    d.publish_node(NODE).unwrap();
    d.sync().unwrap();
    direct
}

fn every_fs_method<F: FileSystem + ?Sized>(f: &F) -> Calls {
    let mut direct = Calls::new();
    let _ = f.capabilities();
    let _ = f.root();
    direct.extend(["capabilities", "root"]);
    f.lookup(ROOT, name()).unwrap();
    f.node_metadata(NODE).unwrap();
    f.read_dir_entry(ROOT, &mut DirCursor::start(), &mut NameBuf::new())
        .unwrap();
    f.read_at(NODE, 0, &mut [0; 4]).unwrap();
    f.stats().unwrap();
    f.forget(NODE);
    f.open_node(NODE).unwrap();
    f.close_node(NODE);
    f.parent(NODE).unwrap();
    f.read_link(NODE, &mut [0; 4]).unwrap();
    f.resolve("/").unwrap();
    f.create(ROOT, name(), NewNode::File, &SetMetadata::new())
        .unwrap();
    f.remove(ROOT, name(), RemoveKind::Any).unwrap();
    f.rename(ROOT, name(), ROOT, name(), RenameFlags::empty())
        .unwrap();
    f.write_at(NODE, 0, b"x").unwrap();
    f.set_len(NODE, 0).unwrap();
    f.set_metadata(NODE, &SetMetadata::new()).unwrap();
    f.sync_node(NODE).unwrap();
    f.publish_node(NODE).unwrap();
    f.sync().unwrap();
    direct
}

fn assert_all(what: &str, trait_name: &str, reached: Calls, direct: Calls) {
    let mut seen = reached;
    seen.extend(direct);
    let missing: Vec<_> = declared(trait_name).difference(&seen).copied().collect();
    assert!(missing.is_empty(), "{what} does not forward {missing:?}");
}

#[test]
fn the_test_sees_every_declared_method() {
    let mut recorder = Recorder::default();
    let direct = every_driver_method(&mut recorder);
    assert_all("Recorder", "FsDriver", recorder.calls, direct);
    assert_eq!(declared("FsDriver"), declared("FileSystem"));
}

#[test]
fn driver_wrappers_forward_every_method() {
    let mut recorder = Recorder::default();
    let direct = every_driver_method(&mut &mut recorder);
    assert_all("&mut D", "FsDriver", recorder.calls, direct);

    let mut boxed = Box::new(Recorder::default());
    let direct = every_driver_method(&mut boxed);
    assert_all("Box<D>", "FsDriver", boxed.calls, direct);

    let mut with = WithResolver::new(Recorder::default(), Lexical);
    let direct = every_driver_method(&mut with);
    let mut calls = with.into_inner().calls;
    assert!(!calls.contains("resolve"), "WithResolver resolves itself");
    calls.insert("resolve");
    assert_all("WithResolver", "FsDriver", calls, direct);

    let vol = Volume::local(Recorder::default());
    let direct = every_driver_method(&mut &vol);
    assert_all("&F as FsDriver", "FsDriver", vol.into_inner().calls, direct);

    let vol = Arc::new(Volume::new(Recorder::default()));
    let mut driver = AsDriver::new(Arc::clone(&vol));
    let direct = every_driver_method(&mut driver);
    drop(driver);
    let vol = Arc::into_inner(vol).unwrap();
    assert_all("AsDriver", "FsDriver", vol.into_inner().calls, direct);
}

#[test]
fn file_system_wrappers_forward_every_method() {
    let vol: Volume<_, StdMutex> = Volume::new(Recorder::default());
    let direct = every_fs_method(&vol);
    assert_all("Volume", "FileSystem", vol.into_inner().calls, direct);

    let vol = Volume::local(Recorder::default());
    let direct = every_fs_method(&&vol);
    assert_all("&F", "FileSystem", vol.into_inner().calls, direct);

    let mut vol = Volume::local(Recorder::default());
    let direct = every_fs_method(&&mut vol);
    assert_all("&mut F", "FileSystem", vol.into_inner().calls, direct);

    let vol = Box::new(Volume::local(Recorder::default()));
    let direct = every_fs_method(&vol);
    assert_all("Box<F>", "FileSystem", vol.into_inner().calls, direct);

    let vol = Arc::new(Volume::new(Recorder::default()));
    let direct = every_fs_method(&vol);
    let vol = Arc::into_inner(vol).unwrap();
    assert_all("Arc<F>", "FileSystem", vol.into_inner().calls, direct);

    let vol = Rc::new(Volume::local(Recorder::default()));
    let direct = every_fs_method(&vol);
    let vol = Rc::into_inner(vol).unwrap();
    assert_all("Rc<F>", "FileSystem", vol.into_inner().calls, direct);
}

/// A driver whose methods are inherent, forwarded by `impl_fs_driver!`.
#[derive(Default)]
struct Inherent {
    inner: Recorder,
}

impl Inherent {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn root(&self) -> NodeId {
        ROOT
    }
    fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, std::io::Error> {
        self.inner.lookup(dir, name)
    }
    fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, std::io::Error> {
        self.inner.node_metadata(node)
    }
    fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, std::io::Error> {
        self.inner.read_dir_entry(dir, cursor, name)
    }
    fn read_at(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, std::io::Error> {
        self.inner.read_at(node, offset, buf)
    }
    fn stats(&mut self) -> FsResult<FsStats, std::io::Error> {
        self.inner.stats()
    }
    fn forget(&mut self, node: NodeId) {
        self.inner.forget(node)
    }
    fn open_node(&mut self, node: NodeId) -> FsResult<(), std::io::Error> {
        self.inner.open_node(node)
    }
    fn close_node(&mut self, node: NodeId) {
        self.inner.close_node(node)
    }
    fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, std::io::Error> {
        self.inner.parent(dir)
    }
    fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, std::io::Error> {
        self.inner.read_link(link, buf)
    }
    fn resolve(&mut self, path: &str) -> FsResult<NodeId, std::io::Error> {
        self.inner.resolve(path)
    }
    fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, std::io::Error> {
        self.inner.create(dir, name, kind, meta)
    }
    fn remove(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: RemoveKind,
    ) -> FsResult<(), std::io::Error> {
        self.inner.remove(dir, name, kind)
    }
    fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), std::io::Error> {
        self.inner.rename(from_dir, from, to_dir, to, flags)
    }
    fn write_at(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &[u8],
    ) -> FsResult<usize, std::io::Error> {
        self.inner.write_at(node, offset, buf)
    }
    fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), std::io::Error> {
        self.inner.set_len(node, len)
    }
    fn set_metadata(
        &mut self,
        node: NodeId,
        changes: &SetMetadata,
    ) -> FsResult<(), std::io::Error> {
        self.inner.set_metadata(node, changes)
    }
    fn sync_node(&mut self, node: NodeId) -> FsResult<(), std::io::Error> {
        self.inner.sync_node(node)
    }
    fn publish_node(&mut self, node: NodeId) -> FsResult<(), std::io::Error> {
        self.inner.publish_node(node)
    }
    fn sync(&mut self) -> FsResult<(), std::io::Error> {
        self.inner.sync()
    }
}

hadris_fs::impl_fs_driver!(
    sync,
    impl[] Inherent,
    error = std::io::Error;
    also = [parent, read_link, resolve, open_node, close_node, publish_node]
);

#[test]
fn the_macro_forwards_every_method_when_asked() {
    let mut fs = Inherent::default();
    let direct = every_driver_method(&mut fs);
    assert_all("impl_fs_driver!", "FsDriver", fs.inner.calls, direct);
}

#[test]
fn volume_queues_close_node_and_forget_while_locked() {
    let vol = Volume::local(Recorder::default());
    {
        let _guard = vol.lock();
        vol.close_node(NODE);
        vol.forget(NODE);
    }
    let _ = vol.stats();
    let calls = vol.into_inner().calls;
    assert!(calls.contains("close_node") && calls.contains("forget"));
}
