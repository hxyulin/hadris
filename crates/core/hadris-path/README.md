# hadris-path

Allocation-free lexical path handling for virtual filesystems, archives, and
disk images. This crate does not access the host filesystem and does not perform
symlink or OS-path canonicalization.

The core API is `no_std` and allocation-free. Enable `alloc` for normalized
owned paths and compatibility splitting helpers.
