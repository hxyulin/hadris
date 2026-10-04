# Embedded FAT rename recovery

An interrupted rename could publish its destination and leave its source visible
through the next `sync`. Both entries then owned the same clusters. A regression
run against `dbcb38fe` reproduced crosslinks for every cluster of a 9,000-byte
file after canceling rename and completing recovery.

The embedded driver now retains a fixed-size rename record until cleanup
finishes. The final short-entry write publishes the move. Recovery compares the
complete destination short entry with the expected bytes and records its decision
before making further writes. A published move completes source cleanup,
updates a moved directory's `..` entry, and redirects open file slots to the new
entry. An incomplete destination is removed. Repeated cancellation or I/O failure
during recovery preserves the chosen outcome and allows another retry.

The destination short slot is cleared before publication begins. This prevents
an ignored, identical short entry after a directory End marker from being mistaken
for a newly published move. Rollback restores the original End marker; successful
insertion first places a new End marker after the destination when that slot exists,
and committed recovery reapplies it. Read-only lookups before recovery cannot expose
the previously hidden records beyond the destination run.
Hidden trailing records therefore remain hidden after either outcome.

This recovery record lives in the mounted driver. The next mutating operation or
`sync` retries it while the driver remains alive. It does not provide a persistent
transaction across driver destruction, remount, or power loss.

## Validation

The two new regression matrices cover four supported embedded geometries:
FAT12, FAT16, FAT32, and FAT12 with 4 KiB filesystem sectors over a 512-byte block
device. Each geometry exercises nonempty files and directories, both within one
directory and across directories. Fixtures include an identical expected short
entry hidden after End and another hidden entry beyond the destination run.

For each scenario, tests interrupt rename at every asynchronous await or fail
each device write. They then interrupt or fail recovery at every boundary for both
rollback and committed outcomes. Recovery must produce exactly one visible name,
retain file contents, update directory parent links, and allow a dirty file handle
opened before rename to append and close successfully. An independent raw-image
structural checker rejects crosslinks and other corruption.

Final checks:

- `cargo test -p hadris-fat --features async --test embedded`: 21 passed.
- Warning-denying no-default-feature checks with `sync,write` and `async,write`: passed.
- Firmware size and stack checks on all three configured bare-metal targets: passed.

The inline state grows from 936 to 1,040 bytes with four file slots, remaining
below the 2 KiB driver-state limit. The mount stack remains below 2 KiB, and the
largest Hadris frame remains below 1 KiB.

| Target | Driver state | Mount stack | Largest synchronous FAT frame | FAT logger flash |
|---|---:|---:|---:|---:|
| `thumbv6m-none-eabi` | 1,040 B | 1,944 B | 912 B | 42,572 B |
| `thumbv7em-none-eabihf` | 1,040 B | 1,872 B | 856 B | 41,452 B |
| `riscv32imc-unknown-none-elf` | 1,040 B | 1,856 B | 848 B | 49,392 B |

The enforced Cortex-M4 logger flash ceiling is 44 KiB. Its existing 20 KiB target
remains open; this change stays within the ceiling. These figures use the pinned
`nightly-2026-09-04`, size optimization, and fat LTO. They describe the script's
static call-graph measurements rather than a general bound for caller code.
