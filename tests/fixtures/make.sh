#!/usr/bin/env bash
# Regenerate the test fixtures. Requires python3 (with fontTools) and qpdf.
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
# Content streams (pdfi-content): a page using every kind of resource and
# operator group, a page full of mistakes, and a page whose /Contents array
# splits operands from their operator.
python3 -I - <<'PY'
import zlib
tounicode = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange 2 beginbfchar <0041> <0041> <0042> <0042> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end\n"
page1 = b"""q
1 0 0 1 50 50 cm
0.5 0 0 rg 0 0 200 100 re f
2 w 0 0 1 RG 10 10 m 190 90 l S
Q
q 0 0 612 720 re W n
/GS1 gs
/CS0 cs 0.1 0.2 0.3 sc
BT /F1 24 Tf 72 700 Td (Hello) Tj [(W) -250 (orld)] TJ ET
BT /F2 12 Tf 3 Tr 1 0 0 1 72 650 Tm <00410042> Tj ET
Q
/P <</MCID 0>> BDC q 100 0 0 50 300 400 cm /Im1 Do Q EMC
/OC /MC0 BDC q 1 0 0 1 100 100 cm /Fm1 Do Q EMC
q 20 0 0 20 400 600 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID """ + bytes(range(12)) + b""" EI Q
BX 1 2 foo EX
q 0 0 612 792 re W n /Sh1 sh Q
"""
page2 = b"""Q
(Text outside) Tj
BT /F9 12 Tf (x) Tj
q 1 0 0 1 0 0 cm Q
ET
1 2 m 3 l
0 0 m 10 10 l W 0 g f
/F1 Tf
1 0 0 rg 1 2 3 4 5 6 7 cm
1 0 0 sc /Pattern cs 1 scn
/Im9 Do /GS9 gs
q q /Tag BMC
EMC EMC
foo
S
BT
"""
objs = {
    1: b"<< /Type /Catalog /Pages 2 0 R >>",
    2: b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 612 792] >>",
    3: b"<< /Type /Page /Parent 2 0 R /Contents 10 0 R /Resources << /Font << /F1 20 0 R /F2 21 0 R >> /XObject << /Im1 30 0 R /Fm1 31 0 R >> /ExtGState << /GS1 << /Type /ExtGState /CA 0.5 /ca 0.5 /LW 3 /BM /Multiply >> >> /ColorSpace << /CS0 [/ICCBased 32 0 R] >> /Shading << /Sh1 33 0 R >> /Properties << /MC0 << /Type /OCG /Name (Layer) >> >> >> >>",
    4: b"<< /Type /Page /Parent 2 0 R /Contents 11 0 R /Resources << /Font << /F1 20 0 R >> >> >>",
    5: b"<< /Type /Page /Parent 2 0 R /Contents [12 0 R 13 0 R] /Resources << >> >>",
    20: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    21: b"<< /Type /Font /Subtype /Type0 /BaseFont /CIDFont /Encoding /Identity-H /DescendantFonts [22 0 R] /ToUnicode 24 0 R >>",
    22: b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 23 0 R /W [65 [500 600]] /DW 1000 >>",
    23: b"<< /Type /FontDescriptor /FontName /CIDFont /Flags 4 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>",
    33: b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 612 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>",
}
streams = {
    10: (b"<< /Length %d /Filter /FlateDecode >>", zlib.compress(page1)),
    11: (b"<< /Length %d >>", page2),
    12: (b"<< /Length %d >>", b"0 0 1"),
    13: (b"<< /Length %d >>", b" rg 10 10 50 50 re f\n"),
    24: (b"<< /Length %d >>", tounicode),
    30: (b"<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length %d >>", bytes(range(12))),
    31: (b"<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Matrix [2 0 0 2 0 0] /Resources << /Font << /F1 20 0 R >> >> /Length %d >>", b"BT /F1 10 Tf 5 5 Td (in form) Tj ET 0 0 50 50 re S\n"),
    32: (b"<< /N 3 /Length %d >>", b"fake ICC profile"),
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
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("content.pdf", "wb").write(out)
PY
# Rendering (pdfi-render): paths, clips, strokes and color spaces; images
# of every basic kind (JPEG data generated once with Pillow, kept as hex);
# a rotated page with a clipped form XObject.
python3 -I - <<'PY'
import zlib
JPEG_RGB = bytes.fromhex("""
ffd8ffe000104a46494600010100000100010000ffdb0043000503040404030504040405050506070c08070707070f0b0b09
0c110f1212110f111113161c1713141a1511111821181a1d1d1f1f1f13172224221e241c1e1f1effdb004301050505070607
0e08080e1e1411141e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e
1e1e1e1e1e1e1e1effc00011080008001003012200021101031101ffc4001500010100000000000000000000000000000006
ffc4001910000105000000000000000000000000000005072431a2ffc4001501010100000000000000000000000000000405
ffc400191100020301000000000000000000000000002204050706ffda000c03010002110311003f009b4d6a6a364a54d6a6
a36400da75fce7621677d3d8a39fffd9
""".replace("\n", ""))
JPEG_GRAY = bytes.fromhex("""
ffd8ffe000104a46494600010100000100010000ffdb0043000503040404030504040405050506070c08070707070f0b0b09
0c110f1212110f111113161c1713141a1511111821181a1d1d1f1f1f13172224221e241c1e1f1effc0000b08000800100101
1100ffc40014000100000000000000000000000000000006ffc4001910000105000000000000000000000000000005072431
a2ffda0008010100003f00369ad4d46c89535a9a8d93ffd9
""".replace("\n", ""))
JPEG_CMYK = bytes.fromhex("""
ffd8ffee000e41646f626500640000000000ffdb0043000503040404030504040405050506070c08070707070f0b0b090c11
0f1212110f111113161c1713141a1511111821181a1d1d1f1f1f13172224221e241c1e1f1effc00014080008000804431100
4d11005911004b1100ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b510000201
0303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282
090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778
797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5
d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffda000e0443004d0059004b00003f00fb2ebe32af8cabecba
ffd9
""".replace("\n", ""))
k = 0.5523
def circle(cx, cy, r):
    c = k * r
    return (b"%g %g m %g %g %g %g %g %g c %g %g %g %g %g %g c %g %g %g %g %g %g c %g %g %g %g %g %g c h" % (
        cx + r, cy, cx + r, cy + c, cx + c, cy + r, cx, cy + r,
        cx - c, cy + r, cx - r, cy + c, cx - r, cy, cx - r, cy - c,
        cx - c, cy - r, cx, cy - r, cx + c, cy - r, cx + r, cy - c, cx + r, cy))
page1 = b"""q 0 0 1 rg 20 20 160 160 re 60 60 80 80 re f* Q
q """ + circle(300, 100, 80) + b""" W n 1 0 0 rg 200 0 200 200 re f Q
q 20 200 100 30 re W n q 0 200 50 50 re W n 0 1 0 rg 0 0 400 400 re f Q Q
0 w 0 g 200 220 m 380 220 l S
q 6 w 1 J [12 6] 0 d 0 0.6 0 RG 20 250 m 380 250 l S Q
q 10 w 0 j 0 g 40 300 m 100 380 l 160 300 l S Q
q 0 0 1 0 k 200 300 60 60 re f Q
q /CSI cs 2 sc 270 300 60 60 re f Q
q /CSS cs 1 scn 340 300 50 40 re f Q
q /CSL cs 50 80 0 sc 340 345 50 40 re f Q
q /GS1 gs 0 0 1 rg 300 150 80 80 re f Q
"""
page2 = b"""q 80 0 0 80 20 300 cm /Bits1 Do Q
q 80 0 0 20 120 360 cm /Idx4 Do Q
q 80 0 0 80 220 300 cm /Rgb16 Do Q
q 1 0 0 rg 80 0 0 80 310 300 cm /Stencil Do Q
q 0 0 1 rg 0 200 400 80 re f Q
q 80 0 0 80 20 200 cm /Soft Do Q
q 80 0 0 80 120 200 cm /Keyed Do Q
q 160 0 0 80 220 200 cm /JpegRgb Do Q
q 160 0 0 80 20 100 cm /JpegGray Do Q
q 80 0 0 80 220 100 cm /JpegCmyk Do Q
q 80 0 0 80 310 100 cm BI /W 4 /H 4 /CS /G /BPC 8 /D [1 0] ID """ + bytes([0, 64, 128, 255] * 4) + b""" EI Q
"""
page3 = b"""q 1 0 0 1 50 50 cm /Fm1 Do Q
0 0 1 RG 4 w 10 10 280 180 re S
"""
form = b"0.8 0.2 0.2 rg -100 -100 400 400 re f 0 g 0 0 m 100 100 l S\n"
objs = {
    1: b"<< /Type /Catalog /Pages 2 0 R >>",
    2: b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 400 400] >>",
    3: b"<< /Type /Page /Parent 2 0 R /Contents 10 0 R /Resources << /ColorSpace << /CSI [/Indexed /DeviceRGB 2 <FF000000FF000000FF>] /CSS [/Separation /Spot /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 0 0 0] /N 1 >>] /CSL [/Lab << /WhitePoint [0.9505 1 1.089] /Range [-128 127 -128 127] >>] >> /ExtGState << /GS1 << /ca 0.5 >> >> >> >>",
    4: b"<< /Type /Page /Parent 2 0 R /Contents 11 0 R /Resources << /XObject << /Bits1 20 0 R /Idx4 21 0 R /Rgb16 22 0 R /Stencil 23 0 R /Soft 24 0 R /Keyed 26 0 R /JpegRgb 27 0 R /JpegGray 28 0 R /JpegCmyk 29 0 R >> >> >>",
    5: b"<< /Type /Page /Parent 2 0 R /Contents 12 0 R /MediaBox [0 0 300 200] /Rotate 90 /Resources << /XObject << /Fm1 30 0 R >> >> >>",
}
img = lambda d: b"<< /Type /XObject /Subtype /Image " + d + b" /Length %d >>"
streams = {
    10: (b"<< /Length %d /Filter /FlateDecode >>", zlib.compress(page1)),
    11: (b"<< /Length %d >>", page2),
    12: (b"<< /Length %d >>", page3),
    20: (img(b"/Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 1"), bytes([0xAA, 0x55] * 4)),
    21: (img(b"/Width 4 /Height 1 /ColorSpace [/Indexed /DeviceRGB 3 <FF000000FF000000FFFFFF00>] /BitsPerComponent 4"), bytes([0x01, 0x23])),
    22: (img(b"/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 16"), bytes([255, 255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 255, 255, 128, 0, 128, 0, 128, 0])),
    23: (img(b"/Width 8 /Height 8 /ImageMask true"), bytes([0x81, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x81])),
    24: (img(b"/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 25 0 R"), bytes([255, 0, 0, 0, 255, 0, 255, 255, 0, 0, 0, 0])),
    25: (img(b"/Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8"), bytes([255, 128, 64, 0])),
    26: (img(b"/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask [0 0 0 0 0 0]"), bytes([255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0])),
    27: (img(b"/Width 16 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"), JPEG_RGB),
    28: (img(b"/Width 16 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode"), JPEG_GRAY),
    29: (img(b"/Width 8 /Height 8 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Decode [1 0 1 0 1 0 1 0] /Filter /DCTDecode"), JPEG_CMYK),
    30: (b"<< /Type /XObject /Subtype /Form /BBox [0 0 100 50] /Matrix [2 0 0 2 0 0] /Length %d >>", form),
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
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("render.pdf", "wb").write(out)
PY
python3 -I - <<'PY'
# Fonts (pdfi-font): one page with each kind of font program, glyph
# selection rule, Type 3 procedures and text rendering modes. Every glyph
# drawn is the "square" (100..600 × 0..700 in a 1000-unit em) so pixels can
# be checked; built with fontTools.
import io
import zlib
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables._c_m_a_p import cmap_format_4

def square(pen):
    pen.moveTo((100, 0)); pen.lineTo((100, 700)); pen.lineTo((600, 700)); pen.lineTo((600, 0)); pen.closePath()

def triangle(pen):
    pen.moveTo((100, 0)); pen.lineTo((350, 700)); pen.lineTo((600, 0)); pen.closePath()

def empty(pen):
    pass

ORDER = [".notdef", "A", "B"]
SHAPES = {".notdef": empty, "A": square, "B": triangle}

def sfnt(cff, symbolic=False):
    fb = FontBuilder(1000, isTTF=not cff)
    fb.setupGlyphOrder(ORDER)
    fb.setupCharacterMap({0x41: "A", 0x42: "B"})
    if cff:
        cs = {}
        for n in ORDER:
            pen = T2CharStringPen(700, None)
            SHAPES[n](pen)
            cs[n] = pen.getCharString()
        fb.setupCFF("PdfiTestCFF", {"FullName": "PdfiTestCFF"}, cs, {})
    else:
        glyphs = {}
        for n in ORDER:
            pen = TTGlyphPen(None)
            SHAPES[n](pen)
            glyphs[n] = pen.glyph()
        fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (700, 100) for n in ORDER})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "PdfiTest", "styleName": "Regular"})
    fb.setupOS2()
    fb.setupPost()
    # Fixed timestamps: the fixture is the same on every run.
    fb.updateHead(created=0, modified=0)
    fb.font.recalcTimestamp = False
    if symbolic:
        sub = cmap_format_4(4)
        sub.platformID, sub.platEncID, sub.language = 3, 0, 0
        sub.cmap = {0xF041: "A", 0xF042: "B"}
        fb.font["cmap"].tables = [sub]
    b = io.BytesIO()
    fb.save(b)
    return b.getvalue(), fb.font

tt, _ = sfnt(False)
tt_sym, _ = sfnt(False, symbolic=True)
otf, otf_font = sfnt(True)
bare_cff = otf_font.getTableData("CFF ")

# A Type 1 font written by hand: one square glyph, eexec encrypted.
def t1num(v):
    if -107 <= v <= 107:
        return bytes([v + 139])
    if 108 <= v <= 1131:
        v -= 108
        return bytes([(v >> 8) + 247, v & 0xFF])
    if -1131 <= v <= -108:
        v = -v - 108
        return bytes([(v >> 8) + 251, v & 0xFF])
    return b"\xff" + v.to_bytes(4, "big", signed=True)

def t1cs(*items):
    out = b""
    for it in items:
        out += t1num(it[0]) if isinstance(it, tuple) else bytes([it])
    return out

def encrypt(data, r):
    out = bytearray()
    for b in data:
        c = b ^ (r >> 8)
        r = ((c + r) * 52845 + 22719) & 0xFFFF
        out.append(c)
    return bytes(out)

HSBW, RMOVETO, RLINETO, CLOSEPATH, ENDCHAR = 13, 21, 5, 9, 14
charstrings = {
    ".notdef": t1cs((0,), (250,), HSBW, ENDCHAR),
    "A": t1cs((0,), (700,), HSBW, (100,), (0,), RMOVETO, (0,), (700,), RLINETO,
              (500,), (0,), RLINETO, (0,), (-700,), RLINETO, CLOSEPATH, ENDCHAR),
}
private = b"dup /Private 8 dict dup begin\n/RD{string currentfile exch readstring pop}executeonly def\n/ND{noaccess def}executeonly def\n/NP{noaccess put}executeonly def\n/lenIV 4 def\n/Subrs 0 array\nND\n2 index /CharStrings %d dict dup begin\n" % len(charstrings)
for n, cs in charstrings.items():
    enc = encrypt(b"\0\0\0\0" + cs, 4330)
    private += b"/%s %d RD " % (n.encode(), len(enc)) + enc + b" ND\n"
private += b"end\nend\nreadonly put\nnoaccess put\ndup/FontName get exch definefont pop\nmark currentfile closefile\n"
t1_clear = b"%!PS-AdobeFont-1.0: PdfiType1 001\n12 dict begin\n/FontName /PdfiType1 def\n/FontType 1 def\n/PaintType 0 def\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/FontBBox {0 0 700 700} readonly def\n/Encoding StandardEncoding def\ncurrentdict end\ncurrentfile eexec\n"
t1_binary = encrypt(b"\0\0\0\0" + private, 55665)
t1_trailer = (b"0" * 64 + b"\n") * 8 + b"cleartomark\n"

desc = lambda flags, ff: b"<< /Type /FontDescriptor /FontName /PdfiTest /Flags %d /FontBBox [0 0 700 700] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 %s >>" % (flags, ff)
simple = lambda sub, base, d, extra=b"": b"<< /Type /Font /Subtype /%s /BaseFont /%s /FirstChar 65 /LastChar 66 /Widths [700 700] /FontDescriptor %d 0 R %s >>" % (sub, base, d, extra)

content = b"""BT /F1 100 Tf 50 650 Td (A) Tj ET
BT /F2 100 Tf 170 650 Td (A) Tj ET
BT /F3 100 Tf 290 650 Td (A) Tj ET
BT /F4 100 Tf 410 650 Td (A) Tj ET
BT /F5 100 Tf 50 500 Td <0005> Tj ET
BT /F6 100 Tf 170 500 Td (A) Tj ET
q 0 0 1 rg BT /F7 100 Tf 290 500 Td (ab) Tj ET Q
BT /F8 100 Tf 470 500 Td (H) Tj ET
q 0 0.6 0 RG 5 w BT 1 Tr /F1 100 Tf 50 350 Td (A) Tj ET Q
q BT 7 Tr /F1 100 Tf 170 350 Td (A) Tj ET 0 0 1 rg 170 350 200 200 re f Q
BT /F9 100 Tf 290 350 Td <0001> Tj ET
BT /F3 100 Tf 410 350 Td (B) Tj ET
BT /F10 100 Tf 470 350 Td <034B> Tj ET
BT /F11 100 Tf 50 200 Td <0001> Tj ET
"""
font_res = b" ".join(b"/F%d %d 0 R" % (i, 20 + i) for i in range(1, 12))
objs = {
    1: b"<< /Type /Catalog /Pages 2 0 R >>",
    2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>",
    3: b"<< /Type /Page /Parent 2 0 R /Contents 10 0 R /Resources << /Font << " + font_res + b" >> >> >>",
    21: simple(b"TrueType", b"PdfiTT", 41, b"/Encoding /WinAnsiEncoding"),
    22: simple(b"TrueType", b"PdfiTTSymbol", 42),
    23: simple(b"Type1", b"PdfiCFF", 43, b"/Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [66 /A] >>"),
    24: simple(b"Type1", b"PdfiOTF", 44, b"/Encoding /WinAnsiEncoding"),
    25: b"<< /Type /Font /Subtype /Type0 /BaseFont /PdfiTT /Encoding /Identity-H /DescendantFonts [35 0 R] /ToUnicode 36 0 R >>",
    35: b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /PdfiTT /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 45 0 R /DW 1000 /W [5 [700]] /CIDToGIDMap 37 0 R >>",
    26: simple(b"Type1", b"PdfiType1", 46),
    27: b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 700 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /sq 50 0 R /tri 51 0 R >> /Encoding << /Type /Encoding /Differences [97 /sq /tri] >> /FirstChar 97 /LastChar 98 /Widths [700 700] /Resources << >> >>",
    28: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    29: b"<< /Type /Font /Subtype /Type0 /BaseFont /PdfiMissingCID /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /PdfiMissingCID /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 49 0 R /DW 700 >>] >>",
    # Not embedded, drawn with Harano Aji: Adobe-Japan1 CID 843 (a
    # hiragana a, Mincho), and an Identity font whose /ToUnicode gives
    # U+6F22 (a kanji, Gothic by the name).
    30: b"<< /Type /Font /Subtype /Type0 /BaseFont /Ryumin-Light-Identity-H /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Ryumin-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 4 >> /FontDescriptor 47 0 R >>] >>",
    31: b"<< /Type /Font /Subtype /Type0 /BaseFont /PdfiGothic /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /PdfiGothic /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 48 0 R >>] /ToUnicode 38 0 R >>",
    47: desc(4, b""),
    48: desc(4, b""),
    41: desc(32, b"/FontFile2 61 0 R"),
    42: desc(4, b"/FontFile2 62 0 R"),
    43: desc(32, b"/FontFile3 63 0 R"),
    44: desc(32, b"/FontFile3 64 0 R"),
    45: desc(4, b"/FontFile2 61 0 R"),
    46: desc(32, b"/FontFile 66 0 R"),
    49: desc(4, b""),
}
z = zlib.compress
tounicode = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Pdfi-UCS def 1 begincodespacerange <0000> <FFFF> endcodespacerange 1 beginbfchar <0005> <0058> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"
streams = {
    10: (b"<< /Length %d /Filter /FlateDecode >>", z(content)),
    36: (b"<< /Length %d >>", tounicode),
    38: (b"<< /Length %d >>", tounicode.replace(b"<0005> <0058>", b"<0001> <6F22>")),
    37: (b"<< /Length %d >>", bytes([0, 0] * 5 + [0, 1])),
    50: (b"<< /Length %d >>", b"700 0 d0 1 0 0 rg 100 0 500 700 re f"),
    51: (b"<< /Length %d >>", b"700 0 0 0 700 700 d1 0 1 0 rg 100 0 500 700 re f"),
    61: (b"<< /Length %%d /Length1 %d /Filter /FlateDecode >>" % len(tt), z(tt)),
    62: (b"<< /Length %%d /Length1 %d /Filter /FlateDecode >>" % len(tt_sym), z(tt_sym)),
    63: (b"<< /Length %d /Subtype /Type1C /Filter /FlateDecode >>", z(bare_cff)),
    64: (b"<< /Length %d /Subtype /OpenType /Filter /FlateDecode >>", z(otf)),
    66: (b"<< /Length %%d /Length1 %d /Length2 %d /Length3 %d >>" % (len(t1_clear), len(t1_binary), len(t1_trailer)), t1_clear + t1_binary + t1_trailer),
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
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("fonts.pdf", "wb").write(out)
PY
python3 -I - <<'PY'
# Painting (pdfi-render, M12): one page per feature.
import struct, zlib

def s15(v):
    return struct.pack(">i", round(v * 65536))

def icc(space, tags):
    """A minimal ICC v2 display profile (PCS XYZ) with `tags` {sig: data}."""
    tags = dict(tags)
    tags[b"desc"] = b"desc\0\0\0\0" + struct.pack(">I", 5) + b"test\0" + b"\0" * 8 + b"\0" * 3 + b"\0" * 67
    tags[b"wtpt"] = b"XYZ \0\0\0\0" + s15(0.9642) + s15(1.0) + s15(0.8249)
    body, table, off = b"", b"", 128 + 4 + 12 * len(tags)
    for sig, data in tags.items():
        data += b"\0" * (-len(data) % 4)
        table += sig + struct.pack(">II", off + len(body), len(data))
        body += data
    size = off + len(body)
    head = struct.pack(">I", size) + b"\0" * 4 + bytes([2, 0x10, 0, 0]) + b"mntr" + space + b"XYZ "
    head += b"\0" * 12 + b"acsp" + b"\0" * 24 + s15(0.9642) + s15(1.0) + s15(0.8249)
    head += b"\0" * (128 - len(head))
    return head + struct.pack(">I", len(tags)) + table + body

def curv(gamma):
    return b"curv\0\0\0\0" + struct.pack(">IH", 1, round(gamma * 256))

def xyz(x, y, z):
    return b"XYZ \0\0\0\0" + s15(x) + s15(y) + s15(z)

# Gray, linear: 0.5 is sRGB 188, not 128.
ICC_GRAY = icc(b"GRAY", {b"kTRC": curv(1.0)})
# RGB with the red and blue colorants swapped: red shows blue.
ICC_SWAP = icc(b"RGB ", {
    b"rXYZ": xyz(0.1431, 0.0606, 0.7141), b"gXYZ": xyz(0.3851, 0.7169, 0.0971),
    b"bXYZ": xyz(0.4361, 0.2225, 0.0139),
    b"rTRC": curv(2.2), b"gTRC": curv(2.2), b"bTRC": curv(2.2),
})

pages = []
objs = {1: b"<< /Type /Catalog /Pages 2 0 R >>"}
streams = {}

def page(content, resources):
    n = 100 + 10 * len(pages)
    pages.append(n)
    objs[n] = b"<< /Type /Page /Parent 2 0 R /Contents %d 0 R /Resources %s >>" % (n + 1, resources)
    streams[n + 1] = (b"<< /Length %d >>", content)

# Page 1: colors. ICC gray and RGB, CalGray, Separation (calculator tint
# transform), DeviceN (sampled tint transform), an image in the ICC space.
streams[20] = (b"<< /N 1 /Length %d >>", ICC_GRAY)
streams[21] = (b"<< /N 3 /Alternate /DeviceRGB /Length %d >>", ICC_SWAP)
streams[22] = (b"<< /FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1 0 1] /Length %d >>", b"{ 0 exch dup 0 }")
streams[23] = (b"<< /FunctionType 0 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Size [2 2] /BitsPerSample 8 /Length %d >>",
               bytes([0, 0, 0, 255, 0, 0, 0, 255, 0, 255, 255, 0]))
streams[24] = (b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace [/ICCBased 21 0 R] /BitsPerComponent 8 /Length %d >>",
               bytes([255, 0, 0, 0, 255, 0]))
page(b"""/ICCG cs 0.5 sc 0 300 100 100 re f
/ICCR cs 1 0 0 sc 100 300 100 100 re f
/CalG cs 0.5 sc 200 300 100 100 re f
/Sep cs 1 scn 300 300 100 100 re f
/DevN cs 1 1 scn 0 200 100 100 re f
/DevN cs 1 0 scn 100 200 100 100 re f
q 200 0 0 100 200 200 cm /Img Do Q
""", b"<< /ColorSpace << /ICCG [/ICCBased 20 0 R] /ICCR [/ICCBased 21 0 R] /CalG [/CalGray << /WhitePoint [0.9505 1 1.089] /Gamma 2.2 >>] /Sep [/Separation /Red /DeviceCMYK 22 0 R] /DevN [/DeviceN [/A /B] /DeviceRGB 23 0 R] >> /XObject << /Img 24 0 R >> >>")

# Page 2: shadings. Axial, radial, function-based (sh in a clip); a
# shading pattern filling a rectangle, stroking and through a stencil
# mask; triangle meshes (free-form, lattice), Coons and tensor patches.
def mesh(kind, extra, data):
    return (b"<< /ShadingType %d /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 %s /Length %%d >>" % (kind, extra), data)
def v16(x, y):
    return struct.pack(">HH", x, y)
streams[30] = (b"<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length %d >>", b"{ 0 }")
# Free-form triangles: flag 0, then flag 1 continuing with one vertex.
streams[31] = mesh(4, b"/BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1]",
    b"\0" + v16(0, 0) + b"\xff\0\0" + b"\0" + v16(65535, 0) + b"\0\xff\0" + b"\0" + v16(0, 65535) + b"\0\0\xff"
    + b"\1" + v16(65535, 65535) + b"\xff\xff\xff")
streams[32] = mesh(5, b"/VerticesPerRow 2 /Decode [0 100 0 100 0 1 0 1 0 1]",
    v16(0, 0) + b"\xff\0\0" + v16(65535, 0) + b"\0\xff\0" + v16(0, 65535) + b"\0\0\xff" + v16(65535, 65535) + b"\xff\xff\0")
# Coons: the boundary of a square with a bulging top edge.
coons = [(0, 0), (0, 33), (0, 66), (0, 100), (33, 130), (66, 130), (100, 100), (100, 66), (100, 33), (100, 0), (66, 0), (33, 0)]
streams[33] = mesh(6, b"/BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1]",
    b"\0" + b"".join(v16(x * 500, y * 500) for x, y in coons) + b"\xff\0\0" + b"\0\xff\0" + b"\0\0\xff" + b"\xff\xff\0")
tensor = coons + [(33, 33), (33, 66), (66, 66), (66, 33)]
streams[34] = mesh(7, b"/BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1]",
    b"\0" + b"".join(v16(x * 500, y * 500) for x, y in tensor) + b"\0\0\0" + b"\xff\xff\xff" + b"\0\0\0" + b"\xff\xff\xff")
streams[35] = (b"<< /Type /XObject /Subtype /Image /Width 8 /Height 8 /ImageMask true /Length %d >>", bytes([0xF0] * 4 + [0x0F] * 4))
objs[36] = b"<< /PatternType 2 /Matrix [1 0 0 1 100 200] /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 1 0] /C1 [1 1 0] /N 1 >> /Extend [true true] >> >>"
page(b"""q 0 300 100 100 re W n /Ax sh Q
q 100 300 100 100 re W n /Rad sh Q
q 200 300 100 100 re W n /Fn sh Q
/Pattern cs /P0 scn 100 200 100 100 re f
/Pattern CS /P0 SCN 20 w 300 350 m 400 350 l S
q 1 0 0 1 200 200 cm /Tri sh Q
q 1 0 0 1 300 200 cm /Lat sh Q
q 1 0 0 1 0 50 cm /Coons sh Q
q 1 0 0 1 100 50 cm /Tensor sh Q
q /Pattern cs /P0 scn 100 0 0 100 0 200 cm /Mask Do Q
""", b"<< /Shading << /Ax << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> /Extend [true true] >> /Rad << /ShadingType 3 /ColorSpace /DeviceGray /Coords [150 350 0 150 350 50] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> /Fn << /ShadingType 1 /ColorSpace /DeviceRGB /Matrix [100 0 0 100 200 300] /Function 30 0 R >> /Tri 31 0 R /Lat 32 0 R /Coons 33 0 R /Tensor 34 0 R >> /Pattern << /P0 36 0 R >> /XObject << /Mask 35 0 R >> >>")

k = 0.5523
def circle(cx, cy, r):
    c = k * r
    return (b"%g %g m %g %g %g %g %g %g c %g %g %g %g %g %g c %g %g %g %g %g %g c %g %g %g %g %g %g c h" % (
        cx + r, cy, cx + r, cy + c, cx + c, cy + r, cx, cy + r,
        cx - c, cy + r, cx - r, cy + c, cx - r, cy, cx - r, cy - c,
        cx - c, cy - r, cx, cy - r, cx + c, cy - r, cx + r, cy - c, cx + r, cy))

# Page 3: tiling patterns. Colored, uncolored (in a /Pattern space with
# a base), rotated, a box larger than the step (wrapping), stroked, and
# inside a form scaled by its /Matrix (pattern space is the form's).
def tiling(extra, content):
    return (b"<< /Type /Pattern /PatternType 1 /TilingType 1 %s /Length %%d >>" % extra, content)
streams[40] = tiling(b"/PaintType 1 /BBox [0 0 20 20] /XStep 20 /YStep 20 /Resources << >>",
                     b"1 0 0 rg 0 0 10 10 re f 0 0 1 rg 10 10 10 10 re f")
streams[41] = tiling(b"/PaintType 2 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
                     b"0 0 5 10 re f")
streams[42] = tiling(b"/PaintType 1 /BBox [0 0 20 20] /XStep 20 /YStep 20 /Matrix [0.7071 0.7071 -0.7071 0.7071 0 0] /Resources << >>",
                     b"0 0.6 0 rg 0 0 10 20 re f")
streams[43] = tiling(b"/PaintType 1 /BBox [0 0 30 30] /XStep 20 /YStep 20 /Resources << >>",
                     b"1 0.5 0 rg " + circle(15, 15, 15) + b" f")
streams[44] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 100 50] /Matrix [2 0 0 2 0 0] /Resources << /Pattern << /P1 40 0 R >> >> /Length %d >>",
               b"/Pattern cs /P1 scn 0 0 100 50 re f")
page(b"""/Pattern cs /P1 scn 0 300 200 100 re f
/PU cs 0 0 1 /P2 scn 200 300 200 100 re f
/Pattern cs /P3 scn 0 200 200 100 re f
/Pattern cs /P4 scn 200 200 200 100 re f
/Pattern CS /P1 SCN 20 w 0 150 m 400 150 l S
q 1 0 0 1 0 0 cm /Fm Do Q
""", b"<< /ColorSpace << /PU [/Pattern /DeviceRGB] >> /Pattern << /P1 40 0 R /P2 41 0 R /P3 42 0 R /P4 43 0 R >> /XObject << /Fm 44 0 R >> >>")

# Page 4: transparency. Multiply and Screen; a transparency group and a
# plain form painted with ca 0.5 (the group's overlap is not darker);
# soft masks: luminosity (a gradient), alpha (a circle), with a white
# backdrop (/BC), with an inverting transfer function (/TR).
def form(bbox, content, group=b"/Group << /S /Transparency /CS /DeviceGray >>", res=b"<< >>"):
    return (b"<< /Type /XObject /Subtype /Form /BBox [%s] %s /Resources %s /Length %%d >>" % (bbox, group, res), content)
streams[50] = form(b"0 200 100 300", b"/S sh", res=b"<< /Shading << /S << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> >> >>")
streams[51] = form(b"100 200 200 300", circle(150, 250, 40) + b" f")
streams[52] = form(b"200 200 300 300", b"0 g 220 220 60 60 re f")
streams[53] = form(b"300 200 400 300", b"1 g 320 220 60 60 re f")
streams[55] = form(b"200 300 300 400", b"0 g 200 300 70 70 re f 230 330 70 70 re f", group=b"/Group << /S /Transparency >>")
streams[56] = form(b"300 300 400 400", b"0 g 300 300 70 70 re f 330 330 70 70 re f", group=b"")
# Non-isolated: white multiplied onto the red under the group leaves red,
# even at ca 0.5 (an isolated group would show pink).
streams[57] = form(b"0 100 100 200", b"/M gs 1 g 0 100 100 100 re f", group=b"/Group << /S /Transparency >>", res=b"<< /ExtGState << /M << /BM /Multiply >> >> >>")
# Knockout groups: red then blue at ca 0.5, overlapping. The blue
# replaces the red where they overlap (isolated: over nothing; not
# isolated: over the green under the group).
KO = b"/H gs 1 0 0 rg %d 100 70 70 re f 0 0 1 rg %d 130 70 70 re f"
KO_RES = b"<< /ExtGState << /H << /ca 0.5 >> >> >>"
streams[58] = form(b"100 100 200 200", KO % (100, 130), group=b"/Group << /S /Transparency /I true /K true >>", res=KO_RES)
streams[59] = form(b"200 100 300 200", KO % (200, 230), group=b"/Group << /S /Transparency /K true >>", res=KO_RES)
page(b"""1 1 0 rg 0 300 70 70 re f q /Mul gs 0 1 1 rg 30 330 70 70 re f Q
1 0 0 rg 100 300 70 70 re f q /Scr gs 0 0 1 rg 130 330 70 70 re f Q
q /Half gs /Grp Do Q
q /Half gs /Plain Do Q
q /SmL gs 1 0 0 rg 0 200 100 100 re f Q
q /SmA gs 0 0 1 rg 100 200 100 100 re f Q
q /SmB gs 0 0.5 0 rg 200 200 100 100 re f Q
q /SmT gs 0.5 0 0.5 rg 300 200 100 100 re f Q
1 0 0 rg 0 100 100 100 re f q /Half gs /NonIso Do Q
/KoI Do 0 1 0 rg 200 100 100 100 re f /KoN Do
""", b"<< /ExtGState << /Mul << /BM /Multiply >> /Scr << /BM /Screen >> /Half << /ca 0.5 >> /SmL << /SMask << /S /Luminosity /G 50 0 R >> >> /SmA << /SMask << /S /Alpha /G 51 0 R >> >> /SmB << /SMask << /S /Luminosity /G 52 0 R /BC [1] >> >> /SmT << /SMask << /S /Luminosity /G 53 0 R /TR << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> >> >> /XObject << /Grp 55 0 R /Plain 56 0 R /NonIso 57 0 R /KoI 58 0 R /KoN 59 0 R >> >>")

# Page 5: image codecs. JPEG 2000 (JP2 RGB; a raw codestream; RGBA with
# /SMaskInData), JBIG2 (generic region; symbols with /JBIG2Globals as a
# stencil mask), CCITT (Group 4 stencil mask, Group 3 image). Made with
# opj_compress, jbig2enc and Pillow (libtiff) from 16 × 16 and 64 × 64
# drawings: red | blue halves, green | transparent halves, a black disc.
JP2_RGB = bytes.fromhex("""
0000000c6a5020200d0a870a00000014667479706a703220000000006a7032200000002d6a70326800000016696864720000
0010000000100003070700000000000f636f6c7201000000000010000000f26a703263ff4fff51002f000000000010000000
100000000000000000000000100000001000000000000000000003070101070101070101ff52000c00000001010104040001
ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e34ff90
000a0000000000770001ff93cfb438115054a36e52840000000184847fdf80f027350ef366be7f66e49fef5df7f0a272386c
50a0eeb344a12eb3285add65ff19182219dedcb20a3cfcc240207408023c92bd2d3a19bf5c3617aa2eb31b5cdd663f05ba7f
80c7da0800216e07facfc014221a0f283fffd9
""".replace("\n", ""))
J2K_RGB = bytes.fromhex("""
ff4fff51002f0000000000100000001000000000000000000000001000000010000000000000000000030701010701010701
01ff52000c00000001010104040001ff5c00074040484850ff640025000143726561746564206279204f70656e4a50454720
76657273696f6e20322e352e34ff90000a0000000000770001ff93cfb438115054a36e52840000000184847fdf80f027350e
f366be7f66e49fef5df7f0a272386c50a0eeb344a12eb3285add65ff19182219dedcb20a3cfcc240207408023c92bd2d3a19
bf5c3617aa2eb31b5cdd663f05ba7f80c7da0800216e07facfc014221a0f283fffd9
""".replace("\n", ""))
JP2_RGBA = bytes.fromhex("""
0000000c6a5020200d0a870a00000014667479706a703220000000006a7032200000004f6a70326800000016696864720000
0010000000100004070700000000000f636f6c72010000000000100000002263646566000400000000000100010000000200
0200000003000300010000000000fa6a703263ff4fff51003200000000001000000010000000000000000000000010000000
1000000000000000000004070101070101070101070101ff52000c00000001010104040001ff5c00074040484850ff640025
000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e34ff90000a00000000007c0001ff93c7
d40c115054afff7fdf8060115054a27db690000309097fdf8060115054a27db690000309097fdf81c8221780852590148538
7500c2226dc60060414f69479b0fcd87e6c3f361f9b0fcd87e6c3f361f9b0fcd87e6c3f361f9b0fcd87e6c3f361f9b0f8080
80cfc014221a0f283fffd9
""".replace("\n", ""))
JB2_GENERIC = bytes.fromhex("""
0000000030000100000013000000400000004000000000000000000100000000000126000100000053000000400000004000
00000000000000000003fffdff02fefefeaa2e4f718a5d5f2cd5f6bfa10b6bcdeee34f292113f9f2a5104d46fd83ef211972
7ae661ac13114ee6a1efbc7a09e0a4e42a7bf36b49bfffac
""".replace("\n", ""))
JB2_PAGE = bytes.fromhex("""
00000001300001000000130000004000000040000000000000000000000000000002062200010000001d0000004000000040
0000000000000000000000000000019dfb0e7fffac
""".replace("\n", ""))
JB2_GLOBALS = bytes.fromhex("""
0000000000010000000048000003fffdff02fefefe00000001000000012175c7c625ed73f755aa4a75d4a9139ff75983f3e7
668ae25af84afd0a9dacc31abfba3c574d3c5e83c6f6486efd6f1108278beeffac
""".replace("\n", ""))
G4 = bytes.fromhex("""
26a0786ffffc8659f040f040f4f083d3d3d3d3d3d3fd3d3ffd3ffd3fffffffffdaffdaffdadafdadadadadadac30b6b060b0
60b1fffe002002
""".replace("\n", ""))
G3 = bytes.fromhex("""
0013503c370013503c370013503c370013503c370013503c370013503c370013503c370013503c37001350cb38658009a817
a80b8009a86c061b0004d434280d00026a0418020004d41803060004d4171b05c004d4302618001350715070013504170400
1350e521c004d439487001350a5614004d42168400135085a10004d42168400135101440013510144001351014400135142c
500135142c500135142c500135142c500135142c500135142c500135142c500135142c500135142c500135142c5001351014
40013510144001351014400135085a10004d42168400135085a10004d429585001350e521c004d4394870013504170400135
071507001350c09860004d4171b05c004d41803060004d40830040009a868501a0004d436030d80026a05ea02e0026a19670
cb0013503c370013503c370013503c370013503c370013503c370013503c370013503c370013503c37
""".replace("\n", ""))
# An Adobe YCCK JPEG (ImageMagick, -define jpeg:colorspace=11), its CMYK
# inverted as Photoshop writes it: /Decode compensates. Red | blue.
JPEG_YCCK = bytes.fromhex("""
ffd8ffee000e41646f626500640000000002ffdb004300020101010101020101010202020202040302020202050404030406
0506060605060606070908060709070606080b08090a0a0a0a0a06080b0c0b0a0c090a0a0affdb0043010202020202020503
03050a0706070a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a
0a0a0a0a0a0affc00014080008001004011100021101031101041100ffc40016000101010000000000000000000000000008
0009ffc40014100100000000000000000000000000000000ffc4001501010100000000000000000000000000000a08ffc400
14110100000000000000000000000000000000ffda000e040100021103110400003f00662ac1ff006fe1782a64a89fffd9
""".replace("\n", ""))
# A one-channel JPEG (Pillow) in a /Separation /Black space: no ink |
# full ink, so white | black (not inverted as DeviceGray would be).
JPEG_SEP = bytes.fromhex("""
ffd8ffe000104a46494600010100000100010000ffdb00430001010101010101010101010101010101010101010101010101
010101010101010101010101010101010101010101010101010101010101010101010101010101ffc0000b08000800100101
1100ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b51000020103030204030505
04040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a
25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a8384858687
88898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2
e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffda0008010100003f00ff003ffaff007f8affd9
""".replace("\n", ""))
def image(extra, data):
    return (b"<< /Type /XObject /Subtype /Image %s /Length %%d >>" % extra, data)
streams[60] = image(b"/Width 16 /Height 16 /Filter /JPXDecode", JP2_RGB)
streams[61] = image(b"/Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode", J2K_RGB)
streams[62] = image(b"/Width 16 /Height 16 /SMaskInData 1 /Filter /JPXDecode", JP2_RGBA)
streams[63] = image(b"/Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode", JB2_GENERIC)
streams[64] = (b"<< /Length %d >>", JB2_GLOBALS)
streams[65] = image(b"/Width 64 /Height 64 /ImageMask true /Filter /JBIG2Decode /DecodeParms << /JBIG2Globals 64 0 R >>", JB2_PAGE)
streams[66] = image(b"/Width 64 /Height 64 /ImageMask true /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 64 /Rows 64 >>", G4)
streams[68] = image(b"/Width 16 /Height 8 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Decode [1 0 1 0 1 0 1 0] /Filter /DCTDecode", JPEG_YCCK)
streams[69] = image(b"/Width 16 /Height 8 /ColorSpace [/Separation /Black /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 0 1] /N 1 >>] /BitsPerComponent 8 /Filter /DCTDecode", JPEG_SEP)
streams[67] = image(b"/Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K 0 /Columns 64 >>", G3)
page(b"""q 100 0 0 100 0 300 cm /Jp2 Do Q
q 100 0 0 100 100 300 cm /J2k Do Q
q 100 0 0 100 200 300 cm /Jp2a Do Q
q 100 0 0 100 300 300 cm /Jb2 Do Q
q 1 0 0 rg 100 0 0 100 0 200 cm /Jb2g Do Q
q 0 0 1 rg 100 0 0 100 100 200 cm /G4 Do Q
q 100 0 0 100 200 200 cm /G3 Do Q
q 100 0 0 50 300 200 cm /Ycck Do Q
q 100 0 0 50 0 100 cm /Sep Do Q
""", b"<< /XObject << /Sep 69 0 R /Ycck 68 0 R /Jp2 60 0 R /J2k 61 0 R /Jp2a 62 0 R /Jb2 63 0 R /Jb2g 65 0 R /G4 66 0 R /G3 67 0 R >> >>")

objs[2] = b"<< /Type /Pages /Kids [%s] /Count %d /MediaBox [0 0 400 400] >>" % (b" ".join(b"%d 0 R" % n for n in pages), len(pages))
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
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref)
open("paint.pdf", "wb").write(out)
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
