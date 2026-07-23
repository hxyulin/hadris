#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="hadris-ntfs-test"

docker build \
  --file "$repo_root/crates/block/hadris-ntfs/Dockerfile.test" \
  --tag "$image" \
  "$repo_root/crates/block/hadris-ntfs"

command=(
  cargo test -p hadris-ntfs --features sync --test read --
  --nocapture --test-threads=1
)
if (( $# > 0 )); then
  command=("$@")
fi

docker run --rm \
  --cap-add SYS_ADMIN \
  --device /dev/fuse \
  --security-opt apparmor=unconfined \
  --volume "$repo_root:/workspace" \
  --volume hadris-cargo-registry:/usr/local/cargo/registry \
  --volume hadris-cargo-git:/usr/local/cargo/git \
  --volume hadris-ntfs-target:/workspace/target \
  "$image" \
  "${command[@]}"
