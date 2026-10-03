# hadris-apfs

`hadris-apfs` is an experimental, read-only reader for APFS containers and
volumes. It supports synchronous and asynchronous I/O and can be used in
`no_std` environments with an allocator. Its API is outside the Hadris 2.x
stability promise and may change in a minor release.

The crate is suitable for inspecting unencrypted APFS images made by macOS. It
is not a recovery, repair or forensic implementation.

## Supported scope

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
  `ApfsError::Unsupported` rather than decoded.
- Encrypted volumes and encrypted B-tree nodes are rejected.
- Case-insensitive lookup folds case but not Unicode normalization, so a name
  must use the same normalization form as the one stored on disk.
- Snapshots, Fusion tier-2 devices, extended attributes other than symlink
  targets, and named data streams are not exposed.
- Every lookup walks the whole filesystem tree. Large volumes are slow.

Do not rely on this crate alone for recovery or forensic conclusions from
damaged, adversarial, compressed or encrypted volumes.

## Example

```rust,no_run
use std::fs::File;

use hadris_apfs::sync::Container;
use hadris_storage::sync::SeekBlockDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

let file = File::open("container.img")?;
let sectors = file.metadata()?.len() / 512;
let geometry = BlockGeometry::new(BlockSize::new(512).unwrap(), BlockCount(sectors));
let mut container = Container::open(SeekBlockDevice::new(file, geometry))?;
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
