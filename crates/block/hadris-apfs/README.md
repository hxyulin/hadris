# hadris-apfs

Incremental APFS support for Hadris.

Current scope:

- no-std friendly on-disk APFS container types
- sync and async container opening over `hadris-storage` block devices
- block 0 superblock parsing and object checksum verification
- foundations for checkpoint/object-map/tree walking

Write support is feature-gated and intentionally starts as scaffolding until the
read path is complete enough to validate allocation state safely.

## Documentation

- [Library API](https://docs.rs/hadris-apfs)
- [APFS inspection CLI](../../tools/hadris-apfs-cli/README.md)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
