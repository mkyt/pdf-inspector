#!/usr/bin/env bash
# Regenerate the test fixtures. Requires python3 and qpdf.
set -euo pipefail
cd "$(dirname "$0")"
python3 -I - <<'PY'
import zlib
objs = {}
objs[1] = b"<< /Type /Catalog /Pages 2 0 R /Outlines 9 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> >>"
for i, n in enumerate((3, 4, 5)):
    objs[n] = b"<< /Type /Page /Parent 2 0 R /Contents %d 0 R >>" % (10 + i)
objs[6] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
objs[7] = b"<< /Title (Fixture \\(test\\)) /Author <FEFF00410042> /Producer (pdf-inspector) >>"
objs[9] = b"<< /Type /Outlines /Count 0 >>"
streams = {}
for i in range(3):
    text = b"BT /F1 24 Tf 72 700 Td (Page %d) Tj ET\n" % (i + 1)
    streams[10 + i] = (b"<< /Length %d /Filter /FlateDecode >>", zlib.compress(text * 20))
out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
offsets = {}
for n in sorted(set(objs) | set(streams)):
    offsets[n] = len(out)
    if n in streams:
        d, data = streams[n]
        out += b"%d 0 obj\n" % n + d % len(data) + b"\nstream\n" + data + b"\nendstream\nendobj\n"
    else:
        out += b"%d 0 obj\n" % n + objs[n] + b"\nendobj\n"
size = max(offsets) + 1
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f\r\n" % size
for n in range(1, size):
    out += (b"%010d 00000 n\r\n" % offsets[n]) if n in offsets else b"0000000000 00000 f\r\n"
out += b"trailer\n<< /Size %d /Root 1 0 R /Info 7 0 R /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>] >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("base.pdf", "wb").write(out)

# Incremental update: change the title and add an object.
upd = bytearray(out)
o7, o14 = len(upd), 0
upd += b"7 0 obj\n<< /Title (Updated) /Producer (pdf-inspector) >>\nendobj\n"
o14 = len(upd)
upd += b"14 0 obj\n(new object)\nendobj\n"
x2 = len(upd)
upd += b"xref\n0 1\n0000000000 65535 f\r\n7 1\n%010d 00000 n\r\n14 1\n%010d 00000 n\r\n" % (o7, o14)
upd += b"trailer\n<< /Size 15 /Root 1 0 R /Info 7 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (xref, x2)
open("incremental.pdf", "wb").write(upd)

# Broken: wrong startxref and a wrong /Length.
broken = bytes(out).replace(b"startxref\n%d" % xref, b"startxref\n%d" % (xref + 7))
i = broken.index(b"/Length ")
j = broken.index(b" ", i + 8)
broken = broken[:i] + b"/Length 5" + broken[j:]
open("broken.pdf", "wb").write(broken)
PY
# Schema problems (pdfi-schema): one of each kind the validator reports.
python3 -I - <<'PY'
objs = {
    1: b"<< /Type /Catalog /Pages 2 0 R /PageLayout /Sideways /Outlines << /Type /Outlines /Count 0 >> /MarkInfo << /Marked true >> /Metadata 99 0 R >>",
    2: b"<< /Type /Pages /Kids [3 0 R] >>",
    3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612] /Resources << >> /Rotate 45 /Contents 7 /Foo 1 >>",
    4: b"<< /Title (Schema problems) /CreationDate (2024-01-31) >>",
}
out = bytearray(b"%PDF-1.3\n%\xe2\xe3\xcf\xd3\n")
offsets = {}
for n in sorted(objs):
    offsets[n] = len(out)
    out += b"%d 0 obj\n" % n + objs[n] + b"\nendobj\n"
size = max(offsets) + 1
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f\r\n" % size
for n in range(1, size):
    out += b"%010d 00000 n\r\n" % offsets[n]
out += b"trailer\n<< /Size %d /Root 1 0 R /Info 4 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("schema.pdf", "wb").write(out)
PY
qpdf --object-streams=generate base.pdf objstm.pdf
qpdf --linearize base.pdf linearized.pdf
qpdf --allow-weak-crypto --encrypt user owner 40 -- base.pdf rc4-40.pdf
qpdf --allow-weak-crypto --encrypt user owner 128 --use-aes=n -- base.pdf rc4-128.pdf
qpdf --allow-weak-crypto --encrypt user owner 128 --use-aes=y -- base.pdf aes-128.pdf
qpdf --encrypt user owner 256 -- base.pdf aes-256.pdf
qpdf --encrypt "" owner 256 -- base.pdf aes-256-nopw.pdf
qpdf --object-streams=generate --encrypt user owner 256 -- base.pdf aes-256-objstm.pdf
ls -l *.pdf
