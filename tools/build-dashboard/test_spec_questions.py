"""Tests of spec_questions.py (no network, no git)."""

import os
import tempfile
import time
import unittest
from pathlib import Path

import spec_questions as sq

SPEC = """# Open Cycling World

Version 2.63 – November 2026

### 3.2.1 Menu

**Open questions**

Still open: switching rides.

### Phase 4 – Avatar and Bicycle Editor

#### 4K – Milestones (proposal)

- **W5 – Backend (Phase 5):** account ledger.
- **W6 – Later:** persistent damage (if Phase 12 Q6 says so), translations.
- **W7 – Kit:** apparel prices (after Q4).
- **W8 – Touch:** touch tests (after Q5).

#### 4L – Open questions (for the user)

- **Q1 – When changes apply:** at once when leaving the workshop (in free ride; new `BikeChange` message, proposal), or keep "from the next start" (v2.7)?
- **Q4 – Apparel:** kit priced in hearts like parts (proposal; body always free), or all apparel free?
- **Q5 – Touch on the kiosk: decided (2026-11-13):** no touchscreen; synthetic events.

### Phase 9 – Sky, Time, Seasons and Weather (planning, 2026-11-12)

#### 9H – Open questions (for the user)

- **Q1 – Whose "now"? Decided (2026-11-12):** the time of the place **in the virtual world**.
- **Q2 – Shared sessions:** who controls the time machine: host / organiser only (recommended), or a personal override?
- **Q6 – Own photos: postponed (2026-11-13):** not possible for now.
- **Q8 – 3D building data (2026-11-13: research):** swissBUILDINGS3D to confirm.
- **Q10 – Priority:** order of S1–S8. Proposal: S1 → S3 → S2.

#### 9G – Architecture and milestones (proposal)

- **Milestones:**
  - **S6** – physics: wind (Q5 decides the trainer part);
  - **S1** – time & sun.

### Phase 11 – VR mode

#### 11E – Open questions (for the user)

- **Q1 – decided (v2.48):** an **Oculus Rift CV1** is the development headset.
- **Q7 – Gates:** make them solid, solid only in wild mode, or only show them (proposal: shown, not solid on the routed road)?

### Phase 12 – Collisions

#### 12J – Open questions (for the user)

- **Q6 – Persistence:** injuries reset per ride (proposal), or carry over?

## 15. Quality Goals

- **Q1 – Stray:** a question outside any phase?
"""


class ParseTest(unittest.TestCase):
    def setUp(self):
        self.qs = sq.parse_questions(SPEC)
        self.by = {q["id"]: q for q in self.qs}

    def test_finds_every_question_with_phase_and_section(self):
        self.assertEqual([q["id"] for q in self.qs], [
            "Phase 4 Q1", "Phase 4 Q4", "Phase 4 Q5", "Phase 9 Q1", "Phase 9 Q2", "Phase 9 Q6", "Phase 9 Q8",
            "Phase 9 Q10", "Phase 11 Q1", "Phase 11 Q7", "Phase 12 Q6", "§15 Q1"])
        q = self.by["Phase 4 Q1"]
        self.assertEqual((q["phase"], q["phase_title"], q["section"]), (4, "Avatar and Bicycle Editor", "4L"))
        self.assertEqual(self.by["Phase 9 Q2"]["phase_title"], "Sky, Time, Seasons and Weather")
        self.assertEqual(q["title"], "When changes apply")
        self.assertIn("BikeChange message", q["text"])
        self.assertNotIn("`", q["text"])

    def test_status_from_the_bold_label(self):
        st = {k: (v["status"], v["status_note"]) for k, v in self.by.items()}
        self.assertEqual(st["Phase 4 Q1"], ("open", ""))
        self.assertEqual(st["Phase 4 Q5"], ("decided", "decided (2026-11-13)"))
        self.assertEqual(st["Phase 9 Q1"], ("decided", "Decided (2026-11-12)"))
        self.assertEqual(self.by["Phase 9 Q1"]["title"], 'Whose "now"?')
        self.assertEqual(st["Phase 9 Q6"], ("decided", "postponed (2026-11-13)"))
        self.assertEqual(st["Phase 9 Q8"], ("open", "2026-11-13: research"))
        self.assertEqual(st["Phase 11 Q1"][0], "decided")
        # A label without a title takes it from the text.
        self.assertEqual(self.by["Phase 11 Q1"]["title"], "an Oculus Rift CV1 is the development headset")

    def test_proposals(self):
        p = {k: v["proposal"] for k, v in self.by.items()}
        self.assertEqual(p["Phase 4 Q1"], "at once when leaving the workshop (in free ride; new BikeChange message)")
        self.assertEqual(p["Phase 4 Q4"], "kit priced in hearts like parts (body always free)")
        self.assertEqual(p["Phase 9 Q2"], "host / organiser only")
        self.assertEqual(p["Phase 9 Q10"], "S1 → S3 → S2")
        self.assertEqual(p["Phase 11 Q7"], "shown, not solid on the routed road")
        self.assertEqual(p["Phase 12 Q6"], "injuries reset per ride")
        self.assertIsNone(p["Phase 9 Q8"])
        self.assertIsNone(p["Phase 4 Q5"])  # decided: no proposal

    def test_blocked_work_names_open_questions_only(self):
        b = sq.blocked_work(SPEC, self.qs)
        self.assertEqual([(x["phase"], x["milestone"], x["waits_for"]) for x in b],
                         [(4, "W6", ["Phase 12 Q6"]), (4, "W7", ["Phase 4 Q4"])])
        # W8 waits for Q5, which is decided; S6's Q5 of Phase 9 does not exist.

    def test_summary_groups_by_phase(self):
        s = sq.summary(self.qs)
        self.assertEqual((s["open"], s["decided"]), (8, 4))
        g = {x["group"]: (x["open"], x["decided"]) for x in s["groups"]}
        self.assertEqual(g["Phase 4"], (2, 1))
        self.assertEqual(g["Phase 9"], (3, 2))
        self.assertEqual([x["group"] for x in s["groups"]][:2], ["Phase 4", "Phase 9"])

    def test_plain(self):
        self.assertEqual(sq.plain("**on** and `ride.damage` [docs](http://x) *Profile*"), "on and ride.damage docs Profile")
        self.assertEqual(sq.plain("ride_input and snake_case stay"), "ride_input and snake_case stay")

    def test_real_spec_parses(self):
        spec = Path(__file__).resolve().parent.parent.parent / "SPECIFICATION.md"
        if not spec.is_file():
            self.skipTest("no SPECIFICATION.md")
        qs = sq.parse_questions(spec.read_text(encoding="utf-8"))
        self.assertGreater(len(qs), 20)
        self.assertEqual(len({q["id"] for q in qs}), len(qs), "question ids are unique")
        for q in qs:
            self.assertTrue(q["title"], q["id"])
            self.assertIn(q["status"], ("open", "decided"))


class FileTest(unittest.TestCase):
    def test_reparsed_when_the_file_changes(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "SPECIFICATION.md"
            p.write_text(SPEC, encoding="utf-8")
            s = sq.SpecQuestions(p)
            self.assertEqual(s.get()["open"], 8)
            self.assertEqual(s.get()["spec_version"], "2.63")
            p.write_text(SPEC.replace("- **Q2 – Shared sessions:**", "- **Q2 – Shared sessions: decided (2026-11-14):**"),
                         encoding="utf-8")
            t = time.time() + 5
            os.utime(p, (t, t))
            self.assertEqual(s.get()["open"], 7)

    def test_missing_file_is_an_error_not_a_crash(self):
        s = sq.SpecQuestions(Path("/nonexistent/SPECIFICATION.md"))
        self.assertIn("error", s.get())
        self.assertEqual(sq.SpecQuestions(None).get()["open"], 0)


if __name__ == "__main__":
    unittest.main()
