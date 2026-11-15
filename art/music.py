"""The OCW Build Diary's music (art/ART-PROJECT.md §3.2): synthesised, no
samples, no licences, deterministic from the seed.

Every day has its own style (a band, a mode, a groove, a set of chord
progressions): `chiptune`, `synthwave`, `funk`, `house`, `lofi`. A day script
can name one (`style`, `mode`); by default the day number picks it. Within a
video the music follows the sections – intro (title), drive (time-lapse),
one groove per chapter, outro (end card) – and varies: a progression per
chapter, a bridge and a key lift in long videos, drum patterns that change
(half-time, breaks, drop-outs), fills, build-ups and stops at chapter
changes, crashes on the downbeat, a lead melody that develops (a motif,
its variations, call and response), and filter and volume swells.

    Music(bpm=120, key="G", seed=4, style=None, mode=None).render(seconds, segments, duck)

`segments` is [(start seconds, kind)] in time order, kind one of intro,
drive, groove, outro. Bars are laid out from t = 0; a bar belongs to the
segment that holds its middle.
"""

from __future__ import annotations

import math
import random
import zlib
from array import array

RATE = 44100

NOTE = {"C": 0, "C#": 1, "Db": 1, "D": 2, "D#": 3, "Eb": 3, "E": 4, "F": 5, "F#": 6, "Gb": 6,
        "G": 7, "G#": 8, "Ab": 8, "A": 9, "A#": 10, "Bb": 10, "B": 11}

MODES = {
    "major": [0, 2, 4, 5, 7, 9, 11], "ionian": [0, 2, 4, 5, 7, 9, 11],
    "dorian": [0, 2, 3, 5, 7, 9, 10], "mixolydian": [0, 2, 4, 5, 7, 9, 10],
    "minor": [0, 2, 3, 5, 7, 8, 10], "aeolian": [0, 2, 3, 5, 7, 8, 10],
    "lydian": [0, 2, 4, 6, 7, 9, 11],
}

# Bass patterns: (16th step, length in steps, chord tone 0/1/2, octave).
BASS = {
    "oct8": [(s, 2, 0, (s // 2) % 2) for s in range(0, 16, 2)],
    "drive8": [(s, 2, 0, 0) for s in range(0, 16, 2)],
    "gallop": [(0, 2, 0, 0), (3, 1, 0, 0), (4, 2, 0, 1), (6, 2, 0, 0), (8, 2, 0, 0), (11, 1, 0, 0),
               (12, 2, 2, 0), (14, 2, 0, 1)],
    "pulse16": [(s, 1, 0, 0) for s in range(16)],
    "rootfifth": [(0, 4, 0, 0), (6, 2, 2, 0), (8, 4, 0, 0), (14, 2, 1, 0)],
    "sync16": [(0, 2, 0, 0), (3, 1, 0, 1), (6, 1, 2, 0), (8, 2, 0, 0), (10, 1, 0, 1), (11, 1, 1, 0),
               (14, 2, 2, 0)],
    "slap": [(0, 1, 0, 0), (2, 1, 0, 1), (4, 2, 0, 0), (7, 1, 2, 0), (8, 1, 0, 0), (10, 1, 0, 1),
             (12, 1, 1, 0), (13, 1, 2, 0), (14, 2, 0, 1)],
    "offbeat": [(2, 2, 0, 0), (6, 2, 0, 0), (10, 2, 0, 0), (14, 2, 0, 0)],
    "offroll": [(2, 2, 0, 0), (6, 2, 0, 0), (10, 2, 0, 0), (13, 1, 0, 1), (14, 2, 2, 0)],
    "lazy": [(0, 6, 0, 0), (7, 1, 2, 0), (10, 4, 0, 0)],
    "walk": [(0, 4, 0, 0), (4, 4, 1, 0), (8, 4, 2, 0), (12, 4, 1, 0)],
}

# Arpeggio orders over [c0, c1, c2, c0+12, c1+12, c2+12].
ARPS = {"up": [0, 1, 2, 3, 4, 5], "updown": [0, 1, 2, 3, 4, 5, 4, 3, 2, 1],
        "broken": [0, 2, 1, 3, 2, 4, 3, 5], "octave": [0, 3, 1, 4, 2, 5], "down": [5, 4, 3, 2, 1, 0],
        "pedal": [0, 3, 0, 4, 0, 5, 0, 4]}

# Drum patterns, 16 steps per voice: x hit, X accent, g ghost, . rest.
# k kick, s snare/clap, h closed hat, o open hat, r rim / shaker.
DRUMS = {
    "four": {"k": "x...x...x...x...", "s": "....x.......x...", "h": "x.x.x.x.x.x.x.x."},
    "drive": {"k": "x...x...x...x...", "s": "....X.......X...", "h": "xgxgxgxgxgxgxgxg"},
    "halftime": {"k": "x.........x.....", "s": "........X.......", "h": "x.x.x.x.x.x.x.x."},
    "break": {"k": "x.....x...x.....", "s": "....x..g.g..x..g", "h": "x.xxx.x.x.xxx.x."},
    "gated": {"k": "x.......x.x.....", "s": "....X.......X...", "h": "x.x.x.x.x.x.x.x."},
    "motorik": {"k": "x...x...x...x...", "s": "....x.......x...", "h": "xxxxxxxxxxxxxxxx"},
    "funk": {"k": "x..x..x...x..x..", "s": "....x..g.g..x..g", "h": "xgxgxgxgxgxgxgxg"},
    "funk2": {"k": "x.x...x..x.x....", "s": "....x.g....gx...", "h": "x.xgx.xgx.xgx.xg",
              "o": "......x........."},
    "house": {"k": "x...x...x...x...", "s": "....x.......x...", "o": "..x...x...x...x.",
              "r": "xgxgxgxgxgxgxgxg"},
    "house2": {"k": "x...x...x...x...", "s": "....x.......x..g", "h": "x.x.x.x.x.x.x.x.",
               "o": "..x...x...x...x."},
    "boombap": {"k": "x......x..x.....", "s": "....x.......x...", "h": "x.x.x.x.x.x.x.x."},
    "lofihalf": {"k": "x.........x..x..", "s": "........x.......", "h": "x...x...x...x...",
                 "r": "..g...g...g...g."},
    "sparse": {"k": "x.......x......."},
    "pulse": {"k": "x...x...x...x..."},
}

# Stab rhythms (chord hits): x hit; length of a hit in steps follows.
STABS = {
    "clav": ("....x..x...x.x..", 1), "clav2": ("..x..x.x..x...x.", 1),
    "offpiano": ("..x...x...x...x.", 2), "piano2": ("x..x..x...x..x..", 2),
    "keys": ("x.........x.....", 10), "keys2": ("x.....x.....x...", 6), "keys3": ("x.......x.......", 8),
}

STYLES = {
    "chiptune": {
        "mode": "major", "swing": 0.0,
        "progs": [[5, 3, 0, 4], [0, 4, 5, 3], [3, 4, 2, 5], [0, 5, 3, 4]], "bridge": [3, 4, 5, 4],
        "cadence": [3, 4, 0, 0], "sevenths": False, "lift": [1, 2],
        "bass": {"wave": "square", "gain": 0.15, "patterns": ["oct8", "drive8", "gallop"]},
        "harm": {"kind": "arp", "wave": "pulse", "gain": 0.075, "rate": 1, "octave": 12,
                 "patterns": ["up", "updown", "broken", "octave"], "echo": 0.0},
        "lead": {"wave": "pulse50", "gain": 0.075, "octave": 12, "density": 0.6, "p16": 0.25,
                 "legato": 0.85, "vibrato": 0.0},
        "pad": {"wave": "saw", "gain": 0.12, "lp": 0.04},
        "kit": "chip", "drive": "drive", "grooves": ["four", "halftime", "break", "motorik"],
        "fills": ["fill", "build", "stop", "fill"],
    },
    "synthwave": {
        "mode": "minor", "swing": 0.0,
        "progs": [[0, 5, 2, 6], [0, 3, 5, 4], [5, 6, 0, 0], [0, 6, 5, 6]], "bridge": [3, 4, 5, 6],
        "cadence": [5, 6, 0, 0], "sevenths": False, "lift": [2, 3],
        "bass": {"wave": "sawbass", "gain": 0.16, "patterns": ["pulse16", "drive8", "oct8"]},
        "harm": {"kind": "arp", "wave": "tri", "gain": 0.09, "rate": 1, "octave": 12,
                 "patterns": ["up", "down", "pedal", "broken"], "echo": 0.38},
        "lead": {"wave": "saw", "gain": 0.07, "octave": 12, "density": 0.32, "p16": 0.05,
                 "legato": 1.0, "vibrato": 0.006},
        "pad": {"wave": "supersaw", "gain": 0.2, "lp": 0.05},
        "kit": "synthwave", "drive": "motorik", "grooves": ["gated", "halftime", "four"],
        "fills": ["build", "fill", "stop"],
    },
    "funk": {
        "mode": "dorian", "swing": 0.12,
        "progs": [[0, 3, 0, 3], [0, 0, 3, 4], [0, 6, 3, 0], [1, 4, 0, 0]], "bridge": [2, 3, 4, 3],
        "cadence": [3, 4, 0, 0], "sevenths": True, "lift": [1, 2],
        "bass": {"wave": "pluckbass", "gain": 0.2, "patterns": ["sync16", "slap", "rootfifth"]},
        "harm": {"kind": "stab", "wave": "pluck", "gain": 0.07, "octave": 12,
                 "patterns": ["clav", "clav2"]},
        "lead": {"wave": "fm", "gain": 0.07, "octave": 12, "density": 0.5, "p16": 0.4,
                 "legato": 0.5, "vibrato": 0.0},
        "pad": {"wave": "organ", "gain": 0.05, "lp": 0.08},
        "kit": "funk", "drive": "funk", "grooves": ["funk", "funk2", "break"],
        "fills": ["fill", "stop", "fill", "build"],
    },
    "house": {
        "mode": "mixolydian", "swing": 0.0,
        "progs": [[0, 6, 3, 0], [0, 3, 6, 3], [5, 3, 0, 6], [0, 0, 6, 3]], "bridge": [1, 3, 4, 6],
        "cadence": [3, 6, 0, 0], "sevenths": True, "lift": [2, 1],
        "bass": {"wave": "sine", "gain": 0.24, "patterns": ["offbeat", "offroll", "rootfifth"]},
        "harm": {"kind": "stab", "wave": "piano", "gain": 0.06, "octave": 12,
                 "patterns": ["offpiano", "piano2"]},
        "lead": {"wave": "pluck", "gain": 0.07, "octave": 24, "density": 0.4, "p16": 0.2,
                 "legato": 0.7, "vibrato": 0.0},
        "pad": {"wave": "organ", "gain": 0.07, "lp": 0.05},
        "kit": "house", "drive": "house", "grooves": ["house", "house2", "halftime"],
        "fills": ["build", "stop", "build", "fill"],
    },
    "lofi": {
        "mode": "major", "swing": 0.18,
        "progs": [[1, 4, 0, 5], [3, 2, 1, 0], [0, 5, 1, 4], [3, 4, 2, 5]], "bridge": [3, 4, 2, 5],
        "cadence": [1, 4, 0, 0], "sevenths": True, "lift": [1, 2],
        "bass": {"wave": "sine", "gain": 0.24, "patterns": ["lazy", "walk", "rootfifth"]},
        "harm": {"kind": "stab", "wave": "fm", "gain": 0.07, "octave": 0,
                 "patterns": ["keys", "keys2", "keys3"]},
        "lead": {"wave": "tri", "gain": 0.075, "octave": 12, "density": 0.3, "p16": 0.1,
                 "legato": 0.9, "vibrato": 0.004},
        "pad": {"wave": "sinepad", "gain": 0.06, "lp": 0.03},
        "kit": "lofi", "drive": "boombap", "grooves": ["boombap", "lofihalf"],
        "fills": ["stop", "fill", "stop"], "crackle": True,
    },
}

# The automatic style per day number: day 1 synthwave, 2 funk, 3 lofi,
# 4 chiptune (the band days 1–4 all had before), 5 house, then around again.
AUTO = ["synthwave", "funk", "lofi", "chiptune", "house"]

KITS = {  # kick (f start, f end, decay s), snare (noise decay, tone Hz, tone gain), hat decay, brightness
    "chip": {"kick": (160, 45, 0.11), "snare": (0.08, 0.0, 0.0), "hat": 0.018, "bright": 1.0, "lp": 1.0},
    "synthwave": {"kick": (120, 42, 0.22), "snare": (0.28, 180.0, 0.5), "hat": 0.025, "bright": 0.7,
                  "lp": 1.0},
    "funk": {"kick": (110, 50, 0.14), "snare": (0.12, 210.0, 0.4), "hat": 0.02, "bright": 0.9, "lp": 1.0},
    "house": {"kick": (130, 42, 0.30), "snare": (0.10, 0.0, 0.0), "hat": 0.022, "bright": 0.85,
              "lp": 1.0, "clap": True},
    "lofi": {"kick": (95, 45, 0.20), "snare": (0.16, 190.0, 0.3), "hat": 0.03, "bright": 0.45,
             "lp": 0.35},
}

VEL = {"x": 1.0, "X": 1.25, "g": 0.35, "o": 1.0}


def stable(*parts) -> int:
    """A seed from values, the same in every process (unlike hash())."""
    return zlib.crc32(repr(parts).encode())


def hz(semitone_from_a4: float) -> float:
    return 440.0 * 2 ** (semitone_from_a4 / 12)


def style_for(day_number: int, style: str | None = None) -> str:
    if style:
        if style not in STYLES:
            raise SystemExit(f"unknown music style {style!r}; one of {', '.join(STYLES)}")
        return style
    return AUTO[(max(1, day_number) - 1) % len(AUTO)]


def _env(n, a, r):
    a, r = max(1, min(a, n)), max(1, min(r, n))
    return [min(1.0, i / a, (n - i) / r) for i in range(n)]


def _lowpass(xs, alpha):
    out, y = [], 0.0
    for x in xs:
        y += alpha * (x - y)
        out.append(y)
    return out


def _add(out, start, data, gain=1.0):
    """out[start:] += data * gain (clipped at the end of out)."""
    if start < 0:
        data, start = data[-start:], 0
    end = min(len(out), start + len(data))
    if end <= start:
        return
    seg = out[start:end]
    if gain == 1.0:
        out[start:end] = [a + b for a, b in zip(seg, data)]
    else:
        out[start:end] = [a + b * gain for a, b in zip(seg, data)]


class Music:
    TAIL = round(1.6 * RATE)  # notes and cymbals ring into the next bar

    def __init__(self, bpm=108, key="C", seed=1, style=None, mode=None):
        self.bpm, self.seed = bpm, seed
        self.style_name = style_for(seed, style)
        self.st = STYLES[self.style_name]
        self.mode_name = mode or self.st["mode"]
        if self.mode_name not in MODES:
            raise SystemExit(f"unknown mode {self.mode_name!r}; one of {', '.join(MODES)}")
        self.scale = MODES[self.mode_name]
        self.key = key
        self.root = NOTE[key] - 9  # the tonic around middle C, relative to A4
        self.beat = 60.0 / bpm
        self.bar_n = round(4 * self.beat * RATE)
        self.step = self.bar_n / 16
        self.kit = KITS[self.st["kit"]]
        self._tones, self._drums, self._bars = {}, {}, {}

    def describe(self) -> str:
        return f"{self.style_name} in {self.key} {self.mode_name}, {self.bpm} bpm"

    # ---------------------------------------------------------- harmony
    def deg(self, d: int) -> int:
        """Semitones of scale degree d (0 = tonic; may be negative or > 6)."""
        return 12 * (d // 7) + self.scale[d % 7]

    def chord(self, degree: int, shift: int) -> tuple:
        n = 4 if self.st["sevenths"] else 3
        base = self.root + shift
        return tuple(base + self.deg(degree + 2 * j) for j in range(n))

    # ---------------------------------------------------------- sounds
    def tone(self, wave, semi, n, vib=0.0):
        key = (wave, semi, n, vib)
        t = self._tones.get(key)
        if t is not None:
            return t
        f, R = hz(semi), RATE
        w = 2 * math.pi * f / R
        inc = f / R
        if wave == "square":  # bass: square + sine
            e = _env(n, 60, 900)
            t = [((0.6 if (i * inc) % 1 < 0.5 else -0.6) + 0.4 * math.sin(w * i)) * e[i] for i in range(n)]
        elif wave == "pulse":
            e = _env(n, 30, 1500)
            t = [(0.6 if (i * inc) % 1 < 0.25 else -0.4) * e[i] for i in range(n)]
        elif wave == "pulse50":
            e = _env(n, 80, 2000)
            t = [(0.5 if (i * inc) % 1 < 0.5 else -0.5) * e[i] * (0.75 + 0.25 * math.exp(-i / (0.3 * R)))
                 for i in range(n)]
        elif wave in ("saw", "sawbass"):
            e = _env(n, 200 if wave == "saw" else 40, 1500)
            if vib:
                ph, raw = 0.0, []
                for i in range(n):
                    ph += inc * (1 + vib * math.sin(2 * math.pi * 5.2 * i / R) * min(1.0, i / (0.3 * R)))
                    raw.append(2 * (ph % 1) - 1)
            else:
                raw = [2 * ((i * inc) % 1) - 1 for i in range(n)]
            raw = _lowpass(raw, 0.12 if wave == "saw" else 0.07)
            sub = 0.5 if wave == "sawbass" else 0.0
            t = [(raw[i] * 1.4 + sub * math.sin(w * i)) * e[i] for i in range(n)]
        elif wave == "tri":
            e = _env(n, 150, 2500)
            if vib:
                ph, t = 0.0, []
                for i in range(n):
                    ph += inc * (1 + vib * math.sin(2 * math.pi * 4.8 * i / R) * min(1.0, i / (0.3 * R)))
                    t.append((4 * abs(ph % 1 - 0.5) - 1) * e[i])
            else:
                t = [(4 * abs((i * inc) % 1 - 0.5) - 1) * e[i] for i in range(n)]
        elif wave == "sine":
            e = _env(n, 120, 1500)
            t = [(math.sin(w * i) + 0.15 * math.sin(2 * w * i)) * e[i] for i in range(n)]
        elif wave == "fm":  # electric piano / bell lead
            e = _env(n, 40, 2500)
            t = [math.sin(w * i + (2.2 * math.exp(-i / (0.18 * R)) + 0.25) * math.sin(w * i))
                 * math.exp(-i / (1.1 * R)) * e[i] for i in range(n)]
        elif wave == "pluck":  # Karplus-Strong
            p = max(2, round(R / f))
            rng = random.Random(stable("pluck", semi))
            buf = [rng.uniform(-1, 1) for _ in range(p)]
            t = []
            for i in range(n):
                v = buf[i % p]
                nxt = buf[(i + 1) % p]
                buf[i % p] = 0.498 * (v + nxt)
                t.append(v)
            e = _env(n, 5, 600)
            t = [t[i] * e[i] * 0.9 for i in range(n)]
        elif wave == "pluckbass":  # saw with a closing filter (a wah)
            raw = [2 * ((i * inc) % 1) - 1 for i in range(n)]
            out, y = [], 0.0
            for i, x in enumerate(raw):
                y += (0.03 + 0.35 * math.exp(-i / (0.06 * R))) * (x - y)
                out.append(y)
            e = _env(n, 20, 800)
            t = [(out[i] * 1.3 + 0.35 * math.sin(w * i)) * e[i] for i in range(n)]
        elif wave == "piano":
            e = _env(n, 20, 1500)
            t = [sum(math.sin(w * k * i) / k ** 1.4 * math.exp(-i * k / (0.6 * R)) for k in (1, 2, 3, 4))
                 * e[i] for i in range(n)]
        elif wave == "organ":
            e = _env(n, 3000, 3000)
            t = [(math.sin(w * i) + 0.5 * math.sin(2 * w * i) + 0.25 * math.sin(3 * w * i)) * e[i] * 0.6
                 for i in range(n)]
        else:
            raise ValueError(wave)
        self._tones[key] = t
        return t

    def pad(self, chord, n, alpha):
        key = ("pad", chord, n, alpha)
        t = self._tones.get(key)
        if t is not None:
            return t
        wave = self.st["pad"]["wave"]
        fs = [hz(s) / RATE for s in chord]
        if wave == "organ":
            t = [0.0] * n
            for s in chord:
                _add(t, 0, self.tone("organ", s, n), 1 / len(chord))
        elif wave == "sinepad":
            e = _env(n, 9000, 9000)
            t = [sum(math.sin(2 * math.pi * f * i) for f in fs) * e[i] / len(fs) for i in range(n)]
        else:
            det = (-0.004, 0.0, 0.004) if wave == "supersaw" else (-0.003, 0.003)
            raw = [sum(((i * f * (1 + d)) % 1.0) * 2 - 1 for f in fs for d in det) for i in range(n)]
            e = _env(n, 9000, 9000)
            lp = _lowpass(raw, alpha)
            t = [lp[i] * e[i] / (len(fs) * len(det) / 2) for i in range(n)]
        self._tones[key] = t
        return t

    def drum(self, kind, vel=1.0):
        key = (kind, vel)
        d = self._drums.get(key)
        if d is not None:
            return d
        R, kit = RATE, self.kit
        rng = random.Random(stable("drum", kind, self.st["kit"]))
        if kind == "k":
            f0, f1, dec = kit["kick"]
            n = round((dec * 4) * R)
            ph, d = 0.0, []
            for i in range(n):
                f = f1 + (f0 - f1) * math.exp(-i / (0.02 * R))
                ph += f / R
                d.append(math.sin(2 * math.pi * ph) * math.exp(-i / (dec * R)))
            d = [x * 0.9 for x in d]
        elif kind == "s":
            dec, tf, tg = kit["snare"]
            n = round(dec * 4 * R)
            if kit.get("clap"):
                d = []
                for i in range(n):
                    burst = sum(math.exp(-(i - o) / (0.006 * R)) for o in (0, 0.01 * R, 0.02 * R) if i >= o)
                    d.append(rng.uniform(-1, 1) * (0.5 * burst + math.exp(-i / (dec * R))) * 0.5)
            else:
                d = [(rng.uniform(-1, 1) + tg * math.sin(2 * math.pi * tf * i / R)) * math.exp(-i / (dec * R))
                     * 0.45 for i in range(n)]
            if kit["lp"] < 1.0:
                d = _lowpass(d, kit["lp"])
        elif kind in ("h", "o", "r"):
            dec = kit["hat"] * (6 if kind == "o" else 1.0) * (0.7 if kind == "r" else 1.0)
            n = round(dec * 6 * R)
            prev, d = 0.0, []
            for i in range(n):
                x = rng.uniform(-1, 1)
                d.append((x - prev * kit["bright"]) * math.exp(-i / (dec * R)) * 0.3)
                prev = x
            if kit["lp"] < 1.0:
                d = _lowpass(d, 0.6)
        elif kind.startswith("t"):  # tom: t0 high … t3 low
            f = (220, 175, 140, 110)[int(kind[1])]
            n = round(0.35 * R)
            ph, d = 0.0, []
            for i in range(n):
                ph += f * (1 + 0.6 * math.exp(-i / (0.03 * R))) / R
                d.append(math.sin(2 * math.pi * ph) * math.exp(-i / (0.12 * R)) * 0.7)
        elif kind == "crash":
            n = round(1.5 * R)
            prev, d = 0.0, []
            for i in range(n):
                x = rng.uniform(-1, 1)
                d.append((x - prev) * math.exp(-i / (0.45 * R)) * 0.16)
                prev = x
        elif kind == "riser":  # noise opening up over a bar, swelling
            n = self.bar_n
            d, y = [], 0.0
            for i in range(n):
                p = i / n
                y += (0.01 + 0.5 * p ** 2) * (rng.uniform(-1, 1) - y)
                d.append(y * p ** 1.5 * 0.45)
        elif kind == "crackle":  # vinyl: faint hiss and clicks, one bar
            n = self.bar_n
            d, y = [], 0.0
            for i in range(n):
                y += 0.05 * (rng.uniform(-1, 1) - y)
                d.append(y * 0.05 + (rng.uniform(-0.6, 0.6) if rng.random() < 0.0004 else 0.0))
        else:
            raise ValueError(kind)
        if vel != 1.0:
            d = [x * vel for x in d]
        self._drums[key] = d
        return d

    # ---------------------------------------------------------- melody
    def motif(self, rng) -> list:
        """A two-bar motif: [(16th step 0–31, length, scale degree from the
        chord root)]."""
        L = self.st["lead"]
        steps = []
        for s in range(0, 32, 2):
            if rng.random() < L["density"] or s in (0, 16):
                steps.append(s)
            if rng.random() < L["p16"] and s + 1 < 32:
                steps.append(s + 1)
        steps = sorted(set(steps))
        if len(steps) < 4:
            steps = sorted(set(steps) | {0, 6, 16, 24})
        if steps[-1] < 22:
            steps.append(24)
        out, d = [], rng.choice([0, 2, 4])
        for i, s in enumerate(steps):
            nxt = steps[i + 1] if i + 1 < len(steps) else 32
            ln = max(1, round((nxt - s) * L["legato"]))
            if i:
                d = max(-3, min(8, d + rng.choice([-2, -1, -1, 1, 1, 2, 0, 3, -3])))
            if s % 8 == 0:
                d = min((0, 2, 4, 7), key=lambda c: abs(c - d))
            out.append((s, ln, d))
        s, ln, d = out[-1]
        out[-1] = (s, ln, min((0, 2, 4), key=lambda c: abs(c - d)))
        return out

    @staticmethod
    def vary(m, op, rng) -> list:
        if op == "seq":
            k = rng.choice([1, -1, 2])
            return [(s, ln, d + k) for s, ln, d in m]
        if op == "inv":
            first = m[0][2]
            return [(s, ln, first - (d - first)) for s, ln, d in m]
        if op == "rhythm":  # push some notes a 16th early (syncopation)
            out = []
            for i, (s, ln, d) in enumerate(m):
                if s % 4 == 0 and s > 0 and rng.random() < 0.5 and (not out or out[-1][0] < s - 1):
                    out.append((s - 1, ln + 1, d))
                else:
                    out.append((s, ln, d))
            return out
        if op == "orn":  # a neighbour note before long notes
            out = []
            for s, ln, d in m:
                if ln >= 4:
                    out += [(s, 1, d + 1), (s + 1, ln - 1, d)]
                else:
                    out.append((s, ln, d))
            return out
        if op == "thin":
            return [n for i, n in enumerate(m) if i % 2 == 0 or n[0] % 8 == 0]
        return m

    @staticmethod
    def answer(call) -> list:
        """The response: the call's first bar, its second bar moved to end
        on the root, the last note held."""
        first = [n for n in call if n[0] < 16]
        second = [n for n in call if n[0] >= 16]
        if not second:
            return first + [(24, 8, 0)]
        k = -second[-1][2]
        second = [(s, ln, d + k) for s, ln, d in second]
        s, ln, d = second[-1]
        second[-1] = (s, 32 - s, d)
        return first + second

    # ---------------------------------------------------------- plan
    def plan(self, seconds, segments) -> list:
        """One spec per bar (hashable: equal bars are rendered once)."""
        st, R = self.st, random.Random(stable(self.seed, "plan", self.style_name, self.mode_name))
        n_bars = max(1, math.ceil(seconds * RATE / self.bar_n))
        bar_s = self.bar_n / RATE
        segments = sorted(segments) or [(0.0, "groove")]

        def seg_of(b):
            mid = (b + 0.5) * bar_s
            i = 0
            for k, (t, _) in enumerate(segments):
                if t <= mid:
                    i = k
            return i

        owner = [seg_of(b) for b in range(n_bars)]
        groove_ids = [i for i, (_, k) in enumerate(segments) if k == "groove"]
        G = len(groove_ids)
        lift_from = round(G * 2 / 3) if G >= 5 and seconds > 90 else None
        lift = R.choice(st["lift"])
        breakdown = None
        if G >= 4:
            cands = [c for c in range(1, G - 1) if lift_from is None or c != lift_from - 1]
            breakdown = R.choice(cands) if cands else None
        motif = self.motif(R)
        plans, prev_prog = {}, None
        for c, i in enumerate(groove_ids):
            choices = [p for p in range(len(st["progs"])) if p != prev_prog]
            pi = R.choice(choices)
            prev_prog = pi
            prog = st["progs"][pi]
            if lift_from is not None and c == lift_from - 1:
                prog = st["bridge"]
            ops = [R.choice(["seq", "inv", "rhythm", "orn", "thin"]) for _ in range(min(3, (c + 1) // 2))]
            call = motif
            lr = random.Random(stable(self.seed, "lead", c))
            for op in ops:
                call = self.vary(call, op, lr)
            plans[i] = {
                "prog": prog, "drums": R.choice(st["grooves"]), "bass": R.choice(st["bass"]["patterns"]),
                "harm": R.choice(st["harm"]["patterns"]), "lead": c % 2 == 0 or R.random() < 0.45,
                "fill": R.choice(st["fills"]), "shift": lift if lift_from is not None and c >= lift_from else 0,
                "breakdown": c == breakdown, "call": call, "answer": self.answer(call), "chapter": c,
            }
        last_shift = plans[groove_ids[-1]]["shift"] if groove_ids else 0
        for i, (_, kind) in enumerate(segments):
            if kind == "groove":
                continue
            base = {"prog": st["progs"][0], "drums": None, "bass": st["bass"]["patterns"][0],
                    "harm": st["harm"]["patterns"][0], "lead": True, "fill": "build", "shift": 0,
                    "breakdown": False, "call": motif, "answer": self.answer(motif), "chapter": -1}
            if kind == "drive":
                base.update(drums=st["drive"], fill="build")
            elif kind == "outro":
                base.update(prog=st["cadence"], shift=last_shift, fill=None)
            plans[i] = base

        specs = []
        for b in range(n_bars):
            i = owner[b]
            kind = segments[i][1]
            p = plans[i]
            k = b - owner.index(i)  # bar number within the segment
            seg_len = owner.count(i)
            last = k == seg_len - 1 and i + 1 < len(segments) and b + 1 < n_bars
            first = k == 0 and i > 0
            prog = p["prog"]
            degree = prog[min(k, len(prog) - 1)] if kind == "outro" else prog[k % len(prog)]
            chord = self.chord(degree, p["shift"])
            phrase = k % 4
            drums, fill, riser, cut, crash = p["drums"], None, False, 16, first and kind != "outro"
            bass, harm, lead = p["bass"], p["harm"], ()
            pad_g, harm_g, bass_g, gain = 1.0, 1.0, 1.0, 1.0
            alpha = self.st["pad"]["lp"]
            if kind == "intro":
                bass, harm_g, drums = None, 0.0, None
                alpha = round(0.008 + (self.st["pad"]["lp"] - 0.008) * min(1.0, (k + 1) / max(1, seg_len)), 4)
                gain = 0.75 + 0.25 * min(1.0, k / max(1, seg_len - 1))  # swell in
                pad_g = 1.5
                if last:
                    harm_g, riser = 0.6, True
            elif kind == "outro":
                drums = "sparse" if k == 0 else None
                bass = bass if k < 2 else None
                harm_g = max(0.0, 0.7 - 0.2 * k)
                pad_g = 1.4
                gain = max(0.55, 1.0 - 0.12 * k)
            else:
                if p["breakdown"] and k < max(1, seg_len - 1):
                    drums, bass, pad_g, harm_g = ("sparse" if k else None), None, 1.6, 0.7
                if last:
                    fill = p["fill"]
                    riser = fill in ("build", "stop")
                    cut = 8 if fill == "stop" else 16
                gain = (0.94, 0.97, 1.0, 1.03)[phrase]  # a phrase swells
            if p["lead"] and kind != "outro" or (kind == "outro" and k < 3):
                if kind == "drive":
                    use = (k // 4) % 2 == 1  # the motif is stated in the second phrase
                elif kind == "intro":
                    use = k >= 1
                else:
                    use = True
                if use:
                    notes = p["call"] if phrase < 2 and kind != "outro" else p["answer"]
                    half = (phrase % 2) if kind != "outro" else min(k, 1)
                    lo = 16 * half
                    lead = tuple((s - lo, ln, self.root + p["shift"] + self.deg(degree + d)
                                  + self.st["lead"]["octave"])
                                 for s, ln, d in notes if lo <= s < lo + 16)
                    if kind == "outro" and k >= 2:
                        lead = ()
                    harm_g *= 0.75
            specs.append((chord, drums, fill, crash, riser, cut, bass, bass_g, harm, harm_g, k % 2,
                          lead, pad_g, alpha, round(gain, 3), p["shift"], kind))
        return specs

    # ---------------------------------------------------------- render
    def t_step(self, s: float) -> int:
        """Sample offset of 16th step s, swung."""
        sw = self.st["swing"] if int(s) % 2 == 1 else 0.0
        return round((s + sw) * self.step)

    def bar(self, spec) -> list:
        got = self._bars.get(spec)
        if got is not None:
            return got
        (chord, drums, fill, crash, riser, cut, bass, bass_g, harm, harm_g, rot, lead, pad_g, alpha, gain,
         _shift, kind) = spec
        st, n = self.st, self.bar_n
        cut_n = self.t_step(cut) if cut < 16 else None
        ton = [0.0] * (n + self.TAIL)

        def note(at_step, len_steps, wave, semi, g, vib=0.0):
            if cut_n is not None and at_step >= cut:
                return
            s = self.t_step(at_step)
            ln = max(200, round(len_steps * self.step))
            if cut_n is not None:
                ln = min(ln, cut_n - s)
            if ln > 0:
                _add(ton, s, self.tone(wave, semi, ln, vib), g)

        _add(ton, 0, self.pad(chord, n, alpha), st["pad"]["gain"] * pad_g)
        if bass:
            for s, ln, ti, octv in BASS[bass]:
                note(s, ln * 0.92, st["bass"]["wave"], chord[ti] - 24 + 12 * octv, st["bass"]["gain"] * bass_g)
        H = st["harm"]
        if harm_g > 0:
            if H["kind"] == "arp":
                tones = [chord[0], chord[1], chord[2], chord[0] + 12, chord[1] + 12, chord[2] + 12]
                order = ARPS[harm]
                rate = H["rate"]
                for j in range(16 // rate):
                    semi = tones[order[(j + rot * 2) % len(order)]] + H["octave"]
                    note(j * rate, rate * 0.8, H["wave"], semi, H["gain"] * harm_g)
                    if H.get("echo"):
                        note(j * rate + 3, rate * 0.8, H["wave"], semi, H["gain"] * harm_g * H["echo"])
            else:
                pat, ln = STABS[harm]
                for s, c in enumerate(pat):
                    if c == "x":
                        for semi in chord:
                            note(s, ln, H["wave"], semi + H["octave"], H["gain"] * harm_g / len(chord) * 2.2)
        for s, ln, semi in lead:
            note(s, ln, st["lead"]["wave"], semi, st["lead"]["gain"], st["lead"]["vibrato"])
        if cut_n is not None:  # a stop: everything breaks off at the half bar
            fade = 400
            for i in range(cut_n, len(ton)):
                ton[i] *= max(0.0, 1 - (i - cut_n) / fade)
        out = ton
        pat = DRUMS.get(drums) if drums else None
        if pat or fill:
            pat = pat or DRUMS["four"]
            for voice, line in pat.items():
                for s, c in enumerate(line[:16]):
                    if c == "." or (fill and s >= 8):
                        continue
                    _add(out, self.t_step(s), self.drum(voice, VEL.get(c, 1.0)), 1.0)
            if fill == "fill":
                for s, t in ((8, "t0"), (10, "t0"), (12, "t1"), (13, "t2"), (14, "t3"), (15, "t3")):
                    _add(out, self.t_step(s), self.drum(t), 1.0)
                for s in (8, 12):
                    _add(out, self.t_step(s), self.drum("k"), 1.0)
            elif fill == "build":
                for s in range(8, 16):
                    _add(out, self.t_step(s), self.drum("s", round(0.35 + 0.65 * (s - 8) / 7, 2)), 1.0)
                for s in (8, 12):
                    _add(out, self.t_step(s), self.drum("k"), 1.0)
            elif fill == "stop":
                _add(out, self.t_step(15), self.drum("s", 0.8), 1.0)
        if crash:
            _add(out, 0, self.drum("crash"), 1.0)
        if riser:
            _add(out, 0, self.drum("riser"), 1.0)
        if st.get("crackle"):
            _add(out, 0, self.drum("crackle"), 1.0)
        if gain != 1.0:
            out = [x * gain for x in out]
        self._bars[spec] = out
        return out

    def render(self, seconds, segments, duck=()):
        """The soundtrack, `seconds` long: an array of floats, loudness
        normalised (RMS about −19 dBFS, peaks at most −2.5 dBFS)."""
        n = round(seconds * RATE)
        out = [0.0] * (n + self.TAIL)
        for b, spec in enumerate(self.plan(seconds, segments)):
            s = b * self.bar_n
            if s >= n:
                break
            _add(out, s, self.bar(spec))
        out = out[:n]
        rms = math.sqrt(sum(x * x for x in out) / max(1, n))
        peak = max(1e-9, max(abs(x) for x in out))
        g = min(0.11 / max(rms, 1e-9), 0.75 / peak)
        out = array("f", (x * g for x in out))
        for a, b, gd in duck:
            for i in range(max(0, round(a * RATE)), min(n, round(b * RATE))):
                out[i] *= gd
        for i in range(min(n // 2, RATE)):  # fade in / out
            out[i] *= i / RATE
            out[n - 1 - i] *= i / RATE
        return out


def stats(samples, rate=RATE) -> dict:
    """Waveform statistics of a track (for tests and checks: nobody here
    can listen): peak and RMS in dBFS, brightness (RMS of the first
    difference over the RMS: high for hats, saws and pulses), zero-crossing
    rate, and onsets per second (rises of the 10 ms envelope)."""
    n = len(samples)
    rms = math.sqrt(sum(x * x for x in samples) / max(1, n)) or 1e-12
    peak = max((abs(x) for x in samples), default=0.0) or 1e-12
    diff = math.sqrt(sum((samples[i] - samples[i - 1]) ** 2 for i in range(1, n)) / max(1, n - 1))
    zc = sum(1 for i in range(1, n) if (samples[i - 1] < 0) != (samples[i] < 0)) / max(1, n) * rate
    w = rate // 100
    envs = [math.sqrt(sum(x * x for x in samples[i:i + w]) / w) for i in range(0, n - w, w)]
    onsets = sum(1 for i in range(1, len(envs)) if envs[i] > 1.5 * envs[i - 1] + 0.01)
    return {"peak_db": 20 * math.log10(peak), "rms_db": 20 * math.log10(rms), "brightness": diff / rms,
            "zcr": zc, "onsets_per_s": onsets / max(1e-9, n / rate)}
