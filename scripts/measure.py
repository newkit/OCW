#!/usr/bin/env python3
"""Runs a command and reports its wall time and peak memory (the largest
resident set of it and its children), for the package and client
measurements of spec §4.7 / §15:

    scripts/measure.py [--label TEXT] [--out FILE] -- CMD [ARGS...]

Prints `measure: LABEL: peak RSS X.XX GB, wall Y s, exit N` on stderr and,
with --out, appends the same line to FILE. Exits with the command's code.
(No /usr/bin/time on every build machine; this uses getrusage.)
"""
import resource
import subprocess
import sys
import time


def main() -> int:
    args = sys.argv[1:]
    label, out = None, None
    while args and args[0] != "--":
        if args[0] == "--label":
            label, args = args[1], args[2:]
        elif args[0] == "--out":
            out, args = args[1], args[2:]
        else:
            break
    if args and args[0] == "--":
        args = args[1:]
    if not args:
        sys.exit(__doc__)
    t = time.monotonic()
    code = subprocess.call(args)
    wall = time.monotonic() - t
    # ru_maxrss is in KiB on Linux.
    peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * 1024 / 1e9
    line = f"measure: {label or args[0]}: peak RSS {peak:.2f} GB, wall {wall:.0f} s, exit {code}"
    print(line, file=sys.stderr)
    if out:
        with open(out, "a", encoding="utf-8") as f:
            f.write(line + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
