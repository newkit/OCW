#!/usr/bin/env python3
"""The WebAssembly features a module was built with (spec 14 Phase 10, Q10).

    scripts/wasm_features.py MODULE.wasm [--require simd128]

Reads the module's `target_features` custom section (written by the linker,
wasm-ld: one entry per feature, `+name` used / `-name` disallowed) and prints
the used ones. `--require F` exits 1 when F is not among them (or the module
has no such section): scripts/build-web.sh checks that both builds of the
browser app use SIMD (Q10 (a): `+simd128`, .cargo/config.toml), and
scripts/web-preview-site.py that the preview's modules do.
"""

import argparse
import sys


def _leb(b, i):
    r = s = 0
    while True:
        x = b[i]
        i += 1
        r |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return r, i


def features(data):
    """The used features (`+name`) of a module's `target_features` section,
    or None when it has none."""
    if data[:4] != b"\0asm":
        raise ValueError("not a WebAssembly module")
    i, found = 8, None
    while i < len(data):
        sid = data[i]
        i += 1
        size, i = _leb(data, i)
        end = i + size
        if sid == 0:
            n, j = _leb(data, i)
            name = data[j:j + n].decode("utf-8", "replace")
            j += n
            if name == "target_features":
                found = set() if found is None else found
                count, j = _leb(data, j)
                for _ in range(count):
                    prefix = data[j]
                    j += 1
                    n, j = _leb(data, j)
                    feat = data[j:j + n].decode("utf-8", "replace")
                    j += n
                    if prefix in (ord("+"), ord("=")):
                        found.add(feat)
        i = end
    return found


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("module")
    p.add_argument("--require", action="append", default=[])
    a = p.parse_args()
    with open(a.module, "rb") as f:
        got = features(f.read())
    print(" ".join(sorted(got)) if got is not None else "(no target_features section)")
    missing = [r for r in a.require if got is None or r not in got]
    if missing:
        print(f"{a.module}: built without {', '.join(missing)}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
