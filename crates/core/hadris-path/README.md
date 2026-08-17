# hadris-path

Allocation-free lexical path handling for virtual filesystems, archives, and
disk images. This crate does not access the host filesystem and does not
perform symlink or OS-path canonicalization.

Use `VPath` when path components come from an image or archive rather than the
host operating system. Separator handling is explicit, and `..` remains a
lexical component until the caller chooses to normalize it.

## Example

```rust
use hadris_path::{Component, Separators, VPath};

let path = VPath::with_separators(
    r"boot\grub/../kernel.efi",
    Separators::SlashOrBackslash,
);
let components: Vec<_> = path.components().collect();
assert_eq!(
    components,
    [
        Component::Normal("boot"),
        Component::Normal("grub"),
        Component::Parent,
        Component::Normal("kernel.efi"),
    ]
);
```

Borrowed path views and component iteration are `no_std` and allocation-free.
Enable `alloc` for normalized owned strings and compatibility helpers.

## Security boundary

Lexical parsing does not make an extraction path safe automatically. Before
writing archive or filesystem entries to a host directory, reject absolute
paths and parent components, then enforce the destination boundary with the
host filesystem API.

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `alloc` | No | Owned normalization and splitting helpers |

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [API reference](https://docs.rs/hadris-path)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
