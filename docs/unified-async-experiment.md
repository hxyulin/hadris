# Unified asynchronous driver experiment

2026-10-07, based on PR #283 commit `3a5bd700`.

Branch: `experiment/unified-async`. The PR branch and production APIs are
unchanged. This is a small model of the driver/device relationship, not a port
of the real FAT or ISO implementations.

## Results

| Requirement | Result |
|---|---|
| One concrete async driver type for local and Send devices | Passed in the model: `Fs<D>` |
| One implementation of the read algorithm | Passed: inherent async methods |
| A local device with suspending non-Send futures | Passed: Rc state retained across Pending |
| A Send device implemented once | Passed: implement `SendDevice`; a blanket bridge supplies `Device` |
| Generic Send caller using one stronger bound | Passed: `F: SendFileSystem`, with qualified calls |
| Normal method syntax with the stronger subtrait | Failed: duplicate method declarations are ambiguous |
| Normal method syntax with independent contracts | Passed: the `peer` alternative |
| Borrowed Send device without `'static` | Passed: `Fs<&mut Memory<'_>>` |
| Borrowed local device without `'static` | Passed with a `Borrowed` adapter |
| Send read future executed on another thread | Passed using scoped threads and borrowed data |
| Allocator-free trait/driver layer | Passed: library is `no_std` and uses no boxing or allocation |
| Universal reference and partition adapters alongside the blanket bridge | Failed: overlapping implementations |

## Mechanism

There are two device contracts. `Device` permits local futures; `SendDevice`
requires both Send device state and Send operation futures. A blanket
`impl<D: SendDevice> Device for D` delegates directly to the stronger methods.
It needs no allocation, no async wrapper and no explicit refinement lint
exception.

The sole `Fs<D>` implementation calls `Device` operations. When `D` satisfies
the stronger contract, the compiler resolves the blanket bridge and proves
that the driver's inherent futures are Send. An ordinary Device implementation
can instead retain Rc state and return non-Send futures. Both use exactly the
same read algorithm, including a private async helper and repeated awaits.

A `SendFileSystem: FileSystem + Send` subtrait redeclares the async methods with
Send return futures. Its implementation for `Fs<D: SendDevice>` forwards the
inherent futures directly. Generic callers use one bound and can pass their
operation future to code requiring Send. They must call
`SendFileSystem::read(fs, offset, buf)` because `fs.read(...)` is ambiguous
between the base and refined trait methods.

The `peer` alternative keeps the two contracts independent. It permits ordinary
`fs.read(...)` in generic Send code, matching today's separate trait contracts,
while still using the same concrete driver. Its stronger bound does not imply
the weaker filesystem trait, so generic code needing both must state both.

## Costs and unresolved integration

On macOS/aarch64 with Rust 1.88.0 in the debug profile:

- `Fs<Memory>` is 16 bytes, equal to its slice-backed device.
- `Fs<LocalMemory>` is 24 bytes, equal to its device including an Rc counter.
- The inherent read future and the refined trait's forwarded read future are
  both 168 bytes. An initial forwarding async block added a frame; returning
  the existing future directly removed it.

These are model type sizes, not real-driver RSS, throughput or binary-size
measurements. The demonstration executor allocates a waker; the driver and
trait adapters introduce no allocation.

The strongest integration obstacle is coherence. The blanket bridge overlaps
with both a universal `Device for &mut D` implementation and a generic
partition adapter implementing local and Send contracts. The model handles
local borrowing with a dedicated Borrowed adapter. Hadris has many existing
storage/I/O adapters, so this cannot be copied wholesale into its current
traits. We need a coherent adapter strategy and compatibility review first.

An empty Send marker on a local filesystem is insufficient, even for a concrete
Send filesystem whose operation creates Rc state across an await. Both failures
are checked as compile-fail doctests. No named associated future types, nightly
features or unsafe Send implementations are used.

## Reproduce

```bash
cargo run -p hadris-example-unified-async
cargo test -p hadris-example-unified-async
cargo check -p hadris-example-unified-async --lib --no-default-features \
  --target thumbv6m-none-eabi
```

Three runtime tests and six compile-fail doctests pass on the Rust 1.88 MSRV.
The negative cases check insufficient Send bounds, marker traits, ambiguous
refined methods, non-Send operation state and coherence conflicts. The library
also builds for thumbv7em and riscv32imc without allocation.

## Recommendation

Pursue one concrete async driver type first, retaining separate trait contracts
if needed for generic caller ergonomics. Before changing PR #283, prototype a
real reader and the partition/borrowed-device adapters against this pattern.
The stronger subtrait is technically possible, but its ambiguous methods make
it a weaker ergonomic choice than the independent contracts unless a better
refinement mechanism is found. Keep sync separate from async.
