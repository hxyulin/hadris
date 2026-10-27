# hadris-apfs

`hadris-apfs` is an experimental, read-only reader for APFS containers and
volumes. It supports synchronous and asynchronous I/O and can be used in
`no_std` environments with an allocator. Its API is outside the Hadris 3.x
stability promise and may change in a minor release; depend on it with
`hadris-apfs = "~3.0"` to take only compatible updates.

The crate is suitable for inspecting unencrypted APFS images and, with the
`encryption` feature, password-encrypted single-key APFS volumes made by macOS. It
is not a recovery, repair or forensic implementation.

`sync::ApfsFs` and `async_::ApfsFs` implement the V3 `FileSystem` trait,
so generic `Volume` paths, file handles, walks and extraction work on APFS.
Mounting needs an allocator, but not `std`. `Container` retains native container,
checkpoint, object-map and space-manager inspection. I/O errors retain the
backend's error through `hadris_fs::Error<D::Error>`.

`Container::open` returns an owned device on failure through `MountError` in
both modes. Use `into_parts()` to inspect the error and recover the device,
or `into_error()` to discard the device and propagate the I/O failure.

A default mount requires exactly one volume. Select another with
`ApfsFs::mount_volume(device, options, VolumeSelector::Index(index))`, or select
by APFS object ID, UUID or name. Failed mounts return the device through
`MountError`. The umbrella's `detect` lists APFS containers without
mounting them; its `unstable-apfs` feature adds mounting of single-volume
containers through `open`, native inspection and explicit volume
selection. The unified CLI provides `hadris apfs` commands.
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

`VolumeSuperblock::name_comparison()` derives the volume's name policies.
For comparisons chosen by the caller, `NameComparison::new` takes
`hadris_fs::CaseRule` and `NormalizationRule` as independent axes. Case
folding alone does not enable normalization; a volume whose APFS case bit
is set still enables both, as the on-disk rules require.

- Optional software encryption: container and volume keybags, password-based
  PBKDF2-HMAC-SHA256, AES key unwrap and AES-XTS metadata/file reads on
  `APFS_FS_ONEKEY` volumes. Crypto-user selection is independent of volume
  selection; wrong credentials retain the device and report `InvalidInput`
  with `Detail::Credentials`.
- Container superblocks, the checkpoint descriptor ring and checkpoint maps.
  Block zero supplies the geometry and descriptor area; its checksum is not
  required. `latest_superblock` and mounting use the newest descriptor-area
  superblock whose checksum, checkpoint map and object map verify, falling
  back to older checkpoints, then to a verified block zero.
- Container and volume object maps, resolved at the volume's transaction.
- Physical and virtual B-trees, including sealed volumes with hashed index
  entries, nodes stored without object headers and file extents kept in the
  file extent tree.
- Incompatible features: mounting accepts the container feature
  `NX_INCOMPAT_VERSION2` and the volume features `CASE_INSENSITIVE`,
  `NORMALIZATION_INSENSITIVE`, `DATALESS_SNAPS`, `ENC_ROLLED` and
  `SEALED_VOLUME`. Version 1 and Fusion containers, volumes with
  `INCOMPLETE_RESTORE` or the reserved `0x40` bit, and any bit the
  specification does not define fail with `ErrorKind::Unsupported` and
  `Detail::Feature`. Directory keys use the hashed layout
  (`j_drec_hashed_key_t`) on case- or normalization-insensitive volumes and
  the plain layout (`j_drec_key_t`) otherwise.
- Directory listing and path lookup, folded by case and canonical
  decomposition on volumes formatted that way. Path lookup handles `.` and
  `..`, clamps parent traversal at the volume root, and requires directories
  for intermediate components and trailing slashes. Symlinks are returned
  without following them.
- Inode metadata: mode, owner, BSD flags, timestamps and data-stream size.
- Directory names as stored, including names that are not UTF-8 and names
  longer than 255 bytes, up to the key's name-length field (1022 bytes for
  hashed keys). Listings of names longer than `DirEntry::MAX_NAME` fail with
  `NameTooLong` at that entry.
- Damage confined to one node: a directory entry whose inode is missing,
  malformed, of an unknown mode or of another type than the entry records,
  whose extents overlap or leave the container, or whose symlink target is
  stored in a data stream, is listed with the entry's type only. `stat`,
  `read`, `open` and `readlink` on that node return the error.
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
- Name comparison uses the Unicode default case folding and canonical
  decomposition rather than Apple's unpublished tables, so a name that
  differs only where the two disagree may not match.
- Snapshots, Fusion tier-2 devices, extended attributes other than symlink
  targets, and named data streams are not exposed.
- Native container path lookup walks the filesystem tree; a keyed query reads
  only the nodes whose key range can hold the records it wants. The mounted
  driver reads a node's records when an operation first needs them and keeps
  them for the nodes in use, so mounting a large volume no longer reads the
  whole filesystem tree. `MountOptions::with_node_limit` caps how many nodes
  are held; an operation that needs one more fails with `LimitExceeded`.

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
| `sync`  | Yes     | Synchronous readers in `hadris_apfs::sync` |
| `async` | No      | Unified asynchronous readers in `hadris_apfs::async_` |
| `encryption` | No | Password-based software FileVault unlocking; implies `alloc` |
| `tracing` | No | Operation spans through the `tracing` crate, also without `std` |

## Tracing

The opt-in `tracing` feature emits operation spans through the application’s
subscriber, with or without `std`. It is disabled by default. See the
[tracing guide](../../../docs/tracing.md) for targets, metadata and async behavior.

## Object mappings

Object-map lookups match full object identifiers and select the newest mapping
at or before the requested transaction. A deleted mapping returns no object
without resurrecting an older version. Volume enumeration uses the container
checkpoint transaction to resolve each live volume once; historical and future
mappings remain available through the raw mapping inspection APIs.

## Unified asynchronous API

One filesystem type in `async_` accepts both `Send` and non-`Send` devices
through `hadris_storage::async_::BlockDevice`. It implements
`hadris_fs::async_::FileSystem` for every compatible device, and
`hadris_fs::async_::SendFileSystem` when the device and its operation state
are `Send`.

## Documentation

- [Library API](https://docs.rs/hadris-apfs/3.0.0-rc.3)
- [Unified CLI (`hadris apfs`)](../../tools/hadris-cli/README.md)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
