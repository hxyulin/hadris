# Function tracing

The filesystem and partition crates offer an opt-in `tracing` feature:
`hadris-fat`, `hadris-iso`, `hadris-udf`, `hadris-apfs`, `hadris-ntfs`,
`hadris-cpio` and `hadris-part`. The umbrella `hadris` forwards it to enabled
formats without enabling additional format features. ISO forwards it to its
partition writer, and UDF forwards it to ISO for bridge planning and emission.
Applications use the same subscriber for every format.

```toml
[dependencies]
hadris-fat = { version = "3.0.0-rc.1", features = ["tracing"] }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

The application installs a subscriber; Hadris never installs a global one:

```rust,ignore
tracing_subscriber::fmt()
    .with_env_filter("hadris=trace")
    .with_file(true)
    .with_line_number(true)
    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
    .init();
```

Spans use the `TRACE` level and targets `hadris::fat`, `hadris::exfat`,
`hadris::iso`, `hadris::udf`, `hadris::apfs`, `hadris::ntfs`, `hadris::cpio`
and `hadris::part`. FAT and exFAT also have `::embedded` children. Each span carries its function name, module,
source file and line. Selected operations also record node/directory IDs,
offsets, requested byte counts, lengths and allocation counts. Devices,
buffers, file contents, paths, credentials and error payloads are skipped, so instrumentation
does not add `Debug` or `Display` requirements to generic device types.
Subscriber-specific layers can export these spans for visualization or
profiling. Async spans enter on each poll and exit when it returns; no span
guard stays entered across an await or after cancellation.

`tracing` explicitly enables `std` (which already enables `alloc` in Hadris).
It is disabled by default. The plain `sync`/`async` and `alloc` tiers remain
available without it; enabling tracing selects the hosted tier even with
`default-features = false`. Cargo unifies features for a crate, so every
dependency must leave tracing and `std` disabled for a no-allocator firmware
build. No custom observer API or global callback registry is needed.

```bash
cargo check -p hadris-fat --no-default-features --features sync,write
cargo check -p hadris-fat --no-default-features --features alloc,async,write
cargo check -p hadris-fat --no-default-features --features tracing,sync,async,write
cargo test -p hadris-fat --all-features --test tracing
```

Measure normal performance with tracing disabled. Enabling it adds callsite
checks and retained span state in async futures, and collecting spans adds
subscriber overhead, even though filesystem results and on-disk ordering
are unchanged. The future-size budget tests are ignored in tracing builds
and run separately with tracing disabled in CI; their size limits are unchanged.

## Reading the spans

The nested spans show how an operation is divided between chain growth,
allocation and data transfer. Repeated `allocate(count = 1)` spans during
small appends identify incremental growth; `cover` without an allocation
shows that the existing chain was sufficient. Requested byte counts and
offsets let you compare small writes with block-aligned transfers.

With `FmtSpan::CLOSE` and timestamps enabled, the formatter reports busy
time (while the span was entered) and idle time (while it existed without
being entered). For async calls, idle time can include device waits and
executor scheduling. Busy time is elapsed time, not a hardware CPU-cycle
measurement. Parent timings include nested calls and should not be summed
with their children. See the
[subscriber documentation](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.SubscriberBuilder.html#method.with_span_events).

These spans do not count device requests, distinguish FAT/metadata/data
traffic, measure heap/stack/flash, or record explicit success, failure and
cancellation outcomes. A closed span alone does not mean the call succeeded.
Use a counting device for exact I/O totals and the firmware-size script for
embedded resource costs. Run the embedded driver on a host with tracing to
inspect its control flow, then benchmark and build firmware with tracing
disabled.

## Coverage by format

| Target | Operations |
| --- | --- |
| `hadris::fat`, `hadris::exfat` | Mount, file and directory operations, allocation, sync and formatting; hosted and embedded APIs |
| `hadris::iso` | Mount, file and directory operations, raw and extent reads, planning, placement, directory construction, image writes and sessions |
| `hadris::udf` | Mount, file and directory operations, raw and extent reads, planning, image and ISO bridge writes |
| `hadris::apfs` | Mount and password mount, file and directory operations, container opening, unlocking, B-tree traversal, object-map lookup and native reads |
| `hadris::ntfs` | Mount, file and directory operations, stream listing and named stream reads |
| `hadris::cpio` | Entry and segment iteration, streaming reads, tree reads, planning, append and streaming writes |
| `hadris::part` | Partition scanning, table reads, writes and creation |

Shared sources instrument each supported sync, Send async and local async mode.
This is operation and phase tracing: individual block transfers and every parser
helper are not instrumented. Format-neutral I/O and storage traits remain free
of span policy; wrap a device to collect transfer counters.

APFS password mounts and unlocking use `skip_all`, with no return or error
recording. Passwords, crypto-user identifiers, keys and decrypted bytes never
become span fields. Failed calls still produce spans, so use the returned result
to determine success.
