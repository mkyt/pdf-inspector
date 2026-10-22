#!/usr/bin/env python3
"""Write a large synthetic PDF for manual and benchmark testing.

Usage: scripts/make-large-pdf.py OUT.pdf [--pages N] [--image-kib K]

Every page has a Flate-compressed content stream and an uncompressed image
XObject of K KiB, so the file size is roughly N * K KiB.
"""
import argparse
import os
import zlib

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("--pages", type=int, default=3000)
ap.add_argument("--image-kib", type=int, default=100)
a = ap.parse_args()

side = int((a.image_kib * 1024 / 3) ** 0.5)
pixels = os.urandom(side * side * 3)
offsets = {}

with open(a.out, "wb") as f:
    def obj(n, body, stream=None):
        offsets[n] = f.tell()
        f.write(b"%d 0 obj\n" % n + body)
        if stream is not None:
            f.write(b"\nstream\n" + stream + b"\nendstream")
        f.write(b"\nendobj\n")

    f.write(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    first = 4  # objects per page: page, contents, image
    kids = b" ".join(b"%d 0 R" % (first + 3 * i) for i in range(a.pages))
    obj(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    obj(2, b"<< /Type /Pages /Count %d /Kids [%s] /MediaBox [0 0 612 792] >>" % (a.pages, kids))
    obj(3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    for i in range(a.pages):
        p = first + 3 * i
        obj(p, b"<< /Type /Page /Parent 2 0 R /Contents %d 0 R /Resources << /Font << /F1 3 0 R >> /XObject << /Im0 %d 0 R >> >> >>" % (p + 1, p + 2))
        text = b"q 400 0 0 400 106 300 cm /Im0 Do Q BT /F1 24 Tf 72 720 Td (Page %d) Tj ET\n" % (i + 1)
        data = zlib.compress(text * 10)
        obj(p + 1, b"<< /Length %d /Filter /FlateDecode >>" % len(data), data)
        obj(p + 2, b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length %d >>" % (side, side, len(pixels)), pixels)
    size = max(offsets) + 1
    xref = f.tell()
    f.write(b"xref\n0 %d\n0000000000 65535 f\r\n" % size)
    for n in range(1, size):
        f.write(b"%010d 00000 n\r\n" % offsets[n])
    f.write(b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref))
