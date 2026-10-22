#!/usr/bin/env bash
# Build pdfi-wasm and its JS glue into apps/web/src/wasm (used by the
# browser-only mode and by the wasm conformance test).
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli at the version
# pinned in crates/pdfi-wasm/Cargo.toml.
set -euo pipefail
cd "$(dirname "$0")/.."

want=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/pdfi-wasm/Cargo.toml)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [[ "$have" != "$want" ]]; then
  echo "wasm-bindgen-cli $want is required (found: ${have:-none})." >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

profile=${PROFILE:-release}
flag=--release
[[ "$profile" == dev ]] && flag=
cargo build -p pdfi-wasm --target wasm32-unknown-unknown $flag
dir=$([[ "$profile" == dev ]] && echo debug || echo release)
wasm-bindgen --target web --out-dir apps/web/src/wasm \
  "target/wasm32-unknown-unknown/$dir/pdfi_wasm.wasm"
