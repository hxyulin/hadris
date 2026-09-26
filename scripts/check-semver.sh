#!/usr/bin/env bash
# Runs cargo-semver-checks for the published library crates (R11).
#
# usage: scripts/check-semver.sh [base-ref]
#   base-ref          the branch a change lands on (default: origin/next,
#                     then next); the fallback baseline is the merge-base of
#                     HEAD and this ref
#   SEMVER_TOOLCHAIN  rustup toolchain to run under (default: stable; 1.88 is too old)
#
# Each crate is checked against its latest 3.x release, the newest tag
# `<crate>-vX.Y.Z` with X >= 3 and no pre-release. A crate without such a
# release (every crate before 3.0.0) is checked against the fallback
# baseline instead, and skipped when it does not exist there.
#
# When the crate's version equals the baseline's, the change must be
# compatible as a minor release: additions pass, breaking changes fail.
# This also holds between 3.0.0 release candidates, where cargo-semver-checks
# would otherwise assume a major change. A deliberate break bumps the
# crate's version in the same change (the next release candidate, or the
# next major after 3.0.0), and cargo-semver-checks then infers the allowed
# change from the two versions.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

toolchain="${SEMVER_TOOLCHAIN:-stable}"
base_ref="${1:-}"
if [[ -z "$base_ref" ]]; then
  for candidate in origin/next next; do
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
fallback="$(git merge-base HEAD "$base_ref")"
echo "fallback baseline: $fallback (merge-base of HEAD and $base_ref)"

if ! cargo "+$toolchain" semver-checks --version >/dev/null 2>&1; then
  echo "cargo-semver-checks is required under the $toolchain toolchain" >&2
  exit 1
fi

crates=(
  hadris
  hadris-common
  hadris-cpio
  hadris-fat
  hadris-fat-raw
  hadris-fs
  hadris-io
  hadris-iso
  hadris-macros
  hadris-ntfs
  hadris-part
  hadris-storage
  hadris-udf
)

metadata="$(cargo metadata --no-deps --format-version 1)"

# Prints the version in the manifest at `rev:path`, or nothing.
manifest_version() {
  git show "$1:$2" 2>/dev/null |
    sed -n '/^\[package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p;}' |
    head -n 1
}

latest_release() {
  git tag --list "$1-v*" |
    sed -n "s/^$1-v\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)$/\1/p" |
    awk -F. '$1 >= 3' |
    sort -t. -k1,1n -k2,2n -k3,3n |
    tail -n 1
}

# Crates that share a baseline and a release type run in one invocation.
declare -A groups=()
for crate in "${crates[@]}"; do
  manifest="$(jq -r --arg name "$crate" '.packages[] | select(.name == $name) | .manifest_path' <<<"$metadata")"
  version="$(jq -r --arg name "$crate" '.packages[] | select(.name == $name) | .version' <<<"$metadata")"
  path="${manifest#"$PWD/"}"
  release="$(latest_release "$crate")"
  if [[ -n "$release" ]]; then
    baseline="$crate-v$release"
    baseline_version="$release"
  else
    baseline="$fallback"
    baseline_version="$(manifest_version "$baseline" "$path")"
    if [[ -z "$baseline_version" ]]; then
      echo "$crate $version: not in $baseline, skipped"
      continue
    fi
  fi
  release_type=infer
  if [[ "$version" == "$baseline_version" ]]; then
    release_type=minor
  fi
  echo "$crate $version against $baseline ($baseline_version), release type $release_type"
  groups["$baseline $release_type"]+=" -p $crate"
done

status=0
for key in "${!groups[@]}"; do
  read -r baseline release_type <<<"$key"
  args=(--baseline-rev "$baseline")
  if [[ "$release_type" != infer ]]; then
    args+=(--release-type "$release_type")
  fi
  # shellcheck disable=SC2086
  cargo "+$toolchain" semver-checks check-release "${args[@]}" ${groups[$key]} || status=1
done
exit "$status"
