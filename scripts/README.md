# Repository tooling

Use Python 3.11 or newer for Python checks. The Nix shell supplies Python,
prek and external image tools; Rust toolchains are managed by rustup.
Run commands from the repository root.

## Build and API checks

| Tool | Purpose | Entry point |
|---|---|---|
| `check-features.py` | Independently build the MSRV feature tiers in `ci-features.json` | `python3 scripts/check-features.py all` |
| `check-targets.sh` | Build bare-metal tiers, selecting allocation support by target atomics | `scripts/check-targets.sh` |
| `firmware-size.py` | Measure firmware flash, driver state and static stack estimates | `python3 scripts/firmware-size.py --check` |
| `check-semver.sh` | Compare crate APIs with release or branch baselines | `scripts/check-semver.sh` |
| `test-check-semver.py` | Test baseline selection and semver failure propagation | `python3 scripts/test-check-semver.py` |
| `check-non-exhaustive.py` | Enforce the extensibility rule for public types | `python3 scripts/check-non-exhaustive.py` |

The feature and target checks have different jobs: host feature compilation
does not establish that a bare-metal target supports the required atomics.
Firmware measurements need the pinned nightly, llvm-tools and embedded targets;
see [CONTRIBUTING.md](../CONTRIBUTING.md). Stack estimates describe direct calls
and list indirect-call boundaries; they are not runtime stack measurements.

## Documentation, specifications and releases

| Tool | Purpose | Entry point |
|---|---|---|
| `check-docs.py` | Validate Markdown links, README conventions and current dependency/API versions | `python3 scripts/check-docs.py` |
| `check-spec-annotations.py` | Validate source annotations and generated coverage tables | `python3 scripts/check-spec-annotations.py` |
| `check-compliance-catalog.py` | Validate requirement catalogs and evidence references | `python3 scripts/check-compliance-catalog.py` |
| `release-plan.py` | Validate release versions/notes and order package publication | Used by [release.yml](../.github/workflows/release.yml) |
| `test-release-plan.py` | Check independent/joint releases and conflicting existing tags | `python3 scripts/test-release-plan.py` |
| `cli-release.py` | Validate CLI tags, package binaries and resume release-asset uploads | Used by [cli-release.yml](../.github/workflows/cli-release.yml) |
| `test-cli-release.py` | Check CLI archive layout, metadata and upload retries | `python3 scripts/test-cli-release.py` |

The specification checks cover source annotations and requirement catalogs
respectively. Keep both when updating evidence. The checkers expose
`--self-test`; CI runs those tests before checking repository content.
The [release guide](../CONTRIBUTING.md#package-versions-and-releases) describes
the release-plan inputs and dry-run workflow.

## Native fixtures and performance peers

| Tool | Purpose | Entry point |
|---|---|---|
| `apfs-encryption-fixtures.py` | Generate disposable encrypted APFS fixtures using macOS utilities | [APFS encryption guide](../docs/apfs-encryption.md) |
| `test-ntfs.sh` | Run NTFS qualification in a Linux container with FUSE permissions | `scripts/test-ntfs.sh` |
| `build-fatfs-peer.py` | Download, verify, patch and build the pinned ChaN FatFs peer | `python3 scripts/build-fatfs-peer.py` |
| `test-fatfs-peer.py` | Check the peer helper's creation/extraction and failure behavior | `python3 scripts/test-fatfs-peer.py tests/target/chan-fatfs/chan-fatfs` |

Full NTFS mount qualification needs Docker, `/dev/fuse` and mount permissions.
Ordinary CI checks formatter-generated NTFS volumes and skips unavailable
mount scenarios. Native APFS fixtures need macOS; software keybag tests do
not qualify Apple-silicon hardware FileVault.

CI performance smoke checks verify that harnesses run. Timed comparisons,
RSS, I/O counts and profiling remain separate measurements described in the
[performance](../docs/performance.md) and
[peer performance](../docs/peer-performance.md) guides.

Fuzz commands and corpus generation live under [fuzz/](../fuzz/README.md).
Store generated images, profiles and scratch reports under ignored `output/`
or a worktree output directory; retain published measurement evidence in
[docs/benchmarks/](../docs/benchmarks/README.md).
