#!/usr/bin/env bash
# Builds the prototype in a privileged Linux container and runs the mount
# scenarios there. Needs Docker with /dev/fuse (OrbStack or Docker Desktop).
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/../.." && pwd)"
image="${HADRIS_FUSE_IMAGE:-rust:1-bookworm}"
exec docker run --rm --privileged --device /dev/fuse \
    -v "$repo":/src \
    -v hadris-fuse-cargo:/usr/local/cargo/registry \
    -v hadris-fuse-target:/target \
    -e CARGO_TARGET_DIR=/target \
    -w /src/experiments/fuse-prototype \
    "$image" bash -c '
        set -euo pipefail
        export RUSTUP_TOOLCHAIN="$(rustup default | cut -d" " -f1)"
        apt-get update -qq >/dev/null
        DEBIAN_FRONTEND=noninteractive apt-get install -y -qq fuse3 dosfstools >/dev/null
        cargo build --release --quiet
        exec bash scripts/scenarios.sh /target/release/hadris-fuse-prototype
    '
