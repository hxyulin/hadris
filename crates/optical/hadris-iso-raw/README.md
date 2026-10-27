# hadris-iso-raw

On-disk layouts and I/O-free codecs extracted from `hadris-iso`.
Works with `no_std`, without an allocator, device, or sync/async feature.
The package versions independently of the driver.

```toml
[dependencies]
hadris-iso-raw = "0.2.0"
```

Every item is at the crate root. `hadris-iso` does not re-export this
crate: its `hadris_iso::raw` module holds only the layouts its own API
returns (`VolumeDescriptor` from `IsoFs::descriptor`, `BootSectionEntry`
from `CatalogEntry::raw`, and the types of their fields and methods). Those
items are frozen for `hadris-iso` 3.x. The driver stays on one raw 0.x line
for its whole major version. Moving it to another raw minor changes the
identity of the re-exported types and requires a new driver major after
3.0. The other raw APIs can change in a 0.x minor; direct users follow
this crate's version separately.
Use `hadris-iso` for mounting, filesystem access and image writing.

`DirectoryRecord::parse` returns `Ok(None)` for a zero length byte at the
end of a sector. Invalid records return `ParseDirectoryRecordError`, which
distinguishes truncated input, invalid lengths, identifier bounds, padding,
reserved flags, and inconsistent byte-order copies. It implements
`core::error::Error`; callers that previously matched `Err(())` use
`Err(error)` or `Err(_)` instead.

## Documentation

- [API reference](https://docs.rs/hadris-iso-raw/0.2.0)
- [Hadris documentation](https://hxyulin.github.io/hadris/)
- [`hadris-iso`](../hadris-iso), the driver using these codecs

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
