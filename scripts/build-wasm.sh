#!/usr/bin/env bash
# Builds calydon.wasm and copies it to res/ (the bytes the tests, the simulation and testnet use).
# Adapted from Preavis scripts/build-wasm.sh: paths are remapped so the bytes do not depend on where
# Cargo's registry or this checkout live. Not yet a reproducible (docker) build.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="--cfg near --remap-path-prefix=$PWD=/calydon --remap-path-prefix=$cargo_home=/cargo"
cargo build --locked --release --target wasm32-unknown-unknown --lib
cp target/wasm32-unknown-unknown/release/calydon.wasm res/calydon.wasm
(cd res && shasum -a 256 calydon.wasm > calydon.wasm.sha256 && cat calydon.wasm.sha256)
