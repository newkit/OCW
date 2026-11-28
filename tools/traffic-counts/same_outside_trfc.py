#!/usr/bin/env python3
"""Checks that two world packages differ only in TRFC's flow factors (spec
18E T11: `world-builder traffic-counts` keeps every other byte).

    python3 -I tools/traffic-counts/same_outside_trfc.py OLD.ocwpkg NEW.ocwpkg

Walks the OCWPKG02 layout (head, tiles, sections), compares the bytes
before the sections and every section's payload, and inside TRFC every
byte but the edges' flow (the last u16 of each 28-byte edge record).
Exit 0 when only flows differ; prints the count of changed edges.
"""

import mmap
import struct
import sys


def spans(b):
    if b[:8] != b"OCWPKG02":
        sys.exit("not an OCWPKG02 package")
    pos = 8
    (name_len,) = struct.unpack_from("<I", b, pos)
    pos += 4 + name_len + 16 + 4
    (samples,) = struct.unpack_from("<I", b, pos)
    pos += 4
    (tiles,) = struct.unpack_from("<I", b, pos)
    pos += 4
    for _ in range(tiles):
        pos += 8 + samples * samples * 4
        (roads,) = struct.unpack_from("<I", b, pos)
        pos += 4
        for _ in range(roads):
            pos += 5
            (pts,) = struct.unpack_from("<I", b, pos)
            pos += 4 + pts * 12
    start = pos
    (n,) = struct.unpack_from("<I", b, pos)
    pos += 4
    out = []
    for _ in range(n):
        tag = bytes(b[pos:pos + 4])
        (ln,) = struct.unpack_from("<Q", b, pos + 4)
        pos += 12
        out.append((tag, pos, ln))
        pos += ln
    if pos != len(b):
        sys.exit("trailing bytes")
    return start, out


def main():
    old_f, new_f = (open(p, "rb") for p in sys.argv[1:3])
    old = mmap.mmap(old_f.fileno(), 0, access=mmap.ACCESS_READ)
    new = mmap.mmap(new_f.fileno(), 0, access=mmap.ACCESS_READ)
    so, a = spans(old)
    sn, b = spans(new)
    bad = []
    if so != sn or old[:so] != new[:sn]:
        bad.append("head / tiles")
    if [t for t, _, _ in a] != [t for t, _, _ in b]:
        bad.append("section list")
    changed = 0
    for (t, po, lo), (_, pn, ln) in zip(a, b):
        if t != b"TRFC":
            if lo != ln or old[po:po + lo] != new[pn:pn + ln]:
                bad.append(t.decode())
            continue
        if lo != ln:
            bad.append("TRFC length")
            continue
        (edges,) = struct.unpack_from("<I", old, po + 3)
        rec = po + 7
        recn = pn + 7
        for i in range(edges):
            o, n = old[rec + i * 28:rec + i * 28 + 28], new[recn + i * 28:recn + i * 28 + 28]
            if o[:26] != n[:26]:
                bad.append(f"TRFC edge {i} outside flow")
                break
            changed += o[26:] != n[26:]
        end = 7 + edges * 28
        if old[po + end:po + lo] != new[pn + end:pn + ln] or old[po:po + 7] != new[pn:pn + 7]:
            bad.append("TRFC outside the edge records")
    sections = len(a)
    if bad:
        print(f"DIFFERENT: {', '.join(bad)}")
        sys.exit(1)
    print(f"same outside TRFC's flow: {sections} sections, {changed} edges with a new flow")


if __name__ == "__main__":
    main()
