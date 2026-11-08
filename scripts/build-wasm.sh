#!/usr/bin/env bash
# Builds calydon.wasm for this host into target/wasm32-unknown-unknown/release/. Paths are remapped so
# the bytes do not depend on where Cargo's registry or this checkout live; the host's architecture
# still matters, so the committed res/calydon.wasm comes from scripts/build-reproducible.sh.
# Adapted from Preavis scripts/build-wasm.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="--cfg near --remap-path-prefix=$PWD=/calydon --remap-path-prefix=$cargo_home=/cargo"
exec cargo build --locked --release --target wasm32-unknown-unknown --lib
