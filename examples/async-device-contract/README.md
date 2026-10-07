# Async device contract experiment

This unpublished package prototypes a common async device contract before
migrating the remaining formats. Existing production APIs are unchanged.

One `BlockDevice` implementation supplies poll hooks and a movable operation
state. Ordinary `read_blocks`, `write_blocks` and `flush` methods return a
concrete future holding `&mut D`, the caller buffer and `D::State`. The
`SendBlockDevice` capability follows automatically when both `D` and its state
are Send. No second device implementation or boxed future is required.

A filesystem value can be Send while its operation futures are non-Send: an
operation state containing `Rc` preserves that distinction. Plain `&mut D`,
nested `Partition<D>` and `Cache<D>` compose through the same contract. Generic
filesystem algorithms can await multiple operations while retaining Send
futures under one stronger device bound, including borrowed non-static devices.

## Run the evidence

```sh
cargo test -p hadris-experiment-async-device-contract
cargo check -p hadris-experiment-async-device-contract --no-default-features
python3.11 examples/async-device-contract/probe_gat.py
python3.11 examples/async-device-contract/probe_iso.py
```

Both probe scripts default to the MSRV, Rust 1.88.0. Use `--toolchain 1.97.1`
to check CI Rust. The ISO script makes a temporary copy of the current ISO
sources and changes the reader's storage alias and stronger device bound. Its
parser and read algorithms are unchanged. It tests the actual allocation-free
reader against a borrowed partition/cache chain and a suspending Rc device.
The existing Send filesystem contract remains usable with the Send chain.
Use `--output-dir /tmp/hadris-iso-poll-probe` to inspect the generated crate.

The alternative GAT probes explain why simply naming each borrowed future is
insufficient for this API on the MSRV. The higher-ranked Send bound imposes an
unwanted static lifetime on borrowed devices. A lifetime-specific bound works
for direct forwarding, but fails for a filesystem algorithm that awaits and
reborrows. The script verifies both expected rejections and the forwarding
control that compiles.

## Costs and limits

Device authors implement `poll_read_blocks`, optional write/flush hooks and
`cancel` instead of implementing async trait methods. The future owns a fresh
operation state; state construction can have costs chosen by the backend.
State must implement Default and Unpin. It cannot retain pointers to its own
movable fields. All hooks receive the same state for one operation.

Safe poll hooks stop accessing caller buffers before returning, including
Pending. Hardware performing I/O between polls needs owned stable buffers.
Drop calls cancellation for a pending operation, but buffer safety must also
hold if a caller forgets the future or invokes poll hooks directly. Cancellation
must not panic, and terminal completion must release operation resources.

The prototype cache holds one fixed 512-byte block and bypasses other block
sizes or multi-block reads. It demonstrates generic composition and
invalidation; it is not the production cache or a performance comparison.

This proves the core contract and ISO reader path. It does not yet implement
all production adapters, arbitrary async-device compatibility, local writers,
sessions, or the other filesystem drivers. Namespace migration remains paused
until the contract direction is accepted. Issue #267 remains open.
