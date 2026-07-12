# hadris-storage

Foundational block-storage geometry, capability traits, and adapters for Hadris.

This crate is format-neutral and does not assume 512-byte sectors. It provides
checked logical-block addressing for block filesystems, partition tables, device
images, and bounded storage views.

The initial API includes synchronous and asynchronous block-device traits plus
adapters over Hadris seekable byte streams. Higher-level caching and partition
views will be added after the base interface has been exercised by multiple
format crates.
