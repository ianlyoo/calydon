#!/usr/bin/env bash
# Builds the contract from a commit inside a pinned linux/amd64 Rust image, so the bytes don't depend
# on the host (Cargo's crate hashes include the host's target triple). Embeds NEP-330 build info
# (image, command, commit), readable on chain through `contract_source_metadata`. Needs Docker.
# On an arm64 host Docker emulates amd64, where parallel rustc jobs have crashed under QEMU; there the
# build runs one job at a time unless CALYDON_BUILD_JOBS=<n> says otherwise. The job count doesn't
# change the bytes (codegen-units = 1). Adapted from Preavis scripts/build-reproducible.sh.
# Usage: scripts/build-reproducible.sh [<commit>]   (default HEAD) -> target/reproducible/calydon.wasm
set -euo pipefail
cd "$(dirname "$0")/.."
image="docker.io/library/rust:1.99.1-slim-bookworm@sha256:392f221964669402bf4ae133d2a46c3ae270f64734133f4b2703d40019235c1a"
rev=$(git rev-parse "${1:-HEAD}^{commit}")
if [ -z "${1:-}" ] && ! git diff --quiet HEAD -- src Cargo.toml Cargo.lock rust-toolchain.toml scripts/build-wasm.sh; then
  echo "the contract's inputs have uncommitted changes; this builds committed source only" >&2
  exit 1
fi
case "$(uname -m)" in x86_64 | amd64) jobs="${CALYDON_BUILD_JOBS:-}" ;; *) jobs="${CALYDON_BUILD_JOBS:-1}" ;; esac
jobs_env=()
[ -n "$jobs" ] && jobs_env=(-e "CARGO_BUILD_JOBS=$jobs")
out="$PWD/target/reproducible"
rm -rf "$out" && mkdir -p "$out"
echo "building commit $rev in $image${jobs:+ with CARGO_BUILD_JOBS=$jobs}" >&2
git archive "$rev" | docker run --rm -i --platform linux/amd64 ${jobs_env[@]+"${jobs_env[@]}"} \
  -e NEP330_BUILD_INFO_BUILD_ENVIRONMENT="$image" \
  -e NEP330_BUILD_INFO_BUILD_COMMAND='["bash","scripts/build-wasm.sh"]' \
  -e NEP330_BUILD_INFO_CONTRACT_PATH="" \
  -e NEP330_BUILD_INFO_SOURCE_CODE_SNAPSHOT="git+https://github.com/ianlyoo/calydon?rev=$rev" \
  -v "$out":/out "$image" \
  bash -c "mkdir /calydon && cd /calydon && tar -x && bash scripts/build-wasm.sh && cp target/wasm32-unknown-unknown/release/calydon.wasm /out/ && chown $(id -u):$(id -g) /out/calydon.wasm"
sha256sum "$out/calydon.wasm" 2>/dev/null || shasum -a 256 "$out/calydon.wasm"
