# hadris-iso-raw

On-disk layouts and I/O-free codecs extracted from `hadris-iso`.
Works with `no_std`, without an allocator, device, or sync/async feature.
The package starts at version 0.1.0 and versions independently of the driver.

```toml
[dependencies]
hadris-iso-raw = "0.1.0"
```

Every item is at the crate root. `hadris-iso` does not re-export this
crate: its `hadris_iso::raw` module holds only the layouts its own API
returns (`VolumeDescriptor` from `IsoFs::descriptor`, `BootSectionEntry`
from `CatalogEntry::raw`, and the types of their fields and methods). Those
items are frozen for `hadris-iso` 3.x: a release of this crate may add to
them but makes no breaking change to them. The rest of this crate can
change in a 0.x minor release without a new major version of `hadris-iso`.
Use `hadris-iso` for mounting, filesystem access and image writing.

## Documentation

- [API reference](https://docs.rs/hadris-iso-raw/0.1.0)
- [Hadris documentation](https://hxyulin.github.io/hadris/)
- [`hadris-iso`](../hadris-iso), the driver using these codecs

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
