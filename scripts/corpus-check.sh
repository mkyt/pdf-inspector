#!/usr/bin/env bash
# Run `pdfi check` over every PDF under a directory and compare page counts
# with qpdf. Usage: scripts/corpus-check.sh <dir> [--decode]
set -u
dir=${1:?usage: corpus-check.sh <dir> [--decode]}
shift
pdfi=${PDFI:-target/release/pdfi}
printf "file\tpdfi_pages\tqpdf_pages\tfailed\tdiags\tstatus\n"
find "$dir" -name '*.pdf' -print0 | sort -z | while IFS= read -r -d '' f; do
  out=$("$pdfi" check "$@" "$f" 2>&1)
  rc=$?
  pages=$(sed -n 's/.*; pages \([0-9]*\).*/\1/p' <<<"$out" | head -1)
  failed=$(sed -n "s/.*failed \([0-9]*\) missing.*/\1/p" <<<"$out" | head -1)
  diags=$(sed -n 's/^diagnostics: \([0-9]*\).*/\1/p' <<<"$out" | head -1)
  q=$(qpdf --show-npages "$f" 2>/dev/null || echo "?")
  status=ok
  [ $rc -ne 0 ] && status="exit$rc"
  [ "${pages:-x}" != "$q" ] && status="${status},pages-mismatch"
  [ "${failed:-0}" != 0 ] && status="${status},objects-failed"
  printf "%s\t%s\t%s\t%s\t%s\t%s\n" "$f" "${pages:-}" "$q" "${failed:-}" "${diags:-0}" "$status"
done
