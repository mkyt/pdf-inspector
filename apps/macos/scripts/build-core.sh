#!/bin/sh
# Build the Rust core for the macOS app: pdfi-ffi as a static library in
# PdfInspectorKit/Frameworks/PdfiCoreFFI.xcframework, and its Swift bindings
# in PdfInspectorKit/Sources/PdfiCore. Both are generated together (the
# bindings check the library's API checksums) and not checked in.
#
#   scripts/build-core.sh           # release (the renderer is slow in debug)
#   scripts/build-core.sh --debug
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
kit="$here/PdfInspectorKit"
target=aarch64-apple-darwin

profile=release
flags=--release
if [ "${1:-}" = "--debug" ]; then
  profile=debug
  flags=
fi

# The app's deployment target (PdfInspectorKit/Package.swift).
export MACOSX_DEPLOYMENT_TARGET=27.0

cargo build --manifest-path "$root/Cargo.toml" -p pdfi-ffi --lib --target "$target" $flags
lib="$root/target/$target/$profile/libpdfi_ffi.a"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
mkdir "$out/include" "$out/swift"
# bindgen OUT_DIR OPTIONS...
bindgen() {
  dir=$1
  shift
  cargo run -q --manifest-path "$root/Cargo.toml" -p pdfi-ffi --features bindgen \
    --bin uniffi-bindgen-swift -- --config "$here/uniffi.toml" "$@" "$lib" "$dir"
}
bindgen "$out/include" --headers --modulemap --module-name PdfiCoreFFI --modulemap-filename module.modulemap
bindgen "$out/swift" --swift-sources

rm -rf "$kit/Frameworks/PdfiCoreFFI.xcframework"
mkdir -p "$kit/Frameworks"
xcodebuild -quiet -create-xcframework -library "$lib" -headers "$out/include" \
  -output "$kit/Frameworks/PdfiCoreFFI.xcframework"

rm -rf "$kit/Sources/PdfiCore"
mkdir -p "$kit/Sources/PdfiCore"
cp "$out/swift/"*.swift "$kit/Sources/PdfiCore/"

echo "PdfiCore: $profile build of pdfi-ffi in $kit"
