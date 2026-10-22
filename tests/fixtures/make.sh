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
# Semantic model (pdfi-model): a nested page tree (one node without /Count)
# with inherited attributes, labels, outline, named destinations, annotations,
# fonts, images, a form XObject, XMP that disagrees with Info, an orphan.
python3 -I - <<'PY'
import zlib
xmp = b"""<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:pdf="http://ns.adobe.com/pdf/1.3/" pdf:Producer="something else">
<dc:title><rdf:Alt><rdf:li xml:lang="x-default">Model fixture</rdf:li></rdf:Alt></dc:title>
<xmp:CreateDate>2024-01-31T03:34:56Z</xmp:CreateDate>
<xmp:ModifyDate>2024-02-02T00:00:00Z</xmp:ModifyDate>
</rdf:Description></rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"""
objs = {
    1: b"<< /Type /Catalog /Pages 2 0 R /Outlines 30 0 R /PageMode /UseOutlines /Lang (en) /PageLabels << /Nums [0 << /S /r >> 2 << /S /D /P (A-) >>] >> /Names << /Dests 40 0 R >> /Metadata 50 0 R /OpenAction [10 0 R /Fit] >>",
    2: b"<< /Type /Pages /Kids [3 0 R 12 0 R 4 0 R] /Count 5 /MediaBox [0 0 612 792] /Resources << /Font << /F1 20 0 R /F2 21 0 R >> >> >>",
    3: b"<< /Type /Pages /Parent 2 0 R /Kids [10 0 R 11 0 R] /Count 2 /CropBox [10 10 602 782] /Rotate 90 >>",
    4: b"<< /Type /Pages /Parent 2 0 R /Kids [13 0 R 14 0 R] >>",
    10: b"<< /Type /Page /Parent 3 0 R /Contents 15 0 R /Annots [<< /Type /Annot /Subtype /Link /Rect [72 700 200 720] /Dest [12 0 R /XYZ 0 792 null] >> 70 0 R 71 0 R 72 0 R] >>",
    11: b"<< /Type /Page /Parent 3 0 R /Contents 16 0 R >>",
    12: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /TrimBox [290 290 10 10] /UserUnit 2 /Contents 17 0 R >>",
    13: b"<< /Type /Page /Parent 4 0 R /Rotate 45 /Contents [18 0 R 19 0 R] >>",
    14: b"<< /Type /Page /Parent 4 0 R /Resources << /Font << /F1 20 0 R /F4 25 0 R >> /XObject << /Im1 60 0 R /Fm1 64 0 R >> >> >>",
    20: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    21: b"<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Embedded /FirstChar 32 /LastChar 32 /Widths [250] /FontDescriptor 22 0 R /ToUnicode 24 0 R /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [32 /space] >> >>",
    22: b"<< /Type /FontDescriptor /FontName /ABCDEF+Embedded /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 23 0 R >>",
    25: b"<< /Type /Font /Subtype /Type0 /BaseFont /CIDFont-Identity /Encoding /Identity-H /DescendantFonts [26 0 R] >>",
    26: b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 27 0 R >>",
    27: b"<< /Type /FontDescriptor /FontName /CIDFont /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 23 0 R >>",
    28: b"<< /Subtype /Type1 /BaseFont /Courier >>",
    30: b"<< /Type /Outlines /First 31 0 R /Last 33 0 R /Count 3 >>",
    31: b"<< /Title (Chapter 1) /Parent 30 0 R /Next 33 0 R /First 32 0 R /Last 32 0 R /Count 1 /Dest (chap1) >>",
    32: b"<< /Title <FEFF00530065006300740069006F006E00200031002E0031> /Parent 31 0 R /Dest [11 0 R /FitH 700] /F 2 /C [1 0 0] >>",
    33: b"<< /Title (Web) /Parent 30 0 R /Prev 31 0 R /A << /S /URI /URI (https://example.com/) >> >>",
    40: b"<< /Kids [41 0 R] >>",
    41: b"<< /Limits [(chap1) (chap2)] /Names [(chap1) [10 0 R /Fit] (chap2) << /D [12 0 R /Fit] >>] >>",
    70: b"<< /Type /Annot /Subtype /Link /Rect [72 650 200 670] /A << /S /URI /URI (https://example.com/) >> >>",
    71: b"<< /Type /Annot /Subtype /Text /Rect [300 700 320 720] /Contents (Note) /NM (note-1) /F 4 /Popup 72 0 R >>",
    72: b"<< /Type /Annot /Subtype /Popup /Rect [320 600 500 700] /Parent 71 0 R >>",
    80: b"<< /Title (Model fixture) /Author (pdf-inspector) /Producer (make.sh) /CreationDate (D:20240131123456+09'00') /ModDate (D:20240201000000Z) /Trapped /False >>",
    90: b"(nobody refers to me)",
}
streams = {
    15: (b"<< /Length %d >>", b"BT /F1 12 Tf 72 720 Td (Page i) Tj ET\n"),
    16: (b"<< /Length %d /Filter /FlateDecode >>", zlib.compress(b"BT /F2 12 Tf 72 720 Td ( ) Tj ET\n")),
    17: (b"<< /Length %d >>", b"0 0 m 300 300 l S\n"),
    18: (b"<< /Length %d >>", b"q 1 0 0 rg\n"),
    19: (b"<< /Length %d >>", b"0 0 100 100 re f Q\n"),
    23: (b"<< /Length %d /Length1 4 >>", b"\x00\x01\x00\x00"),
    24: (b"<< /Length %d >>", b"/CIDInit /ProcSet findresource begin 1 begincodespacerange <00> <FF> endcodespacerange endcmap\n"),
    50: (b"<< /Type /Metadata /Subtype /XML /Length %d >>", xmp),
    60: (b"<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length %d >>", bytes(range(12))),
    61: (b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace [/ICCBased 62 0 R] /BitsPerComponent 8 /SMask 63 0 R /Interpolate true /Length %d >>", b"\xff\x00\x00"),
    62: (b"<< /N 3 /Length %d >>", b"fake ICC profile"),
    63: (b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length %d >>", b"\x80"),
    64: (b"<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F3 28 0 R >> /XObject << /Im2 61 0 R >> >> /Length %d >>", b"BT /F3 10 Tf (x) Tj ET /Im2 Do\n"),
}
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
out += b"trailer\n<< /Size %d /Root 1 0 R /Info 80 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("model.pdf", "wb").write(out)
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
