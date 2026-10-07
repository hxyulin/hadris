#!/usr/bin/env python3
"""Probe real ISO reading and optional writing against the poll contract."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap


TESTS = r'''
use std::cell::Cell;
use std::convert::Infallible;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use hadris_fs::{Content, MountOptions, Name, Node, Resolve, Tree};
use hadris_io::{Error, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};
use iso_poll_probe::async_::IsoFs;
use proto::{BlockDevice, SendBlockDevice};

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}
fn require_send(_: impl Future + Send) {}
fn generic_iso_future<D: SendBlockDevice>(fs: &mut IsoFs<D>, buf: &mut [u8]) {
    require_send(fs.lookup(fs.root(), Name::new("hello.txt")));
    require_send(fs.read(fs.root(), 0, buf));
}
async fn generic_filesystem_read<F: hadris_fs::r#async::FileSystem<DeviceError = Infallible>>(
    fs: &mut F,
) -> [u8; 12] {
    let node = fs.resolve(b"/hello.txt", Resolve::Follow).await.unwrap();
    let mut bytes = [0; 12];
    assert_eq!(fs.read(node, 0, &mut bytes).await.unwrap(), 12);
    fs.forget(node, 1);
    bytes
}
fn image() -> Vec<u8> {
    let mut tree = Tree::new();
    tree.insert("hello.txt", Node::file(Content::bytes("hello world\n")))
        .unwrap();
    let options = legacy_iso::IsoOptions::new().with_joliet();
    let size = legacy_iso::plan(&tree, &options).unwrap().size();
    let mut device = MemDevice::new(vec![0; size as usize], BlockSize::new(2048).unwrap());
    legacy_iso::sync::write(&mut device, &tree, &options).unwrap();
    device.into_inner()
}

#[test]
fn real_iso_borrowed_partition_has_send_futures_and_generic_filesystem_support() {
    let image = image();
    let len = image.len() as u64;
    let mut disk = vec![0xA5; 2048];
    disk.extend_from_slice(&image);
    let mut device = MemDevice::new(disk, BlockSize::new(512).unwrap());
    let partition = Partition::new(&mut device, 2048, len);
    let adapter = CACHE_CONSTRUCTION;
    require_send(IsoFs::mount(adapter, MountOptions::new()));
    let partition = Partition::new(&mut device, 2048, len);
    let adapter = CACHE_CONSTRUCTION;
    let mut fs = block_on(IsoFs::mount(adapter, MountOptions::new())).unwrap();
    let mut buf = [0; 12];
    generic_iso_future(&mut fs, &mut buf);
    std::thread::scope(|scope| {
        let future = generic_filesystem_read(&mut fs);
        assert_eq!(
            scope.spawn(move || block_on(future)).join().unwrap(),
            *b"hello world\n"
        );
    });
}

struct RcDevice {
    inner: MemDevice<Vec<u8>>,
    pending: Rc<Cell<usize>>,
    cancelled: Rc<Cell<usize>>,
    active: bool,
}
impl ErrorType for RcDevice {
    type Error = Infallible;
}
impl BlockDevice for RcDevice {
    type State = bool;
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 512
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut bool,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        if !*state {
            assert!(!self.active);
            self.active = true;
            *state = true;
            self.pending.set(self.pending.get() + 1);
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            self.active = false;
            Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
                &mut self.inner,
                first,
                buf,
            ))
        }
    }
    fn cancel(&mut self, _: &mut bool) {
        assert!(self.active);
        self.active = false;
        self.cancelled.set(self.cancelled.get() + 1);
    }
}

#[test]
fn same_real_iso_reader_accepts_local_device_and_cancels_pending_io() {
    let cancelled = Rc::new(Cell::new(0));
    let pending = Rc::new(Cell::new(0));
    let mut device = RcDevice {
        inner: MemDevice::new(image(), BlockSize::new(512).unwrap()),
        pending: Rc::clone(&pending),
        cancelled: Rc::clone(&cancelled),
        active: false,
    };
    {
        let mut mount = std::pin::pin!(IsoFs::mount(&mut device, MountOptions::new()));
        assert!(
            mount
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(cancelled.get(), 1);
    assert!(!device.active);
    let mut fs = block_on(IsoFs::mount(&mut device, MountOptions::new())).unwrap();
    let root = fs.root();
    {
        let mut lookup = std::pin::pin!(fs.lookup(root, Name::new("hello.txt")));
        assert!(
            lookup
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(cancelled.get(), 2);
    let node = block_on(fs.lookup(root, Name::new("hello.txt"))).unwrap();
    let mut bytes = [0; 12];
    {
        let mut read = std::pin::pin!(fs.read(node, 0, &mut bytes));
        assert!(
            read.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(cancelled.get(), 3);
    assert_eq!(block_on(fs.read(node, 0, &mut bytes)).unwrap(), 12);
    assert_eq!(&bytes, b"hello world\n");
    fs.forget(node, 1);
    assert!(pending.get() > 3);
}
'''



SECTORS_ADAPTER = r'''
struct SectorsState<S> {
    inner: S,
    scratch: [u8; 4096],
    done: usize,
    pending: bool,
}

impl<S: Default> Default for SectorsState<S> {
    fn default() -> Self {
        Self {
            inner: S::default(),
            scratch: [0; 4096],
            done: 0,
            pending: false,
        }
    }
}

impl<D: BlockDevice> BlockDevice for Sectors<'_, D> {
    type State = SectorsState<D::State>;

    fn block_size(&self) -> BlockSize {
        const { BlockSize::new(512).unwrap() }
    }

    fn block_count(&self) -> u64 {
        self.len / 512
    }

    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut core::task::Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> core::task::Poll<Result<(), hadris_fs::Error<Self::Error>>> {
        use core::task::Poll;
        let past_end = || {
            hadris_fs::Error::new(
                ErrorKind::InvalidInput,
                "block request past the end of the device",
            )
            .with_location(hadris_fs::Location::Block(first.get()))
        };
        let Some(offset) = first.get().checked_mul(512) else {
            return Poll::Ready(Err(past_end()));
        };
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Poll::Ready(Err(past_end()));
        }
        let bs = self.dev.block_size().get() as usize;
        if bs > 4096 {
            return Poll::Ready(Err(ErrorKind::Unsupported.into()));
        }
        while state.done < buf.len() {
            let pos = offset + state.done as u64;
            let within = (pos % bs as u64) as usize;
            let left = buf.len() - state.done;
            let block = BlockIndex::new(pos / bs as u64);
            let aligned = within == 0 && left >= bs;
            let whole = if aligned { left - left % bs } else { bs };
            if block
                .get()
                .checked_add((whole / bs) as u64)
                .is_none_or(|end| end > self.dev.block_count())
            {
                return Poll::Ready(Err(past_end()));
            }
            state.pending = true;
            let result = if aligned {
                self.dev.poll_read_blocks(
                    &mut state.inner,
                    cx,
                    block,
                    &mut buf[state.done..state.done + whole],
                )
            } else {
                self.dev
                    .poll_read_blocks(&mut state.inner, cx, block, &mut state.scratch[..bs])
            };
            match result {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    state.pending = false;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(())) => {
                    state.pending = false;
                    state.inner = D::State::default();
                    if aligned {
                        state.done += whole;
                    } else {
                        let take = (bs - within).min(left);
                        buf[state.done..state.done + take]
                            .copy_from_slice(&state.scratch[within..within + take]);
                        state.done += take;
                    }
                }
            }
        }
        Poll::Ready(Ok(()))
    }

    fn cancel(&mut self, state: &mut Self::State) {
        if state.pending {
            self.dev.cancel(&mut state.inner);
            state.pending = false;
        }
    }
}
'''

def run(directory: Path, repository: Path, toolchain: str, allocated: bool = False) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    shutil.copytree(repository / "crates/optical/hadris-iso/src", directory / "src", dirs_exist_ok=True)
    unified = directory / "src/unified.rs"
    source = unified.read_text()
    source = source.replace("use hadris_storage::async_ as storage;", "use proto as storage;")
    source = source.replace("hadris_storage::async_::SendBlockDevice", "proto::SendBlockDevice")
    unified.write_text(source)
    if allocated:
        asynchronous = directory / "src/async.rs"
        source = asynchronous.read_text()
        source = source.replace("use hadris_storage::r#async as storage;", "use proto as storage;")
        source = source.replace("use hadris_fs::r#async as fs;", "use hadris_fs::local as fs;")
        source = source.replace("use hadris_fs::r#async::FileSystem;", "use hadris_fs::local::FileSystem;")
        asynchronous.write_text(source)
        session = directory / "src/session.rs"
        source = session.read_text()
        start = source.index("impl<D: BlockDevice> BlockDevice for Sectors<'_, D> {")
        end = source.index("/// An existing image read into a [`Tree`]", start)
        source = source[:start] + SECTORS_ADAPTER + source[end:]
        session.write_text(source)
        partition_directory = directory / "partition"
        shutil.copytree(repository / "crates/block/hadris-part/src", partition_directory / "src", dirs_exist_ok=True)
        partition_lib = partition_directory / "src/lib.rs"
        source = partition_lib.read_text().replace("use hadris_storage::r#async as storage;", "use proto as storage;")
        partition_lib.write_text(source)
    paths = {
        "hadris-io": "crates/core/hadris-io",
        "hadris-fs": "crates/core/hadris-fs",
        "hadris-storage": "crates/core/hadris-storage",
        "hadris-macros": "crates/core/hadris-macros",
        "hadris-iso-raw": "crates/optical/hadris-iso-raw",
    }
    features = {
        "hadris-io": ["async"],
        "hadris-fs": ["async", "async-local"],
        "hadris-storage": ["async"],
    }
    manifest = textwrap.dedent('''\
        [package]
        name = "iso-poll-probe"
        version = "0.0.0"
        edition = "2024"
        rust-version = "1.88"
        publish = false
        [workspace]
        [features]
        default = ["async"]
        async = []
        async-local = []
        sync = []
        std = ["alloc", "hadris-io/std", "hadris-fs/std", "hadris-storage/std", "hadris-part?/std"]
        alloc = ["hadris-io/alloc", "hadris-fs/alloc", "hadris-storage/alloc", "dep:hadris-part"]
        cache = []
        tracing = []
        [dependencies]
        bitflags = "2"
        bytemuck = { version = "1", features = ["derive", "min_const_generics"] }
    ''')
    for name, path in paths.items():
        manifest += f'{name} = {{ path = {json.dumps(str(repository / path))}, default-features = false'
        if name in features:
            manifest += f', features = {json.dumps(features[name])}'
        manifest += ' }\n'
    manifest += 'proto = { package = "hadris-experiment-async-device-contract", path = '
    manifest += json.dumps(str(repository / "examples/async-device-contract")) + (', default-features = false, features = ["alloc"] }\n' if allocated else ', default-features = false }\n')
    if allocated:
        partition_manifest = manifest[:manifest.index('[dependencies]')].replace('[workspace]\n', '')
        partition_manifest = partition_manifest.replace('name = "iso-poll-probe"', 'name = "hadris-part-poll-probe"')
        partition_manifest = partition_manifest.replace('"hadris-part?/std"', '"hadris-storage/std"').replace(', "dep:hadris-part"', '')
        partition_manifest += manifest[manifest.index('[dependencies]'):]
        (partition_directory / "Cargo.toml").write_text(partition_manifest)
        manifest += 'hadris-part = { package = "hadris-part-poll-probe", path = "partition", default-features = false, optional = true, features = ["alloc", "async"] }\n'
    manifest += '[dev-dependencies]\nstatic_assertions = "1"\nlegacy_iso = { package = "hadris-iso", path = '
    manifest += json.dumps(str(repository / "crates/optical/hadris-iso")) + ', features = ["std", "sync"] }\n'
    if not allocated:
        manifest = manifest.replace(', "hadris-part?/std"', '').replace(', "dep:hadris-part"', '')
    (directory / "Cargo.toml").write_text(manifest)
    shutil.copyfile(repository / "Cargo.lock", directory / "Cargo.lock")
    (directory / "tests").mkdir(exist_ok=True)
    (directory / "tests/real_reader.rs").write_text(TESTS.replace("CACHE_CONSTRUCTION", "proto::Cache::new(partition)"))
    if allocated:
        writer_tests = repository / "examples/async-device-contract/iso_write_tests.rs.txt"
        (directory / "tests/real_writer.rs").write_text(writer_tests.read_text())
    command = ["cargo", f"+{toolchain}", "test", "--manifest-path", str(directory / "Cargo.toml"), "--test", "real_reader"]
    if allocated:
        command += ["--features", "std", "--test", "real_writer"]
    mode = "allocated writer/session" if allocated else "allocation-free reader"
    print(f"ISO probe: {directory}; mode: {mode}; adapter stack: Cache + borrowed Partition", flush=True)
    if not allocated:
        check = ["cargo", f"+{toolchain}", "check", "--manifest-path", str(directory / "Cargo.toml"), "--lib"]
        subprocess.run(check, check=True, env=os.environ.copy())
    subprocess.run(command, check=True, env=os.environ.copy())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allocated", action="store_true", help="Also compile real writers and sessions with allocated state")
    parser.add_argument("--toolchain", default="1.88.0")
    parser.add_argument("--output-dir", type=Path, help="Keep the generated probe crate at this path")
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[2]
    if args.output_dir:
        run(args.output_dir.resolve(), repository, args.toolchain, args.allocated)
    else:
        with tempfile.TemporaryDirectory(prefix="hadris-iso-poll-probe-") as temporary:
            run(Path(temporary), repository, args.toolchain, args.allocated)


if __name__ == "__main__":
    main()
