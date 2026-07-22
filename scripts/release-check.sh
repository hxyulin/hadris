#!/usr/bin/env bash
set -euo pipefail

expected_version="${1:-2.0.0-rc.3}"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "release check failed: working tree is not clean" >&2
  exit 1
fi

for tool in cargo git rg python3; do
  command -v "$tool" >/dev/null || {
    echo "release check failed: missing required tool: $tool" >&2
    exit 1
  }
done

manifest_versions="$(rg '^version = ' --glob 'Cargo.toml' crates | sed -E 's/.*version = "([^"]+)"/\1/' | sort -u)"
if [[ "$manifest_versions" != "$expected_version" ]]; then
  echo "release check failed: publishable manifests are not uniformly $expected_version" >&2
  echo "$manifest_versions" >&2
  exit 1
fi

if rg -n '2\.0\.0-rc\.[12]' Cargo.toml Cargo.lock README.md crates website \
  --glob '!**/CHANGELOG.md' \
  --glob '!**/hadris-2.0.0-rc.1-release-notes.md' \
  --glob '!**/hadris-2.0.0-rc.2-release-notes.md'; then
  echo "release check failed: stale active RC1/RC2 reference" >&2
  exit 1
fi

cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
if [[ "${RELEASE_DEPENDENCIES_PUBLISHED:-0}" == "1" ]]; then
  cargo package --workspace --allow-dirty
else
  # Cargo resolves versioned path dependencies through crates.io while
  # packaging, even with --no-verify. The dependency-free first wave can be
  # inspected before publication; rerun with RELEASE_DEPENDENCIES_PUBLISHED=1
  # after publishing in topological order to inspect and verify all archives.
  for crate in hadris-fixed hadris-io hadris-path hadris-macros; do
    cargo package -p "$crate" --allow-dirty
  done
fi

echo "release check passed for $expected_version"
echo "publish in workspace dependency order; do not promote before the soak completes"
