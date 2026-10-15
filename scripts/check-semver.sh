#!/usr/bin/env bash
# Runs cargo-semver-checks for the published library crates (R11).
#
# usage: scripts/check-semver.sh [base-ref]
#   base-ref          the branch a change lands on (default: origin/main,
#                     then main); the fallback baseline is the merge-base of
#                     HEAD and this ref
#   SEMVER_TOOLCHAIN  rustup toolchain to run under (default: stable; 1.88 is too old)
#
# Each crate is checked against its latest 3.x release, the newest tag
# `<crate>-vX.Y.Z` with X >= 3 and no pre-release. A crate without such a
# release (every crate before 3.0.0) is checked against the fallback
# baseline instead, and skipped when it does not exist there.
#
# When the crate's version equals the baseline's and that version is tagged
# as released (`<crate>-v<version>`), the change must be compatible as a
# minor release: additions pass, breaking changes fail. This also holds
# between 3.0.0 release candidates, where cargo-semver-checks would otherwise
# assume a major change. A deliberate break bumps the crate's version in the
# same change (the next release candidate, or the next major after 3.0.0),
# and cargo-semver-checks then infers the allowed change from the two
# versions. A version that was never released may still break, since nothing
# depends on it, so it never fails the check. When an earlier pre-release of
# the same version is tagged (`<crate>-v3.0.0-rc.1` for 3.0.0-rc.2 or 3.0.0),
# the crate is compared against it as a minor release and breaking changes
# are reported as warnings; otherwise it is skipped. The preview and internal crates
# (hadris-apfs, hadris-ntfs, hadris-common) are exempt from semver within
# 3.x: a minor bump may break their API, so it is checked as a major.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

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
fallback="$(git merge-base HEAD "$base_ref")"
echo "fallback baseline: $fallback (merge-base of HEAD and $base_ref)"

if ! cargo "+$toolchain" semver-checks --version >/dev/null 2>&1; then
  echo "cargo-semver-checks is required under the $toolchain toolchain" >&2
  exit 1
fi

crates=(
  hadris
  hadris-apfs
  hadris-common
  hadris-cpio
  hadris-cpio-raw
  hadris-fat
  hadris-fat-raw
  hadris-fs
  hadris-io
  hadris-iso
  hadris-iso-raw
  hadris-macros
  hadris-ntfs
  hadris-part
  hadris-storage
  hadris-udf
  hadris-udf-raw
)

exempt=" hadris-apfs hadris-common hadris-ntfs "

metadata="$(cargo metadata --no-deps --format-version 1)"

# Prints the version in the manifest at `rev:path`, or nothing.
manifest_version() {
  local manifest
  if ! manifest="$(git show "$1:$2" 2>/dev/null)"; then
    return 0
  fi
  sed -n '/^\[package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p;}' <<<"$manifest" |
    head -n 1
}

latest_release() {
  git tag --list "$1-v*" |
    sed -n "s/^$1-v\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)$/\1/p" |
    awk -F. '$1 >= 3' |
    sort -t. -k1,1n -k2,2n -k3,3n |
    tail -n 1
}

# Prints the newest tagged pre-release of version `$2`, or of the version
# `$2` is a pre-release of, that sorts before `$2` and is an ancestor of
# HEAD, or nothing.
previous_prerelease() {
  local core="${2%%-*}" tag candidate
  while read -r tag; do
    candidate="${tag#"$1-v"}"
    if [[ -z "$tag" || "$candidate" == "$2" ]]; then
      continue
    fi
    # Every pre-release of a version precedes it; two pre-releases compare by
    # version sort.
    if [[ "$2" == *-* && "$(printf '%s\n%s\n' "$candidate" "$2" | sort -V | head -n 1)" != "$candidate" ]]; then
      continue
    fi
    if git merge-base --is-ancestor "$tag^{commit}" HEAD; then
      echo "$candidate"
      return 0
    fi
  done < <(git tag --list "$1-v$core-*" --sort=-v:refname)
}

# Crates that share a baseline and a release type run in one invocation.
declare -A groups=()
# Unreleased crates reported against an earlier pre-release, by its commit.
declare -A previews=()
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
    if [[ -z "$(git tag --list "$crate-v$version")" ]]; then
      previous="$(previous_prerelease "$crate" "$version")"
      if [[ -n "$previous" ]]; then
        echo "$crate $version: not released yet, reported against $crate-v$previous without failing"
        previews["$(git rev-parse "$crate-v$previous^{commit}")"]+=" -p $crate"
      else
        echo "$crate $version: not released yet, breaking changes allowed, skipped"
      fi
      continue
    fi
    release_type=minor
  elif [[ "$exempt" == *" $crate "* && "$(cut -d. -f1-2 <<<"$version")" != "$(cut -d. -f1-2 <<<"$baseline_version")" ]]; then
    release_type=major
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
for commit in "${!previews[@]}"; do
  # A reported break only warns; a run that fails without a semver summary
  # (a tool or baseline error) fails the check.
  log="$(mktemp)"
  # shellcheck disable=SC2086
  if ! cargo "+$toolchain" semver-checks check-release --baseline-rev "$commit" --release-type minor ${previews[$commit]} 2>&1 | tee "$log"; then
    if grep -q "semver requires new" "$log"; then
      message="breaking changes since the previous pre-release of${previews[$commit]//-p /}; allowed until it is released"
      if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
        echo "::warning title=semver-checks (informational)::$message"
      else
        echo "warning: $message"
      fi
    else
      echo "semver-checks failed against the previous pre-release of${previews[$commit]//-p /}" >&2
      status=1
    fi
  fi
  rm -f "$log"
done
exit "$status"
