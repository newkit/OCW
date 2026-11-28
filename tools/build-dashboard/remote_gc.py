#!/usr/bin/env python3
"""scripts/remote-gc.sh: the build machines' cleanup by hand (spec 11.2).

  scripts/remote-gc.sh [-v] [MACHINE...]       dry run: what the rules would delete
  GC=1 scripts/remote-gc.sh [MACHINE...]       delete            (make remote-gc [GC=1])
  scripts/remote-gc.sh --branch BRANCH [--dry-run] [MACHINE...]
        cleanup on merge: delete the target/ of BRANCH's checkout on every
        reachable machine at once, whatever the rules say (make remote-gc-branch
        BRANCH=…); the coordinator runs it after merging or abandoning a branch
  GC=1 scripts/remote-gc.sh --only NAME [MACHINE...]
        only the items whose path contains NAME (a checkout, "tmp/", "~/ocw-release")
  scripts/remote-gc.sh --selftest MACHINE...
        makes a scratch checkout there (<repo>-gc-selftest-N with paths longer
        than 260 characters in target/ and in a leftover target.ocw-gc.1) and
        deletes it with the real snippets: the check of the Windows deletion

The rules are those of tools/build-dashboard/cleanup_rules.py – the same the
dashboard's janitor applies once a day on every enabled machine:

@RULES@

Thresholds (environment, also read by the janitor):
@LIMITS@

Default: every machine of config/machines.toml; unreachable ones are
skipped. Prints per machine every deletion with its size and rule, what is
kept and why (-v: also every checkout without target/), totals per class
and the space freed (or that would be freed). Only machine names are
printed, never hosts. A checkout with a live task marker (running or
waiting) is never touched: the machine checks again right before deleting,
and a target/ or checkout is renamed before it is removed. Every deletion
is logged to test-output/build-log/DAY/janitor.jsonl of the main checkout
(machine, path, size, reason), like the janitor's (OCW_BUILD_LOG=0: no log).
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import cleanup_rules as rules
import janitor

SOURCE = "remote-gc.sh"


def help_text() -> str:
    doc = rules.__doc__.split("\n\n")
    body = "\n\n".join(doc[1:-1])  # the rules, without the module's first and last paragraphs
    return __doc__.replace("@RULES@", body).replace("@LIMITS@", rules.limits_help())


def selftest_script(m, name: str) -> str:
    """Makes the scratch checkout `name` next to the main one: sources, a
    target/ and a leftover target.ocw-gc.1, both with a file 330 characters
    deep (beyond Windows' MAX_PATH)."""
    if m.os == "windows":
        return r"""
$ErrorActionPreference = 'Stop'
$c = Join-Path (Split-Path @REPO@) @NAME@; $deep = (('d' * 40) + '\') * 8
foreach ($t in 'target', 'target.ocw-gc.1') { $q = '\\?\' + (Join-Path (Join-Path $c $t) $deep); [void][IO.Directory]::CreateDirectory($q); [IO.File]::WriteAllText($q + 'f.txt', 'x') }
[void](New-Item -ItemType Directory -Force -Path (Join-Path $c 'crates')); [IO.File]::WriteAllText((Join-Path $c 'crates\lib.rs'), 'x')
'@@made'
""".replace("@REPO@", rules._psq(m.repo_path)).replace("@NAME@", rules._psq(name))
    return r"""
r=@REPO@; c=${r%/*}/@NAME@; deep=$(printf 'dddddddddddddddddddddddddddddddddddddddd/%.0s' 1 2 3 4 5 6 7 8)
for t in target target.ocw-gc.1; do mkdir -p "$c/$t/$deep" && echo x >"$c/$t/${deep}f.txt" || exit 1; done
mkdir -p "$c/crates" && echo x >"$c/crates/lib.rs" && echo @@made
""".replace("@REPO@", rules._sh_path(m.repo_path)).replace("@NAME@", name)


def selftest(m, ssh) -> tuple:
    """(lines, ok): the real delete snippets against a scratch checkout."""
    name = "%s-gc-selftest-%d" % (rules.base_name(m.repo_path), int(time.time()))
    lines, ok = ["== %s: selftest with %s" % (m.name, name)], True
    try:
        if "@@made" not in ssh(m, selftest_script(m, name), 120):
            return lines + ["  FAIL could not make the scratch checkout"], False
        steps = [(["target|" + name, "stray|%s/target.ocw-gc.1" % name], False, "would"),
                 (["target|" + name, "stray|%s/target.ocw-gc.1" % name], True, "deleted"),
                 (["target|" + name], True, "gone"), (["checkout|" + name], True, "deleted"),
                 (["checkout|" + name], True, "gone")]
        for ops, delete, want in steps:
            res = janitor.run_ops(ssh, m, ops, dry=not delete)
            for op in ops:
                got = res.get(op, {}).get("result")
                ok = ok and got == want
                lines.append("  %-4s %-9s %s" % ("ok" if got == want else "FAIL", got, op))
    except Exception as e:
        return lines + ["  FAIL %s: %s" % (type(e).__name__, e)], False
    return lines, ok


def gc_machine(m, ssh, facts, lim, delete: bool, branch: str | None, verbose: bool, log, only: str | None = None) -> tuple:
    """(lines, bytes freed or to free) for one machine."""
    try:
        ssh(m, "'@@ok'" if m.os == "windows" else "echo @@ok", 45)
    except Exception as e:
        return ["== %s: unreachable, skipped (%s)" % (m.name, e)], 0
    base = rules.base_name(m.repo_path)
    lines = []
    try:
        if branch:
            items = [rules.branch_item(base, branch)]
        else:
            inv = janitor.inventory(ssh, m)
            p = rules.plan(inv, facts, lim, time.time())
            if only:
                p = dict(p, items=[i for i in p["items"] if only in i["path"]])
            items = p["items"]
            disks = ", ".join("%s %s%% used, %s free" % (d["name"], janitor.disk_pct(d), rules.gb(d.get("free")))
                              for d in inv["disks"])
            lines.append("== %s: %s; %d checkouts" % (m.name, disks, len(inv["checkouts"])))
            lines += rules.plan_text(p, verbose, delete)
        todo = [i for i in items if i["delete"]]
        if branch:
            lines.append("== %s" % m.name)
        if not delete and not branch:
            freed = sum(i["bytes"] for i in todo)
        else:
            res = janitor.run_ops(ssh, m, [i["op"] for i in todo], dry=not delete)
            freed = 0
            for i in todo:
                r = res.get(i["op"], {"result": "failed"})
                size = r.get("bytes") or 0
                if r["result"] in ("deleted", "would"):
                    freed += size
                if r["result"] == "gone" and branch:
                    lines.append("  (no target/ of %s)" % i["name"])
                elif r["result"] != "gone":
                    word = {"deleted": "deleted", "would": "would delete", "busy": "in use now, kept",
                            "kept": "kept (output in it)", "refused": "refused", "failed": "DELETE FAILED"}[r["result"]]
                    lines.append("  %-16s %9s  %-50s %s" % (word, rules.gb(size), i["path"], i["reason"]))
                if r["result"] in ("deleted", "failed"):
                    log({"machine": m.name, "path": i["path"], "bytes": size, "reason": i["reason"], "kind": i["kind"],
                         "class": i["cls"], "action": "delete" if r["result"] == "deleted" else "failed",
                         "mode": "on", "source": SOURCE})
    except Exception as e:
        return lines + ["== %s: failed (%s: %s)" % (m.name, type(e).__name__, e)], 0
    lines.append("  %s %s on %s" % ("freed" if delete else "would free" + ("" if branch else " (GC=1)"),
                                    rules.gb(freed), m.name))
    return lines, freed


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="remote-gc.sh", add_help=False)
    ap.add_argument("-h", "--help", action="store_true")
    ap.add_argument("--branch")
    ap.add_argument("--only")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("-n", "--dry-run", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--config", type=Path)
    ap.add_argument("machines", nargs="*")
    a = ap.parse_args(argv)
    if a.help:
        print(help_text())
        return 0
    gc_env = os.environ.get("GC", "")
    delete = gc_env == "1" or (bool(a.branch) and gc_env != "0")
    if a.dry_run:  # --dry-run wins over --branch's default of deleting
        delete = False
    if a.branch is not None and (a.branch in ("main", "HEAD", "") or not re.fullmatch(r"[A-Za-z0-9._/+-]+", a.branch)):
        print("remote-gc: refusing branch '%s'" % a.branch, file=sys.stderr)
        return 2
    import dashboard  # the machines of config/machines.toml and their ssh
    repo = Path(__file__).resolve().parent.parent.parent
    cfg = a.config or dashboard.find_config_dir(repo)
    if not (cfg / "machines.toml").is_file():
        print("%s/machines.toml missing" % cfg, file=sys.stderr)
        return 1
    machines = dashboard.load_machines(cfg)
    if a.machines:
        by = {m.name: m for m in machines}
        unknown = [n for n in a.machines if n not in by]
        if unknown:
            print("remote-gc: unknown machine(s): %s" % " ".join(unknown), file=sys.stderr)
            return 2
        machines = [by[n] for n in a.machines]
    main_repo = dashboard.main_checkout(repo)
    log_root = None if os.environ.get("OCW_BUILD_LOG", "1") == "0" else main_repo / "test-output" / "build-log"
    logger = janitor.Janitor([], main_repo, log_root, "on")
    ssh = lambda m, script, timeout, stdin=None: dashboard.run_ssh(m, script, timeout, stdin, None)  # noqa: E731
    if a.selftest:
        if not a.machines:
            print("remote-gc: --selftest needs machine names", file=sys.stderr)
            return 2
        bad = 0
        for m in machines:
            lines, ok = selftest(m, ssh)
            print("\n".join(lines))
            bad += not ok
        return 1 if bad else 0
    facts = None if a.branch else rules.git_facts(main_repo)
    lim = rules.limits()
    with ThreadPoolExecutor(max_workers=max(1, len(machines))) as ex:
        results = list(ex.map(lambda m: gc_machine(m, ssh, facts, lim, delete, a.branch, a.verbose, logger.log, a.only),
                              machines))
    total = 0
    for lines, freed in results:
        print("\n".join(lines))
        total += freed
    print("total: %s %s" % ("freed" if delete else "would free", rules.gb(total)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
