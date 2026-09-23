#!/usr/bin/env bash
# Runs cargo-semver-checks for the published library crates against the
# merge-base of HEAD and the V2 line (R11). Report-only in CI until 3.0.0-rc.1.
#
# usage: scripts/check-semver.sh [base-ref]
#   base-ref          ref to take the merge-base with (default: origin/main, then main)
#   SEMVER_TOOLCHAIN  rustup toolchain to run under (default: stable; 1.88 is too old)
set -euo pipefail

toolchain="${SEMVER_TOOLCHAIN:-stable}"
base_ref="${1:-}"
if [[ -z "$base_ref" ]]; then
  for candidate in origin/main main; do
    if git rev-parse --verify --quiet "$candidate^{commit}" >/dev/null; then
      base_ref="$candidate"
      break
    fi
  done
fi
if [[ -z "$base_ref" ]]; then
  echo "no base ref found; pass one explicitly" >&2
  exit 2
fi

baseline="$(git merge-base HEAD "$base_ref")"
echo "semver baseline: $baseline (merge-base of HEAD and $base_ref)"

if ! cargo "+$toolchain" semver-checks --version >/dev/null 2>&1; then
  echo "cargo-semver-checks is required under the $toolchain toolchain" >&2
  exit 1
fi

crates=(
  hadris
  hadris-block
  hadris-cd
  hadris-common
  hadris-cpio
  hadris-fat
  hadris-io
  hadris-iso
  hadris-macros
  hadris-ntfs
  hadris-optical
  hadris-part
  hadris-path
  hadris-storage
  hadris-udf
)

args=()
for crate in "${crates[@]}"; do
  args+=(-p "$crate")
done

cargo "+$toolchain" semver-checks check-release --baseline-rev "$baseline" "${args[@]}"
