# Hadris performance & caching notes

Status: findings and roadmap; **no performance code is planned for 2.0**.

Caching and performance were deliberately deferred out of the 2.0 line. This
note records what exists today and where future work should go, so the decisions
are not re-derived later. It complements the roadmap's position that caching
should become a `hadris-storage` adapter "only after the base interface is
stable" ([`hadris-2-roadmap.md`](hadris-2-roadmap.md)).

## What exists

- **FAT sector cache (sync only).** `hadris-fat` has a feature-gated
  (`cache`) write-back LRU sector cache (`crates/block/hadris-fat/src/cache.rs`).
  Built-in `FatVolume` operations route FAT-table reads/writes through it when
  installed. It is **sync only** — the async path silently bypasses the cache
  (deferred as issue #27 / "phase C5b").
- **ISO criterion benches.** `hadris-iso` has `benches/iso_benchmarks.rs` plus a
  timing comparison against xorriso.
- **Allocation-free `IsoReader`.** Reads and streams without allocating, but does
  **no buffering** — every access re-seeks and re-reads from the source.

## Findings / opportunities (post-2.0)

1. **Async FAT cache parity.** The cheapest concrete win: give the async `FatVolume`
   the same write-back cache the sync path has. Bounded scope, real benefit.
2. **`IsoReader` scratch buffer.** A caller-supplied window/scratch buffer would
   cut repeated seeks for sequential directory and file reads while staying
   `no_std`- and allocation-free at the library level.
3. **`hadris-storage` is an unused island.** Its `BlockDevice` /
   `BlockDeviceMut` traits (`crates/core/hadris-storage/src/sync.rs`) are consumed
   by **no format crate** — FAT, ISO, UDF, and partitioning all sit directly on
   `hadris-io`'s `Read + Seek`. A generic block-cache or instrumentation adapter
   written against `BlockDevice` would therefore benefit nobody today. The real
   architectural question is whether block-oriented formats should adopt
   `BlockDevice` so a single cache/instrumentation layer can serve all of them.
   That is a 2.x design question, not a 2.0 change.
4. **No FAT benchmarks.** ISO has criterion benches; FAT has none. Adding FAT
   criterion benches is the right first step before any FAT performance work, so
   changes are measured rather than assumed.

## Guidance

Do not add a caching layer to a format crate speculatively. Establish the
`hadris-storage` consumer story (finding 3) and a benchmark baseline (finding 4)
first; then adapters have both a place to live and a way to prove their value.
