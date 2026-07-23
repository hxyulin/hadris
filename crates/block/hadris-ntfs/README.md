# hadris-ntfs

`hadris-ntfs` is an experimental, read-only NTFS filesystem reader. It
supports synchronous and asynchronous I/O and can be used in `no_std`
environments with an allocator.

The crate is suitable for inspecting known-good NTFS volumes. It is not yet a
complete recovery, repair, or forensic implementation.

## Supported scope

The current reader supports:

- boot-sector signature and geometry validation;
- MFT records protected by update sequence arrays;
- resident and non-resident unnamed data streams;
- sparse data runs and zero-filled uninitialized stream tails;
- resident directory indexes and active index-allocation buffers selected by
  the directory bitmap;
- UTF-16 filenames, including surrogate pairs;
- POSIX case-sensitive lookup and Win32/DOS lookup using the volume's
  `$UpCase` table; and
- MFT sequence-number validation for file references.

See the [NTFS specification coverage matrix](../../../docs/spec-coverage.md#hadris-ntfs)
for implementation-level coverage and source references.

## Limitations

- The filesystem is read-only. Creating, modifying, deleting, formatting, and
  repairing volumes are not supported.
- `$ATTRIBUTE_LIST` records are recognized but not resolved. Attributes stored
  in extension FILE records are therefore unavailable; fragmented MFT data,
  files, or indexes that depend on extension records may fail to open or appear
  incomplete.
- `$MFTMirr` is not used to recover unreadable MFT records.
- NTFS-compressed and encrypted data streams are rejected rather than decoded.
- Only the unnamed `$DATA` stream is exposed; named alternate data streams are
  not available through the public file API.
- Reparse-point payloads and their filesystem semantics are not interpreted.
- `$LogFile` replay, dirty-volume recovery, and consistency checking are not
  implemented.
- Security descriptors, timestamps, hard-link metadata, and several other NTFS
  metadata attributes are not exposed by the high-level API.
- Directory enumeration scans active index buffers. It does not yet perform a
  keyed descent through the on-disk B-tree, so large-directory lookup is not
  optimized.

Do not use this crate as the sole source for recovery or forensic conclusions
from damaged, dirty, adversarial, compressed, or encrypted volumes.

## Example

```rust,no_run
use std::fs::File;

use hadris_ntfs::sync::{NtfsFs, NtfsFsReadExt};

let image = File::open("disk.img")?;
let filesystem = NtfsFs::open(image)?;

for entry in filesystem.root_dir().entries()? {
    println!(
        "{} ({})",
        entry.name(),
        if entry.is_directory() { "directory" } else { "file" }
    );
}

# Ok::<(), Box<dyn std::error::Error>>(())
```

## Feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `read` | Yes | Enables filesystem reading and requires `alloc`. |
| `std` | Yes | Enables standard-library support, `alloc`, and `sync`. |
| `alloc` | Via `std`/`read` | Enables APIs that allocate. |
| `sync` | Via `std` | Enables the synchronous API. |
| `async` | No | Enables the asynchronous API. |

The default synchronous API is available under `hadris_ntfs::sync` and is also
re-exported from the crate root. Enable `async` to use
`hadris_ntfs::r#async`.

## Development

Run the NTFS checks from the repository root in the project development
container:

```console
scripts/test-ntfs.sh
```

Pass a command to the same script to run an individual check:

```console
scripts/test-ntfs.sh cargo test -p hadris-ntfs --all-features
```
