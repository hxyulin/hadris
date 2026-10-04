# hadris-apfs

`hadris-apfs` is an experimental, read-only reader for APFS containers and
volumes. It supports synchronous and asynchronous I/O and can be used in
`no_std` environments with an allocator. Its API is outside the Hadris 3.x
stability promise and may change in a minor release.

The crate is suitable for inspecting unencrypted APFS images and, with the
`encryption` feature, password-encrypted single-key APFS volumes made by macOS. It
is not a recovery, repair or forensic implementation.

`sync::ApfsFs` and `r#async::ApfsFs` implement the V3 `FileSystem` trait,
so generic `Volume` paths, file handles, walks and extraction work on APFS.
Mounting needs an allocator, but not `std`. `Container` retains native container,
checkpoint, object-map and space-manager inspection. I/O errors retain the
backend's error through `hadris_fs::Error<D::Error>`.

A default mount requires exactly one volume. Select another with
`ApfsFs::mount_volume(device, options, VolumeSelector::Index(index))`, or select
by APFS object ID, UUID or name. Failed mounts return the device through
`MountError`. The umbrella detects and mounts single-volume APFS containers through `detect`.
The `unstable-apfs` feature exposes native inspection and explicit volume
selection; the unified CLI provides `hadris apfs` commands.
Enable `hadris`'s `apfs-encryption` feature for password unlocking through its
native re-export. Ordinary format detection does not enable crypto dependencies.

```rust,no_run
use hadris_apfs::sync::ApfsFs;
use hadris_fs::{MountOptions, OpenOptions};
use hadris_fs::sync::Volume;
use hadris_storage::host::FileDevice;
use std::io::Read;

let device = FileDevice::open("container.img")?;
let volume = Volume::new(ApfsFs::mount(device, MountOptions::new())?);
for entry in volume.read_dir("/")? {
    println!("{:?}", entry?.name());
}
let mut contents = Vec::new();
volume.open("/notes.txt", OpenOptions::new().read())?.read_to_end(&mut contents)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The [V3 use cases](../../../docs/apfs-v3-use-cases.md) explain how inspection,
generic file access, VFS operations and volume selection shape this API.

## Supported scope

- Optional software encryption: container and volume keybags, password-based
  PBKDF2-HMAC-SHA256, AES key unwrap and AES-XTS metadata/file reads on
  `APFS_FS_ONEKEY` volumes. Crypto-user selection is independent of volume
  selection; wrong credentials retain the device and report `InvalidInput`
  with `Detail::Credentials`.
- Container superblocks, the checkpoint descriptor ring and checkpoint maps.
- Container and volume object maps, resolved at the volume's transaction.
- Physical and virtual B-trees, including sealed volumes with hashed index
  entries and nodes stored without object headers.
- Directory listing and path lookup, case-insensitive on volumes formatted
  that way. Path lookup handles `.` and `..`, clamps parent traversal at the
  volume root, and requires directories for intermediate components and trailing
  slashes. Symlinks are returned without following them.
- Inode metadata: mode, owner, BSD flags, timestamps and data-stream size.
- File data from extents, with sparse extents and uncovered ranges read as
  zeros.
- Symlink targets and hard links.
- Space manager summaries and chunk-info blocks.

## Limitations

- The reader is read-only. Creating, modifying and repairing volumes are not
  supported.
- Compressed files (`decmpfs`) are reported as
  `ErrorKind::Unsupported` rather than decoded.
- Software encryption requires the `encryption` feature and a password mount.
  Secure Enclave/hardware encryption, per-file keys and encryption rolling are
  not qualified. Keybags are capped at 1 MiB and 256 records, and the entire
  password-record search is capped at 1,000,000 PBKDF2 iterations.
- Case-insensitive lookup folds case but not Unicode normalization, so a name
  must use the same normalization form as the one stored on disk.
- Snapshots, Fusion tier-2 devices, extended attributes other than symlink
  targets, and named data streams are not exposed.
- Native container path lookup walks the filesystem tree. The mounted driver
  builds an in-memory index, so mounting a large volume needs memory proportional
  to the indexed records.

Do not rely on this crate alone for recovery or forensic conclusions from
damaged, adversarial, compressed or encrypted volumes.

## Password unlocking

`ApfsFs::mount_with_password(device, options, password, crypto_user)` mounts a
sole encrypted volume. `mount_volume_with_password` also accepts a
`VolumeSelector`. Passwords are borrowed as byte slices; host prompting and
keychain access belong to the caller. `None` for `crypto_user` searches supported
password records; an explicit UUID restricts that search.

The native `Container::unlock_volume` replaces its unlocked volume, including
when unlocking fails. Cancellation leaves a candidate key uninstalled. Retained
keys are wiped on drop and redacted from diagnostics. Native encrypted reads
require volume-scoped APIs: `read_volume_extents_at` and
`read_volume_btree_node_with_flags`; the existing unscoped methods reject
encrypted inputs. Decrypted objects still undergo checksum and bounds checks.

The [encryption qualification guide](../../../docs/apfs-encryption.md) describes
the native Apple oracle, resource limits and separate Asahi hardware path.

## Example

```rust,no_run
use std::fs::File;

use hadris_apfs::sync::Container;
use hadris_storage::sync::StreamDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

let file = File::open("container.img")?;
let sectors = file.metadata()?.len() / 512;
let geometry = BlockGeometry::new(BlockSize::new(512).unwrap(), BlockCount::new(sectors));
let mut container = Container::open(StreamDevice::with_block_count(
    hadris_storage::ReadOnly::new(hadris_io::StdIo::new(file)),
    geometry.logical_block_size(), geometry.block_count().get(),
))?;
let latest = container.latest_superblock()?;
for volume in container.volume_superblocks(&latest)? {
    if let Some(entry) = container.resolve_path(&volume, "/notes.txt")? {
        let data = container.read_file(&volume, entry.file_id, 1 << 20)?;
        println!("{}: {} bytes", volume.name()?, data.len());
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

The image must start with the APFS container. For a whole-disk image, open the
APFS partition first, for example with `hadris-part`.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | Yes     | Standard library support (enables `alloc`) |
| `alloc` | Yes     | Heap allocation; required by the B-tree and file readers |
| `read`  | Yes     | The container readers |
| `sync`  | Yes     | Synchronous readers in `hadris_apfs::sync` |
| `async` | No      | Asynchronous readers in `hadris_apfs::r#async` |

## Documentation

- [Library API](https://docs.rs/hadris-apfs)
- [APFS inspection CLI](../../tools/hadris-apfs-cli/README.md)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).

The opt-in `tracing` feature enables `std` and emits operation spans through the
application’s subscriber. It is disabled by default. See the
[tracing guide](../../../docs/tracing.md) for targets, metadata and async behavior.
