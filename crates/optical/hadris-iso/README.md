# Hadris ISO

ISO 9660 images for Rust: an allocation-free reader for every tree an image
can carry, and a reproducible writer with Joliet, Rock Ridge, El Torito and
hybrid MBR/GPT boot. It runs on any `hadris-storage` block device, in
desktop tools as well as `no_std` bootloaders, kernels and firmware.

## Overview

- **One reader for every tree.** `IsoImage` opens an image and `view` picks
  the primary tree, Rock Ridge names and metadata over it, the Joliet tree or
  the ISO 9660:1999 enhanced tree. A view is a `hadris-fs` `FsDriver`, so
  the shared path helpers, handles and `extract_to_host` work on it.
- **No allocator needed to read.** Views, lookups, listings and file reads
  use fixed buffers; only the boot catalog listing needs `alloc`.
- **A writer driven by a shared tree.** `write` lays out a
  `hadris_fs::tree::Tree` as `IsoOptions` says and returns a `Report` of the
  image size, where each file went and what could not be stored. `plan`
  gives the same report without writing. The clock is injected, so images
  are reproducible.
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
| `async-send` | Asynchronous API with `Send` futures in `hadris_iso::async_send` | - |

No feature changes what an item does. Joliet, Rock Ridge, El Torito and the
enhanced tree are always available.

## Usage

### Reading an image

```rust,no_run
use hadris_fs::sync::DriverExt;
use hadris_iso::Namespace;
use hadris_iso::sync::IsoImage;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut iso = IsoImage::open(std::fs::File::open("image.iso")?)?;
println!("trees: {:?}", iso.namespaces().iter().collect::<Vec<_>>());

// Rock Ridge if present, then Joliet, the enhanced tree, the primary tree.
let mut view = iso.view(Namespace::Preferred)?;
for item in view.read_dir("/")? {
    let item = item?;
    println!("{:?} {}", item.file_type(), String::from_utf8_lossy(item.name_bytes()));
}
let config = view.read_to_vec("/boot/grub/grub.cfg")?;
hadris_fs::sync::extract_to_host(&mut view, "/", "out")?;
# let _ = config;
# Ok(())
# }
```

In the primary and enhanced trees, `lookup` tries the exact name first and
then ignores ASCII case, and version suffixes (`;1`) are not part of listed
names. `IsoView::rock_ridge` returns the Rock Ridge entries of a node,
`IsoView::raw_record` its directory record, `IsoView::extents` where its data
lies, and `IsoImage::boot_catalog` the El Torito catalog. The on-disk layouts
are in `hadris_iso::raw`.

### Without an allocator

```toml
[dependencies]
hadris-iso = { version = "2.4.0", default-features = false, features = ["sync"] }
```

`IsoImage::open` takes any `BlockDevice` whose blocks are at most 4096
bytes; the image's logical block size may be 512, 1024 or 2048 bytes. Every
read goes through one fixed-size buffer.

### Writing an image

```rust,no_run
use hadris_fs::SystemClock;
use hadris_fs::tree::{Content, FromFsOptions, Tree};
use hadris_iso::{IsoOptions, JolietLevel, RockRidge, VolumeIdentifiers};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut tree = Tree::from_fs("rootfs", FromFsOptions::new())?;
tree.add_file("README.txt", Content::bytes("Built with hadris-iso\n"))?;

let options = IsoOptions::default()
    .with_volume(VolumeIdentifiers::new("MY_DISC").with_publisher("Example"))
    .with_joliet(JolietLevel::L3)
    .with_rock_ridge(RockRidge::default())
    .with_clock(SystemClock);

let size = hadris_iso::sync::plan(&tree, &options)?.size_bytes();
let file = std::fs::File::options()
    .read(true)
    .write(true)
    .create(true)
    .truncate(true)
    .open("out.iso")?;
let report = hadris_iso::sync::write(file, &tree, &options)?;
assert_eq!(report.size_bytes(), size);
for warning in report.warnings() {
    eprintln!("warning: {warning}");
}
# Ok(())
# }
```

`IsoLevel` (`L1`, `L2`, `L3`) sets the primary tree's name rules and file
sizes, `NameCase` whether names keep their case, and `Charset::Strict` maps
invalid characters. `with_enhanced_tree` adds an ISO 9660:1999 tree. Rock
Ridge stores permissions, owners, times, symlinks, device nodes and hard
links; without it they are dropped with a warning. File contents come from
bytes, a reader, an async reader or, with `std`, a host path, and are read
once while the image is written.

### Bootable images

```rust,no_run
use hadris_iso::{BootEntry, BootInfo, ElTorito, HybridBoot, IsoOptions, Platform};

let options = IsoOptions::default()
    .with_el_torito(
        ElTorito::new(
            BootEntry::new("boot/bios.img")
                .with_load_size(4)
                .with_boot_info_table(BootInfo::Standard),
        )
        .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi))
        .with_catalog_path("boot/boot.cat"),
    )
    .with_hybrid(HybridBoot::hybrid());
```

`HybridBoot::mbr`, `gpt` and `hybrid` add partition tables for USB sticks;
the EFI system partition is the image of the only UEFI boot entry unless
`with_efi_partition` names another file. Without `with_catalog_path` the
boot catalog is not listed in any tree.

### Sessions

```rust,no_run
use hadris_fs::tree::Content;
use hadris_iso::SessionMode;
use hadris_iso::sync::Session;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let file = std::fs::File::options().read(true).write(true).open("image.iso")?;
let mut session = Session::open(file)?;
session.tree_mut().add_file("notes.txt", Content::bytes("added later"))?;
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
move into a relocation directory (`rr_moved` unless
`RockRidge::with_relocation` names another) and appear in their real place
to Rock Ridge readers. A root directory with that name is reused and keeps
its own entries; a root file with that name fails.
`Relocation::Reject` fails instead. Joliet and enhanced trees keep the real
hierarchy.

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
