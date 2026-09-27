#!/usr/bin/env bash
# Builds the web version (WebAssembly + WebGPU) into dist/.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --target wasm32-unknown-unknown
rm -rf dist
mkdir -p dist
wasm-bindgen --target web --no-typescript --out-dir dist target/wasm32-unknown-unknown/release/camel-eon.wasm
cp web/index.html dist/
