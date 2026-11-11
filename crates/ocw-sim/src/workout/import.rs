//! Import of the plain-text `.erg` (minutes + watts) and `.mrc` (minutes +
//! % FTP) workout files:
//!
//! ```text
//! [COURSE HEADER]
//! VERSION = 2
//! UNITS = ENGLISH
//! DESCRIPTION = 2x5 min at 250 W
//! MINUTES WATTS            (or MINUTES PERCENT)
//! [END COURSE HEADER]
//! [COURSE DATA]
//! 0.00    100              minutes, then watts or % FTP
//! 5.00    180              linear between points: a ramp
//! 5.00    250              same minute twice: a step
//! 10.00   250
//! [END COURSE DATA]
//! [COURSE TEXT]            optional
//! 300     Hold it  15      seconds, text, seconds shown (tab-separated)
//! [END COURSE TEXT]
//! ```
//!
//! Lenient: header keys and section names in any case, CRLF, blank lines,
//! `;` / `#` comments, extra columns and unknown header lines are ignored;
//! a missing `MINUTES …` line falls back to the file type. Errors name the
//! line.

use super::{CUE_DURATION_S, Cue, Power, Segment, Target, Workout, WorkoutError, err};

/// What the second column of `[COURSE DATA]` holds, when the header does
/// not say (by the file type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportUnits {
    /// `.erg`: absolute watts.
    Watts,
    /// `.mrc`: percent of FTP.
    Percent,
}

#[derive(PartialEq)]
enum Section {
    None,
    Header,
    Data,
    Text,
    Other,
}

/// Parses an `.erg` / `.mrc` file. `default_units` applies without a
/// `MINUTES WATTS` / `MINUTES PERCENT` header line; `fallback_name` names a
/// workout without `DESCRIPTION` / `FILE NAME`.
pub fn parse_erg_mrc(
    text: &str,
    default_units: ImportUnits,
    fallback_name: &str,
) -> Result<Workout, WorkoutError> {
    let mut section = Section::None;
    let mut units = None;
    let mut description = None;
    let mut file_name = None;
    let mut points: Vec<(f64, f64, usize)> = Vec::new();
    let mut cues = Vec::new();
    let mut saw_data = false;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let name = line
                .trim_matches(['[', ']'])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_uppercase();
            section = match name.as_str() {
                "COURSE HEADER" => Section::Header,
                "COURSE DATA" => {
                    saw_data = true;
                    Section::Data
                }
                "COURSE TEXT" => Section::Text,
                s if s.starts_with("END ") => Section::None,
                _ => Section::Other,
            };
            continue;
        }
        match section {
            Section::Header => {
                let upper = line.to_ascii_uppercase();
                let words: Vec<&str> = upper.split_whitespace().collect();
                if words.first() == Some(&"MINUTES") || words.first() == Some(&"MINS") {
                    units = Some(match words.get(1).copied() {
                        Some("WATTS" | "WATT") => ImportUnits::Watts,
                        Some("PERCENT" | "%") => ImportUnits::Percent,
                        _ => {
                            return err(format!(
                                "line {n}: unknown data columns {line:?} (expected MINUTES WATTS or MINUTES PERCENT)"
                            ));
                        }
                    });
                } else if matches!(
                    words.first(),
                    Some(&("DISTANCE" | "MILES" | "KM" | "KILOMETERS"))
                ) {
                    return err(format!(
                        "line {n}: {line:?}: distance-based (slope) courses are not workouts; only MINUTES WATTS / MINUTES PERCENT files can be imported"
                    ));
                } else if let Some((k, v)) = line.split_once('=') {
                    match k.trim().to_ascii_uppercase().as_str() {
                        "DESCRIPTION" => description = Some(v.trim().to_string()),
                        "FILE NAME" | "FILENAME" => file_name = Some(v.trim().to_string()),
                        _ => {}
                    }
                }
            }
            Section::Data => {
                let mut cols = line
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|c| !c.is_empty());
                let num = |c: Option<&str>, what: &str| -> Result<f64, WorkoutError> {
                    let c = c.ok_or_else(|| {
                        WorkoutError(format!("line {n}: missing {what} in {line:?}"))
                    })?;
                    c.parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| {
                            WorkoutError(format!("line {n}: {what} {c:?} is not a number"))
                        })
                };
                let minutes = num(cols.next(), "minutes")?;
                let value = num(cols.next(), "power")?;
                if minutes < 0.0 || value < 0.0 {
                    return err(format!("line {n}: negative values in {line:?}"));
                }
                if let Some(&(prev, _, pn)) = points.last()
                    && minutes < prev - 1e-9
                {
                    return err(format!(
                        "line {n}: time {minutes} min goes back before {prev} min (line {pn})"
                    ));
                }
                points.push((minutes, value, n));
            }
            Section::Text => {
                // seconds <tab> text <tab> seconds shown; spaces work too.
                let fields: Vec<&str> = if line.contains('\t') {
                    line.split('\t')
                        .map(str::trim)
                        .filter(|f| !f.is_empty())
                        .collect()
                } else {
                    let mut f: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
                    if let Some(rest) = f.pop() {
                        match rest.rsplit_once(char::is_whitespace) {
                            Some((t, d)) if d.parse::<f64>().is_ok() => f.extend([t.trim(), d]),
                            _ => f.push(rest.trim()),
                        }
                    }
                    f
                };
                let at_s = fields
                    .first()
                    .and_then(|t| t.parse::<f64>().ok())
                    .filter(|t| t.is_finite() && *t >= 0.0)
                    .ok_or_else(|| {
                        WorkoutError(format!(
                            "line {n}: course text must start with the time in seconds: {line:?}"
                        ))
                    })?;
                let Some(text) = fields.get(1) else {
                    return err(format!("line {n}: course text without text"));
                };
                let duration_s = fields
                    .get(2)
                    .and_then(|d| d.parse::<f64>().ok())
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .unwrap_or(CUE_DURATION_S);
                cues.push(Cue {
                    at_s,
                    duration_s,
                    text: text.to_string(),
                });
            }
            Section::None | Section::Other => {}
        }
    }
    if !saw_data {
        return err("no [COURSE DATA] section (not an .erg / .mrc workout?)");
    }
    if points.len() < 2 {
        return err("[COURSE DATA] needs at least two points (minutes and power)");
    }
    let units = units.unwrap_or(default_units);
    let power = |v: f64| match units {
        ImportUnits::Watts => Power::Watts(v),
        ImportUnits::Percent => Power::Ftp(v / 100.0),
    };
    let mut segments: Vec<Segment> = Vec::new();
    for w in points.windows(2) {
        let ((t0, p0, _), (t1, p1, _)) = (w[0], w[1]);
        let duration_s = (t1 - t0) * 60.0;
        if duration_s < 1e-6 {
            continue; // a step: same time, new power
        }
        let target = Target::Power {
            from: power(p0),
            to: power(p1),
        };
        // Points along a steady stretch become one segment.
        if let Some(last) = segments.last_mut()
            && last.target == target
            && p0 == p1
        {
            last.duration_s += duration_s;
            continue;
        }
        segments.push(Segment::new(duration_s, target));
    }
    if segments.is_empty() {
        return err("[COURSE DATA] covers no time (all points at the same minute)");
    }
    let name = description
        .filter(|d| !d.is_empty())
        .or_else(|| {
            file_name
                .filter(|f| !f.is_empty())
                .map(|f| f.rsplit_once('.').map_or(f.clone(), |(s, _)| s.to_string()))
        })
        .unwrap_or_else(|| fallback_name.to_string());
    Workout::new(name, "", segments, cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ERG: &str = include_str!("../../test-data/workouts/two-by-five.erg");
    const MRC: &str = include_str!("../../test-data/workouts/over-unders.mrc");

    #[test]
    fn erg_steps_and_ramps_in_watts() {
        let w = parse_erg_mrc(ERG, ImportUnits::Percent, "x").unwrap();
        assert_eq!(w.name, "2x5 min at 250 W");
        let s = w.segments();
        assert_eq!(s.len(), 5);
        assert_eq!(
            s[0].target,
            Target::Power {
                from: Power::Watts(100.0),
                to: Power::Watts(180.0)
            }
        );
        assert_eq!(s[0].duration_s, 300.0);
        assert_eq!(s[1].target, Target::steady(Power::Watts(250.0)));
        assert_eq!(s[2].duration_s, 180.0);
        assert_eq!(w.duration_s(), 23.0 * 60.0);
        // FTP does not matter for absolute targets.
        assert_eq!(w.target_w_at(150.0, 300.0), Some(140.0));
        assert_eq!(w.target_w_at(14.0 * 60.0, 300.0), Some(250.0));
        assert_eq!(w.cues().len(), 2);
        assert_eq!(w.cues()[0].text, "First block, hold it");
        assert_eq!(w.cues()[0].at_s, 300.0);
        assert_eq!(w.cues()[0].duration_s, 15.0);
    }

    #[test]
    fn mrc_percent_with_comments_and_blank_lines() {
        let w = parse_erg_mrc(MRC, ImportUnits::Watts, "x").unwrap();
        assert_eq!(w.name, "Over-unders");
        let s = w.segments();
        assert_eq!(s.len(), 5, "{s:?}");
        assert_eq!(
            s[0].target,
            Target::Power {
                from: Power::Ftp(0.5),
                to: Power::Ftp(0.75)
            }
        );
        assert_eq!(s[1].target, Target::steady(Power::Ftp(0.95)));
        assert_eq!(w.target_w_at(12.5 * 60.0, 200.0), Some(210.0));
        assert_eq!(w.duration_s(), 1200.0);
    }

    #[test]
    fn units_from_the_file_type_without_a_header_line() {
        let text = "[course data]\n0 50\n2 50\n[end course data]\n";
        let w = parse_erg_mrc(text, ImportUnits::Percent, "plain").unwrap();
        assert_eq!(w.name, "plain");
        assert_eq!(w.segments()[0].target, Target::steady(Power::Ftp(0.5)));
        let w = parse_erg_mrc(text, ImportUnits::Watts, "plain").unwrap();
        assert_eq!(w.segments()[0].target, Target::steady(Power::Watts(50.0)));
        // Repeated points on a steady stretch merge.
        let w = parse_erg_mrc(
            "[COURSE DATA]\n0 100\n1 100\n2 100,extra\n",
            ImportUnits::Watts,
            "m",
        )
        .unwrap();
        assert_eq!(w.segments().len(), 1);
        assert_eq!(w.duration_s(), 120.0);
        // FILE NAME names it without a description.
        let w = parse_erg_mrc(
            "[COURSE HEADER]\nFILE NAME = intervals.erg\n[END COURSE HEADER]\n[COURSE DATA]\n0 1\n1 1\n",
            ImportUnits::Watts,
            "m",
        )
        .unwrap();
        assert_eq!(w.name, "intervals");
    }

    #[test]
    fn errors_name_the_line() {
        let e = |t: &str| parse_erg_mrc(t, ImportUnits::Watts, "x").unwrap_err().0;
        assert!(e("hello").contains("no [COURSE DATA]"));
        assert!(e("[COURSE DATA]\n0 100\n").contains("at least two points"));
        assert_eq!(
            e("[COURSE DATA]\n0 100\n5 1x0\n"),
            "line 3: power \"1x0\" is not a number"
        );
        assert_eq!(
            e("[COURSE DATA]\n0 100\n5\n"),
            "line 3: missing power in \"5\""
        );
        assert_eq!(
            e("[COURSE DATA]\n0 100\n5 100\n4 100\n"),
            "line 4: time 4 min goes back before 5 min (line 3)"
        );
        assert!(e("[COURSE DATA]\n0 -5\n1 5\n").contains("negative"));
        assert!(e("[COURSE DATA]\n1 100\n1 200\n").contains("covers no time"));
        assert!(
            e("[COURSE HEADER]\nDISTANCE GRADE WIND\n[END COURSE HEADER]\n[COURSE DATA]\n0 1\n")
                .contains("line 2")
        );
        assert!(e("[COURSE HEADER]\nMINUTES FURLONGS\n").contains("line 2: unknown data columns"));
        assert!(e("[COURSE DATA]\n0 1\n1 1\n[COURSE TEXT]\nsoon hello\n").contains("line 5"));
        assert!(e("[COURSE DATA]\n0 1\n1 1\n[COURSE TEXT]\n12\n").contains("without text"));
        // Out of range targets are caught by the model.
        assert!(e("[COURSE DATA]\n0 9000\n1 9000\n").contains("9000 W"));
    }

    #[test]
    fn course_text_separated_by_spaces() {
        let w = parse_erg_mrc(
            "[COURSE DATA]\n0 1\n1 1\n[COURSE TEXT]\n30 Spin it up 5\n40 No duration here\n",
            ImportUnits::Watts,
            "x",
        )
        .unwrap();
        assert_eq!(w.cues()[0].text, "Spin it up");
        assert_eq!(w.cues()[0].duration_s, 5.0);
        assert_eq!(w.cues()[1].text, "No duration here");
        assert_eq!(w.cues()[1].duration_s, CUE_DURATION_S);
    }
}
