#!/usr/bin/env python3
"""Generate crates/pdfi-font/assets/cmaps.bin: Adobe's predefined CMaps
(code -> CID; not the UTF-8 and UTF-32 ones, which PDF does not
predefine) and the CID -> Unicode maps of the character collections,
from the cmap-resources and mapping-resources-pdf of TeX Live's
`adobemapping` package (BSD-3-Clause, Adobe).

usage: scripts/gen-cmaps.py [adobemapping dir]

Layout (little endian): b"PDFICMAP", u32 count, then per entry a name
(u8 length, bytes), u32 offset and u32 length of a zlib stream relative
to the end of the directory. CMap entries inflate to:
  u8 kind (0), u8 wmode, str ordering, str usecmap (empty: none),
  u32 n, n × (u8 bytes, u32 low, u32 high)            codespace ranges
  u32 n, n × (u8 bytes, u32 low, u32 high, u32 cid)   sorted by (bytes, low)
CID -> Unicode entries ("Adobe-Japan1-UCS2", ...) inflate to:
  u8 kind (1),
  u32 n, n × (u32 low cid, u32 high cid, u8 k, k × u32 scalar)
where a range adds `cid - low` to the last scalar. Strings are u8 length
and bytes.
"""
import os
import re
import struct
import sys
import zlib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
SRC = sys.argv[1] if len(sys.argv) > 1 else "/usr/local/texlive/2026/texmf-dist/fonts/cmap/adobemapping"
OUT = os.path.join(ROOT, "crates/pdfi-font/assets/cmaps.bin")

TOKEN = re.compile(rb"<[0-9A-Fa-f\s]*>|\[|\]|/[^\s/<>\[\]()]+|\([^)]*\)|[^\s/<>\[\]()]+")


def hexval(t):
    h = re.sub(rb"\s", b"", t[1:-1])
    return len(h) // 2, int(h, 16) if h else 0


def tokens(path):
    data = open(path, "rb").read()
    data = b"\n".join(l for l in data.split(b"\n") if not l.lstrip().startswith(b"%"))
    return TOKEN.findall(data)


def s(b):
    return struct.pack("<B", len(b)) + b


def merge(entries):
    """(bytes, low, high, cid) sorted, with runs joined."""
    out = []
    for e in sorted(entries):
        if out:
            n, lo, hi, cid = out[-1]
            if e[0] == n and e[1] == hi + 1 and e[3] == cid + (hi - lo) + 1:
                out[-1] = (n, lo, e[2], cid)
                continue
        out.append(e)
    return out


def cmap(path):
    t = tokens(path)
    wmode, ordering, use = 0, b"", b""
    space, cids = [], []
    i = 0
    while i < len(t):
        w = t[i]
        if w == b"usecmap" and t[i - 1].startswith(b"/"):
            use = t[i - 1][1:]
        elif w == b"/WMode":
            wmode = int(t[i + 1])
        elif w == b"/Ordering":
            ordering = t[i + 1][1:-1]
        elif w == b"begincodespacerange":
            i += 1
            while t[i] != b"endcodespacerange":
                (n, lo), (_, hi) = hexval(t[i]), hexval(t[i + 1])
                space.append((n, lo, hi))
                i += 2
        elif w == b"begincidrange":
            i += 1
            while t[i] != b"endcidrange":
                (n, lo), (_, hi) = hexval(t[i]), hexval(t[i + 1])
                cids.append((n, lo, hi, int(t[i + 2])))
                i += 3
        elif w == b"begincidchar":
            i += 1
            while t[i] != b"endcidchar":
                n, c = hexval(t[i])
                cids.append((n, c, c, int(t[i + 1])))
                i += 2
        i += 1
    cids = merge(cids)
    out = struct.pack("<BB", 0, wmode) + s(ordering) + s(use)
    out += struct.pack("<I", len(space)) + b"".join(struct.pack("<BII", *r) for r in space)
    out += struct.pack("<I", len(cids)) + b"".join(struct.pack("<BIII", *r) for r in cids)
    return out


def scalars(t):
    """UTF-16BE hex to scalars."""
    h = re.sub(rb"\s", b"", t[1:-1])
    return [ord(c) for c in bytes.fromhex(h.decode()).decode("utf-16-be", "replace")]


def to_unicode(path):
    t = tokens(path)
    ent = []
    i = 0
    while i < len(t):
        w = t[i]
        if w == b"beginbfchar":
            i += 1
            while t[i] != b"endbfchar":
                _, c = hexval(t[i])
                ent.append((c, c, scalars(t[i + 1])))
                i += 2
        elif w == b"beginbfrange":
            i += 1
            while t[i] != b"endbfrange":
                (_, lo), (_, hi) = hexval(t[i]), hexval(t[i + 1])
                if t[i + 2] == b"[":
                    j = i + 3
                    c = lo
                    while t[j] != b"]":
                        ent.append((c, c, scalars(t[j])))
                        c += 1
                        j += 1
                    i = j + 1
                else:
                    ent.append((lo, hi, scalars(t[i + 2])))
                    i += 3
        i += 1
    ent.sort()
    merged = []
    for lo, hi, u in ent:
        if merged:
            plo, phi, pu = merged[-1]
            if lo == phi + 1 and len(u) == 1 and len(pu) == 1 and u[0] == pu[0] + (phi - plo) + 1:
                merged[-1] = (plo, hi, pu)
                continue
        merged.append((lo, hi, u))
    out = struct.pack("<BI", 1, len(merged))
    for lo, hi, u in merged:
        out += struct.pack("<IIB", lo, hi, len(u)) + b"".join(struct.pack("<I", x) for x in u)
    return out


def main():
    entries = []
    res = os.path.join(SRC, "cmap-resources")
    for coll in sorted(os.listdir(res)):
        d = os.path.join(res, coll, "CMap")
        if not os.path.isdir(d):
            continue
        for name in sorted(os.listdir(d)):
            # PDF predefines no UTF-8 or UTF-32 CMaps.
            if "UTF8" in name or "UTF32" in name:
                continue
            entries.append((name, cmap(os.path.join(d, name))))
    uni = os.path.join(SRC, "mapping-resources-pdf", "pdf2unicode")
    for name in sorted(os.listdir(uni)):
        entries.append((name, to_unicode(os.path.join(uni, name))))
    entries.sort()
    names = [n for n, _ in entries]
    assert len(names) == len(set(names)), "duplicate names"
    blobs = [zlib.compress(b, 9) for _, b in entries]
    head = b"PDFICMAP" + struct.pack("<I", len(entries))
    off = 0
    for (name, _), b in zip(entries, blobs):
        head += s(name.encode()) + struct.pack("<II", off, len(b))
        off += len(b)
    with open(OUT, "wb") as f:
        f.write(head + b"".join(blobs))
    print(f"{len(entries)} entries, {len(head) + off} bytes -> {OUT}")


if __name__ == "__main__":
    main()
