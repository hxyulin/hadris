# Hadris ISO

ISO 9660 images for Rust: an allocation-free reader for every tree an image
can carry, and a reproducible writer with Joliet, Rock Ridge, El Torito and
hybrid MBR/GPT boot. It runs on any `hadris-storage` block device, in
desktop tools as well as `no_std` bootloaders, kernels and firmware.

## Overview

- **One reader for every tree.** `IsoFs::mount` reads the most capable tree
  an image has and `IsoFs::mount_namespace` picks the primary tree, Rock
  Ridge names and metadata over it, the Joliet tree or the ISO 9660:1999
  enhanced tree. `IsoFs` is a `hadris-fs` `FileSystem`, so `Volume`, its
  handles and `read_tree` work on it.
- **No allocator needed to read.** Mounts, lookups, listings and file reads
  use fixed buffers; only the boot catalog listing needs `alloc`.
- **A writer driven by a shared tree.** `write` lays out a
  `hadris_fs::Tree` as `IsoOptions` says and returns a `hadris_fs::Report`
  of the image size, where each file went and what could not be stored.
  `plan` gives the same report without I/O. The writer reads no clock:
  `with_time` dates the image, so images are reproducible.
- **Sessions.** `Session` reads an image back into a tree whose files point
  at their extents, and writes it again as a new session or rebuilt in
  place.
- **Sync, async and `Send` async** APIs generated from one source.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Implies `alloc`; `std::io::Error` conversions and host files as tree content | Yes |
| `alloc` | The writer, sessions, `BootCatalog` and the `Tree` input | via `std` |
| `sync` | Blocking API in `hadris_iso::sync` | Yes |
| `async` | Asynchronous API in `hadris_iso::r#async` | - |

No feature changes what an item does. Joliet, Rock Ridge, El Torito and the
enhanced tree are always available.

## Usage

### Reading an image

```rust,no_run
use std::io::Read;

use hadris_fs::sync::Volume;
use hadris_fs::{MountOptions, OpenOptions};
use hadris_iso::sync::IsoFs;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
// Rock Ridge if present, then Joliet, the enhanced tree, the primary tree.
let iso = IsoFs::mount(hadris_storage::host::FileDevice::open("image.iso")?, MountOptions::new())?;
println!("trees: {:?}", iso.namespaces().iter().collect::<Vec<_>>());
let vol = Volume::new(iso);
for item in vol.read_dir("/")? {
    let item = item?;
    println!("{:?} {}", item.file_type(), String::from_utf8_lossy(item.name().as_bytes()));
}
let mut config = String::new();
vol.open("/boot/grub/grub.cfg", OpenOptions::new().read())?.read_to_string(&mut config)?;
let tree = hadris_fs::sync::read_tree(&vol, "/")?;
std::fs::create_dir_all("out")?;
hadris_fs::host::write_tree("out", &tree)?;
# Ok(())
# }
```

In the primary and enhanced trees, `lookup` tries the exact name first and
then ignores ASCII case, and version suffixes (`;1`) are not part of listed
names. `IsoFs::info` returns the primary volume descriptor's block size,
volume size, identifiers (`IsoId`) and dates (`IsoDate`);
`IsoFs::rock_ridge` the Rock Ridge entries of a node; `IsoFs::records` where
its directory records lie and `IsoFs::extents` where its data lies, both
read with `IsoFs::read_raw`; and `IsoFs::boot_catalog(&mut buf)` the El
Torito catalog, parsed from the caller's buffer, with `IsoFs::boot_image`
locating each entry's image. The on-disk layouts are in `hadris_iso::raw`.

### Without an allocator

```toml
[dependencies]
hadris-iso = { version = "3.0.0-rc.1", default-features = false, features = ["sync"] }
```

`IsoFs::mount` takes any `BlockDevice` whose blocks are at most 4096
bytes; the image's logical block size may be 512, 1024 or 2048 bytes. Every
read goes through one fixed-size buffer.

### Writing an image

```rust,no_run
use hadris_fs::host::{self, TreeOptions};
use hadris_fs::{Content, NoClock, Node};
use hadris_iso::{IsoId, IsoOptions};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let (mut tree, _skipped) = host::read_tree("rootfs", &TreeOptions::new())?;
tree.insert("README.txt", Node::file(Content::bytes("Built with hadris-iso\n")))?;

let options = IsoOptions::default()
    .with_id(IsoId::Volume, "MY_DISC")
    .with_id(IsoId::Publisher, "Example")
    .with_joliet()
    .with_rock_ridge()
    .with_time(host::source_date_epoch()?.unwrap_or(NoClock::TIME));

let size = hadris_iso::plan(&tree, &options)?.size();
let file = std::fs::File::options()
    .read(true)
    .write(true)
    .create(true)
    .truncate(true)
    .open("out.iso")?;
let report = hadris_iso::sync::write(hadris_storage::host::FileDevice::new(file)?, &tree, &options)?;
assert_eq!(report.size(), size);
for warning in report.warnings() {
    eprintln!("warning: {warning}");
}
# Ok(())
# }
```

`IsoLevel` (`L1`, `L2`, `L3`) sets the primary tree's name rules and file
sizes and `NameCase` whether names keep their case. `with_id` and
`with_date` set the descriptor identifiers and dates; identifiers are stored
as given and must fit their fields. `with_iso1999` adds an ISO 9660:1999
tree. Rock
Ridge stores permissions, owners, times, symlinks, device nodes and hard
links; without it they are dropped with a warning. File contents come from
bytes, a host file (`host::file`) or a mounted volume (`read_tree`), and are
read once while the image is written.

### Bootable images

```rust,no_run
use hadris_fs::Content;
use hadris_iso::{AppendedPartition, BootEntry, BootInfo, ElTorito, Hybrid, IsoOptions};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let esp = Content::bytes(std::fs::read("efi.img")?);
let options = IsoOptions::default()
    .with_el_torito(
        ElTorito::new()
            .with_entry(
                BootEntry::bios("boot/bios.img")
                    .with_load_size(4)
                    .with_boot_info(BootInfo::Table),
            )
            .with_entry(BootEntry::uefi_appended(0))
            .with_catalog_path("boot/boot.cat"),
    )
    .with_hybrid(Hybrid::gpt_hybrid_mbr().with_appended(AppendedPartition::esp(esp)));
# Ok(())
# }
```

`Hybrid::mbr`, `gpt` and `gpt_hybrid_mbr` add partition tables for USB
sticks. `with_appended` stores a partition after the files, so an EFI
system partition is stored once for the GPT and for El Torito
(`BootEntry::uefi_appended`). Without one, the GPT's EFI system partition is
the image of the only UEFI boot entry, `BootEntry::uefi(path)`. Without
`with_catalog_path` the boot catalog is not listed in any tree.

### Sessions

```rust,no_run
use hadris_fs::{Content, Node};
use hadris_iso::SessionMode;
use hadris_iso::sync::Session;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let file = std::fs::File::options().read(true).write(true).open("image.iso")?;
let mut session = Session::open(hadris_storage::host::FileDevice::new(file)?)?;
session.tree_mut().insert("notes.txt", Node::file(Content::bytes("added later")))?;
session.tree_mut().remove("old.log")?;
let options = session.options();
session.write(&options, SessionMode::Append)?;
# Ok(())
# }
```

`Append` writes a new descriptor set and the new files after the old
volume and copies the descriptors to sector 16, so every reader sees the new
session. `Rewrite` rebuilds the directories in place and keeps the old file
data where it is. Unchanged files are never copied.

### Rock Ridge relocation

ECMA-119 allows eight directory levels. With Rock Ridge, deeper directories
move into a relocation directory and appear in their real place to Rock
Ridge readers. It is `rr_moved` (`Relocation::RrMoved`, the default) or
`.rr_moved` (`IsoOptions::with_relocation(Relocation::DotRrMoved)`), the only
names libarchive (`bsdtar`) reads relocated directories from. A root
directory with that name is reused and keeps its own entries; a root file
with that name fails. libarchive takes the first root directory with either
name for the relocation directory, so a root directory with the other name
that would come first in the directory fails with `Detail::Relocation`:
`.rr_moved` with `NameCase::Preserve`, or `rr_moved` when the relocation
directory is `.rr_moved`. `Relocation::Refuse` fails instead. Joliet and
enhanced trees keep the real hierarchy.

## Extension Support

| Extension | Read | Write | Notes |
|-----------|------|-------|-------|
| ISO 9660 Levels 1-3 | Yes | Yes | Multi-extent files above 4 GiB at Level 3 |
| ISO 9660:1999 | Yes | Yes | Enhanced volume descriptor tree |
| SUSP | Yes | Yes | Continuation areas both ways |
| Rock Ridge (RRIP) | Yes | Yes | PX, PN, NM, SL, TF, CL, PL, RE; not SF or RR |
| Joliet | Yes | Yes | UCS-2 (BMP) names, levels 1-3 |
| El Torito | Yes | Yes | Sections, emulation modes, boot info tables |
| Hybrid MBR/GPT | - | Yes | Through `hadris-part` |

## Compatibility

The conformance harness checks Hadris images with an independent ECMA-119
oracle and measures widely available ISO readers. Current bounded results are:

| Consumer of Hadris images | Result |
|---|---:|
| ECMA-119 raw-image oracle | 2/2 |
| xorriso/libisofs 1.5.8 | 2/2 |
| Linux kernel ISO driver | 2/2 |
| macOS 26.6.2 built-in ISO reader | 2/2 |
| Windows `Mount-DiskImage` | Available manual target; not yet measured |

The [compliance profile](../../../docs/compliance/hadris-iso.md) contains the
producer matrix, methodology, and known peer deviations.

## Examples

```bash
cargo run -p hadris-iso --example read_iso -- path/to/image.iso
cargo run -p hadris-iso --example extract_files -- path/to/image.iso ./output
cargo run -p hadris-iso --example create_bootable_iso -- bootable.iso
```

## Specification References

- ECMA-119 (ISO 9660)
- IEEE P1281 (System Use Sharing Protocol / SUSP)
- IEEE P1282 (Rock Ridge Interchange Protocol / RRIP)
- Joliet Specification (Microsoft)
- El Torito Bootable CD-ROM Format Specification

## Documentation

- [Read ISO images](https://hxyulin.github.io/hadris/guides/read-iso)
- [Create ISO images](https://hxyulin.github.io/hadris/creation/iso)
- [Validate generated images](https://hxyulin.github.io/hadris/guides/validate-images)
- [API reference](https://docs.rs/hadris-iso)

## License

This project is licensed under the [MIT license](../../../LICENSE-MIT).
