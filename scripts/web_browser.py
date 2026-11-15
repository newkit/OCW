#!/usr/bin/env python3
"""The browser of the browser tests (spec §11.4, §14 Phase 10): a pinned
Chromium build, downloaded once and checked against its SHA-256.

    python3 scripts/web_browser.py fetch      download + unpack the pinned build (`make deps-browser`)
    python3 scripts/web_browser.py path       the browser the tests use
    python3 scripts/web_browser.py version    its path and `--version`

The pin is scripts/browsers.json: open-source Chromium (BSD-3-Clause, with
the licences of its third-party parts in the build's about:credits) from
Chromium's own snapshot builds (the `chromium-browser-snapshots` bucket),
one revision per platform with the SHA-256 of its zip. It is unpacked into
~/.cache/ocw/browsers/<name>-<version>/ (OCW_BROWSER_CACHE overrides
~/.cache/ocw/browsers).

The tests (scripts/test-web*.py) take the browser from `find()`:
  1. OCW_BROWSER (a path or a command name),
  2. the pinned build, when `make deps-browser` has unpacked it,
  3. the system's chromium / chromium-browser / google-chrome (a warning
     says the pinned one is missing).
"""

import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
PINS = os.path.join(HERE, "browsers.json")
SYSTEM = ("chromium", "chromium-browser", "google-chrome", "google-chrome-stable")
MARKER = "ocw-browser.json"


def pins(path=PINS):
    with open(path) as f:
        return json.load(f)


def platform_key(system=None, machine=None):
    """`linux-x64`, `mac-arm64`, `win-x64` (None elsewhere)."""
    system = (system or platform.system()).lower()
    machine = (machine or platform.machine()).lower()
    arch = {"x86_64": "x64", "amd64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(machine)
    osname = {"linux": "linux", "darwin": "mac", "windows": "win"}.get(system)
    return f"{osname}-{arch}" if osname and arch else None


def cache_root():
    return os.environ.get("OCW_BROWSER_CACHE") or os.path.join(
        os.path.expanduser("~"), ".cache", "ocw", "browsers")


def pin_for(p, key):
    """The pin of platform `key`, with the common fields merged in."""
    plat = p["platforms"].get(key)
    if plat is None:
        return None
    return {"name": p["name"], "version": p["version"], **plat, "platform": key}


def install_dir(pin, root=None):
    return os.path.join(root or cache_root(), f"{pin['name']}-{pin['version']}")


def installed(pin, root=None):
    """The executable of the pinned build if it is unpacked and its marker
    names the pinned checksum, else None."""
    d = install_dir(pin, root)
    exe = os.path.join(d, pin["exe"])
    try:
        with open(os.path.join(d, MARKER)) as f:
            mark = json.load(f)
    except (OSError, ValueError):
        return None
    if mark.get("sha256") != pin["sha256"] or not os.path.isfile(exe):
        return None
    return exe


def find(warn=True, p=None, key=None, root=None, which=shutil.which):
    """The browser the tests start (see the module's doc)."""
    env = os.environ.get("OCW_BROWSER")
    if env:
        return which(env) or env
    try:
        p = p or pins()
    except OSError:
        p = None
    pin = pin_for(p, key or platform_key()) if p else None
    exe = installed(pin, root) if pin else None
    if exe:
        return exe
    for name in SYSTEM:
        path = which(name)
        if path:
            if warn and pin:
                print(f"web_browser: the pinned {pin['name']} {pin['version']} is not installed "
                      f"(make deps-browser); using {path}", file=sys.stderr)
            return path
    return "chromium"


def version(exe):
    """`exe --version` (e.g. "Chromium 157.0.8103.0"), or None."""
    try:
        out = subprocess.run([exe, "--version"], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return out.stdout.strip() or None


def describe(exe):
    """For the tests' summaries: the path, `--version` and whether it is the pin."""
    pinned = False
    try:
        pin = pin_for(pins(), platform_key())
        pinned = bool(pin) and installed(pin) == exe
    except OSError:
        pass
    return {"path": exe, "version": version(exe), "pinned": pinned}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url, dest, progress=True):
    """`url` to the file `dest`; returns the SHA-256 of what arrived."""
    h = hashlib.sha256()
    with urllib.request.urlopen(url, timeout=60) as r, open(dest, "wb") as f:
        total = int(r.headers.get("Content-Length") or 0)
        got, shown = 0, -1
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            f.write(chunk)
            h.update(chunk)
            got += len(chunk)
            pct = got * 100 // total if total else -1
            if progress and total and pct // 10 != shown:
                shown = pct // 10
                print(f"  {got / 1048576:.0f} of {total / 1048576:.0f} MiB", file=sys.stderr)
    return h.hexdigest()


def safe_name(name):
    """A member name that stays inside the target directory."""
    norm = os.path.normpath(name)
    return not (os.path.isabs(name) or norm.startswith("..") or name.startswith(("/", "\\"))
                or ":" in name.split("/")[0])


def unzip(archive, dest):
    """Unpacks with the members' Unix modes and symlinks (macOS app bundles
    have both; zipfile alone keeps neither)."""
    with zipfile.ZipFile(archive) as z:
        for info in z.infolist():
            if not safe_name(info.filename):
                raise ValueError(f"unsafe path in the archive: {info.filename}")
            target = os.path.join(dest, info.filename)
            mode = info.external_attr >> 16
            if info.is_dir():
                os.makedirs(target, exist_ok=True)
                continue
            os.makedirs(os.path.dirname(target), exist_ok=True)
            if stat.S_ISLNK(mode):
                link = z.read(info).decode()
                resolved = os.path.normpath(os.path.join(os.path.dirname(info.filename), link))
                if os.path.isabs(link) or resolved == ".." or resolved.startswith(".." + os.sep):
                    raise ValueError(f"unsafe symlink in the archive: {info.filename} -> {link}")
                if os.path.lexists(target):
                    os.remove(target)
                os.symlink(link, target)
                continue
            with z.open(info) as src, open(target, "wb") as out:
                shutil.copyfileobj(src, out, 1 << 20)
            if mode & 0o777:
                os.chmod(target, mode & 0o777)


def fetch(pin, root=None, progress=True):
    """Downloads, checks and unpacks `pin`; returns the executable."""
    exe = installed(pin, root)
    if exe:
        print(f"{pin['name']} {pin['version']} already installed: {exe}", file=sys.stderr)
        return exe
    root = root or cache_root()
    os.makedirs(root, exist_ok=True)
    final = install_dir(pin, root)
    work = tempfile.mkdtemp(prefix=".fetch-", dir=root)
    try:
        archive = os.path.join(work, "browser.zip")
        print(f"downloading {pin['name']} {pin['version']} ({pin['platform']}): {pin['url']}",
              file=sys.stderr)
        got = download(pin["url"], archive, progress)
        if got != pin["sha256"]:
            raise ValueError(f"{pin['url']}: SHA-256 {got}, expected {pin['sha256']}")
        unpacked = os.path.join(work, "unpacked")
        os.makedirs(unpacked)
        unzip(archive, unpacked)
        os.remove(archive)
        if not os.path.isfile(os.path.join(unpacked, pin["exe"])):
            raise ValueError(f"the archive has no {pin['exe']}")
        with open(os.path.join(unpacked, MARKER), "w") as f:
            json.dump({k: pin[k] for k in ("name", "version", "platform", "url", "sha256")}, f, indent=2)
        if os.path.isdir(final):
            shutil.rmtree(final)
        os.rename(unpacked, final)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    exe = os.path.join(final, pin["exe"])
    print(f"installed {exe}", file=sys.stderr)
    return exe


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    cmd = argv[0] if argv else "path"
    if cmd == "fetch":
        key = platform_key()
        pin = pin_for(pins(), key) if key else None
        if pin is None:
            sys.exit(f"web_browser: no pinned browser for {platform.system()} {platform.machine()} "
                     "(scripts/browsers.json); the tests use the system's chromium")
        exe = fetch(pin)
        print(exe)
        v = version(exe)
        if v:
            print(v)
    elif cmd == "path":
        print(find())
    elif cmd == "version":
        exe = find()
        print(exe)
        print(version(exe) or "(no --version)")
    else:
        sys.exit(f"usage: {sys.argv[0]} fetch|path|version")


if __name__ == "__main__":
    main()
