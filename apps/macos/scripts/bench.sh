#!/bin/sh
# Measure the app on a PDF: build it (Release), copy the file where the
# sandbox can read it and run a benchmark of PDFInspector/App/PerfBench.swift.
#
#   scripts/bench.sh FILE [MODE] [OPTIONS...]
#
# MODE: keys (default; ↓ in the operation list), select, scroll, pages,
# objects, bytes. OPTIONS go to the app, like -PerfBenchPage 11 (0-based),
# -PerfBenchSteps 50, -PerfBenchPanes schema,bytes,objects,
# -PerfStderr YES (every request and slow main thread turn as it happens).
# Set NOBUILD=1 to use the last build.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
file=$1
mode=${2:-keys}
shift
[ $# -gt 0 ] && shift

derived="${DERIVED:-$here/build/bench}"
if [ -z "${NOBUILD:-}" ]; then
  xcodebuild -project "$here/PDFInspector.xcodeproj" -scheme PDFInspector -configuration Release \
    -derivedDataPath "$derived" build -quiet
fi
app="$derived/Build/Products/Release/PDF Inspector.app/Contents/MacOS/PDF Inspector"

# Inside the app's container, so the sandbox can read it.
dir="$HOME/Library/Containers/com.juzbox.pdf-inspector/Data/Library/Caches/bench"
mkdir -p "$dir"
cp "$file" "$dir/bench.pdf"

"$app" -PerfBench "$mode" -PerfBenchFile "$dir/bench.pdf" -PerfLog YES -ApplePersistenceIgnoreState YES "$@" 2>/dev/null
