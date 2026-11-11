//! Our own workout format (`.toml`, spec §2.8). See README "Workouts":
//!
//! ```toml
//! name = "Sweet spot 3 × 10"
//! description = "Three 10-minute blocks at 90 % FTP"
//! units = "ftp"            # bare numbers in power fields: % FTP (default) or "watts"
//!
//! [[steps]]
//! type = "ramp"            # steady | ramp | intervals | free
//! duration = "10:00"       # seconds, "M:SS", "H:MM:SS", "90s", "5min"
//! from = 50
//! to = 75
//! text = "Easy spin"
//!
//! [[steps]]
//! type = "intervals"
//! repeat = 3
//! on = { duration = "10:00", power = 90, cadence = 90 }
//! off = { duration = "5:00", power = "150W" }   # "…W" or "…%" overrides `units`
//!
//! [[steps]]
//! type = "free"
//! duration = "5:00"
//!
//! [[cues]]
//! at = "12:00"             # workout time
//! text = "Halfway through the first block"
//! duration = 10            # optional, seconds (default 10)
//! ```

use serde::Deserialize;
use toml::Value;

use super::{CUE_DURATION_S, Cue, Power, Segment, Target, Workout, WorkoutError, err};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDef {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    units: Option<String>,
    #[serde(default)]
    steps: Vec<StepDef>,
    #[serde(default)]
    cues: Vec<CueDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepDef {
    #[serde(rename = "type")]
    kind: String,
    duration: Option<Value>,
    power: Option<Value>,
    from: Option<Value>,
    to: Option<Value>,
    cadence: Option<f64>,
    text: Option<String>,
    repeat: Option<i64>,
    on: Option<PartDef>,
    off: Option<PartDef>,
}

/// The on or off part of an interval.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartDef {
    duration: Value,
    power: Option<Value>,
    from: Option<Value>,
    to: Option<Value>,
    cadence: Option<f64>,
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CueDef {
    at: Value,
    text: String,
    duration: Option<Value>,
}

/// How bare numbers in power fields are read.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Units {
    Ftp,
    Watts,
}

/// Parses our TOML workout format.
pub fn parse_toml(text: &str) -> Result<Workout, WorkoutError> {
    let def: FileDef = toml::from_str(text).map_err(|e| WorkoutError(e.to_string()))?;
    let units = match def.units.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("ftp" | "percent" | "%") => Units::Ftp,
        Some("watts" | "w") => Units::Watts,
        Some(other) => return err(format!("units = {other:?}: expected \"ftp\" or \"watts\"")),
    };
    let mut segments = Vec::new();
    for (i, step) in def.steps.iter().enumerate() {
        let at = |e: String| WorkoutError(format!("step {} ({}): {e}", i + 1, step.kind));
        segments.extend(step_segments(step, units).map_err(at)?);
    }
    let mut cues = Vec::new();
    for (i, c) in def.cues.iter().enumerate() {
        let at = |e: String| WorkoutError(format!("cue {}: {e}", i + 1));
        cues.push(Cue {
            at_s: duration(&c.at).map_err(at)?,
            duration_s: match &c.duration {
                Some(d) => duration(d).map_err(at)?,
                None => CUE_DURATION_S,
            },
            text: c.text.clone(),
        });
    }
    Workout::new(def.name, def.description, segments, cues)
}

fn step_segments(step: &StepDef, units: Units) -> Result<Vec<Segment>, String> {
    let kind = step.kind.to_ascii_lowercase();
    let only_intervals = [
        ("repeat", step.repeat.is_some()),
        ("on", step.on.is_some()),
        ("off", step.off.is_some()),
    ];
    if kind != "intervals"
        && let Some((name, _)) = only_intervals.iter().find(|(_, set)| *set)
    {
        return Err(format!("`{name}` belongs to type = \"intervals\""));
    }
    let need_duration = || {
        step.duration
            .as_ref()
            .ok_or_else(|| "missing `duration`".to_string())
            .and_then(duration)
    };
    let segment = match kind.as_str() {
        "steady" | "ramp" | "warmup" | "cooldown" => {
            let target = power_target(&step.power, &step.from, &step.to, units)?;
            if kind == "ramp" && step.power.is_some() {
                return Err("a ramp needs `from` and `to`, not `power`".into());
            }
            Segment {
                cadence_rpm: cadence(step.cadence)?,
                text: step.text.clone(),
                ..Segment::new(need_duration()?, target)
            }
        }
        "free" => {
            if step.power.is_some() || step.from.is_some() || step.to.is_some() {
                return Err("a free ride step has no power target".into());
            }
            Segment {
                cadence_rpm: cadence(step.cadence)?,
                text: step.text.clone(),
                ..Segment::new(need_duration()?, Target::Free)
            }
        }
        "intervals" => return intervals(step, units),
        other => {
            return Err(format!(
                "unknown type {other:?} (expected steady, ramp, intervals or free)"
            ));
        }
    };
    Ok(vec![segment])
}

fn intervals(step: &StepDef, units: Units) -> Result<Vec<Segment>, String> {
    if step.duration.is_some() || step.power.is_some() || step.from.is_some() || step.to.is_some() {
        return Err(
            "intervals take `repeat`, `on` and `off` (each with its own duration and power)".into(),
        );
    }
    let n = match step.repeat {
        Some(n) if (1..=1000).contains(&n) => n as usize,
        Some(n) => return Err(format!("repeat = {n}: expected 1–1000")),
        None => return Err("missing `repeat`".into()),
    };
    let on = step.on.as_ref().ok_or("missing `on`")?;
    let part = |p: &PartDef, name: &str| -> Result<Segment, String> {
        let at = |e: String| format!("{name}: {e}");
        Ok(Segment {
            cadence_rpm: cadence(p.cadence).map_err(at)?,
            text: p.text.clone().or_else(|| step.text.clone()),
            ..Segment::new(
                duration(&p.duration).map_err(at)?,
                power_target(&p.power, &p.from, &p.to, units).map_err(at)?,
            )
        })
    };
    let on = part(on, "on")?;
    let off = step.off.as_ref().map(|p| part(p, "off")).transpose()?;
    let mut out = Vec::new();
    for k in 1..=n {
        out.push(Segment {
            label: Some(format!("{k}/{n} on")),
            ..on.clone()
        });
        if let Some(off) = &off {
            out.push(Segment {
                label: Some(format!("{k}/{n} off")),
                ..off.clone()
            });
        }
    }
    Ok(out)
}

fn power_target(
    power: &Option<Value>,
    from: &Option<Value>,
    to: &Option<Value>,
    units: Units,
) -> Result<Target, String> {
    match (power, from, to) {
        (Some(p), None, None) => Ok(Target::steady(power_value(p, units)?)),
        (None, Some(a), Some(b)) => Ok(Target::Power {
            from: power_value(a, units)?,
            to: power_value(b, units)?,
        }),
        (None, None, None) => Err("missing `power` (or `from` and `to`)".into()),
        _ => Err("give either `power` or both `from` and `to`".into()),
    }
}

/// A power value: a number (in the file's `units`) or a string with a
/// unit, "250W" / "88%".
fn power_value(v: &Value, units: Units) -> Result<Power, String> {
    let (number, units) = match v {
        Value::Integer(i) => (*i as f64, units),
        Value::Float(f) => (*f, units),
        Value::String(s) => {
            let t = s.trim();
            let (num, u) = if let Some(n) = t.strip_suffix(['W', 'w']) {
                (n, Units::Watts)
            } else if let Some(n) = t.strip_suffix('%') {
                (n, Units::Ftp)
            } else {
                (t, units)
            };
            let n = num
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("power {s:?}: expected a number, \"250W\" or \"88%\""))?;
            (n, u)
        }
        other => return Err(format!("power {other}: expected a number or a string")),
    };
    if !number.is_finite() {
        return Err(format!("power {v}: not a number"));
    }
    Ok(match units {
        Units::Ftp => Power::Ftp(number / 100.0),
        Units::Watts => Power::Watts(number),
    })
}

fn cadence(c: Option<f64>) -> Result<Option<f64>, String> {
    match c {
        Some(rpm) if !(20.0..=200.0).contains(&rpm) => {
            Err(format!("cadence {rpm}: expected 20–200 rpm"))
        }
        c => Ok(c),
    }
}

/// A duration: seconds (number), "M:SS", "H:MM:SS", "90s", "5min" / "5m",
/// "1h".
pub(super) fn duration(v: &Value) -> Result<f64, String> {
    let secs = match v {
        Value::Integer(i) => *i as f64,
        Value::Float(f) => *f,
        Value::String(s) => parse_duration(s)
            .ok_or_else(|| format!("duration {s:?}: expected seconds, \"M:SS\" or \"H:MM:SS\""))?,
        other => return Err(format!("duration {other}: expected seconds or \"M:SS\"")),
    };
    if !secs.is_finite() || secs < 0.0 {
        return Err(format!("duration {v}: must not be negative"));
    }
    Ok(secs)
}

fn parse_duration(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase();
    if t.contains(':') {
        let parts: Vec<&str> = t.split(':').collect();
        if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
            return None;
        }
        let mut total = 0.0;
        for (i, p) in parts.iter().enumerate() {
            let v: f64 = p.trim().parse().ok()?;
            // Minutes and seconds after the first field stay below 60.
            if i > 0 && !(0.0..60.0).contains(&v) {
                return None;
            }
            total = total * 60.0 + v;
        }
        return Some(total);
    }
    for (suffix, factor) in [("min", 60.0), ("h", 3600.0), ("m", 60.0), ("s", 1.0)] {
        if let Some(n) = t.strip_suffix(suffix) {
            return n.trim().parse::<f64>().ok().map(|v| v * factor);
        }
    }
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../test-data/workouts/sweet-spot.toml");

    #[test]
    fn parses_the_example_workout() {
        let w = parse_toml(EXAMPLE).unwrap();
        assert_eq!(w.name, "Sweet spot 3 × 10");
        assert!(w.description.starts_with("Three"));
        // Ramp, 3 × (on, off), free = 8 segments.
        let s = w.segments();
        assert_eq!(s.len(), 8);
        assert_eq!(
            s[0].target,
            Target::Power {
                from: Power::Ftp(0.5),
                to: Power::Ftp(0.75)
            }
        );
        assert_eq!(s[0].duration_s, 600.0);
        assert_eq!(s[0].text.as_deref(), Some("Easy spin, get the legs going"));
        assert_eq!(s[1].target, Target::steady(Power::Ftp(0.9)));
        assert_eq!(s[1].cadence_rpm, Some(90.0));
        assert_eq!(s[1].label.as_deref(), Some("1/3 on"));
        assert_eq!(s[2].target, Target::steady(Power::Watts(150.0)));
        assert_eq!(s[2].label.as_deref(), Some("1/3 off"));
        assert_eq!(s[6].label.as_deref(), Some("3/3 off"));
        assert_eq!(s[7].target, Target::Free);
        assert_eq!(w.duration_s(), 600.0 + 3.0 * 900.0 + 300.0);
        assert_eq!(w.cues().len(), 1);
        assert_eq!(w.cues()[0].at_s, 720.0);
        assert_eq!(w.cues()[0].duration_s, 10.0);
        assert_eq!(w.target_w_at(700.0, 200.0), Some(180.0));
    }

    #[test]
    fn watts_units_and_explicit_suffixes() {
        let w = parse_toml(
            r#"
            name = "w"
            units = "watts"
            [[steps]]
            type = "steady"
            duration = 60
            power = 220
            [[steps]]
            type = "steady"
            duration = "1m"
            power = "105%"
            [[steps]]
            type = "ramp"
            duration = 30.5
            from = "100 W"
            to = 300.0
            "#,
        )
        .unwrap();
        let s = w.segments();
        assert_eq!(s[0].target, Target::steady(Power::Watts(220.0)));
        assert_eq!(s[1].target, Target::steady(Power::Ftp(1.05)));
        assert_eq!(s[1].duration_s, 60.0);
        assert_eq!(s[2].duration_s, 30.5);
    }

    #[test]
    fn durations() {
        let d = |s: &str| parse_duration(s);
        assert_eq!(d("1:30"), Some(90.0));
        assert_eq!(d("1:02:03"), Some(3723.0));
        assert_eq!(d("90s"), Some(90.0));
        assert_eq!(d("5min"), Some(300.0));
        assert_eq!(d("5 m"), Some(300.0));
        assert_eq!(d("1h"), Some(3600.0));
        assert_eq!(d("45"), Some(45.0));
        assert_eq!(d("1:75"), None);
        assert_eq!(d("1::2"), None);
        assert_eq!(d("soon"), None);
        assert!(duration(&Value::Integer(-5)).is_err());
        assert!(duration(&Value::Boolean(true)).is_err());
    }

    fn error_of(body: &str) -> String {
        parse_toml(&format!("name = \"x\"\n{body}")).unwrap_err().0
    }

    #[test]
    fn errors_name_the_step_and_the_problem() {
        let e = error_of("[[steps]]\ntype = \"steady\"\nduration = 60\n");
        assert_eq!(e, "step 1 (steady): missing `power` (or `from` and `to`)");
        let e = error_of("[[steps]]\ntype = \"sprint\"\nduration = 60\n");
        assert!(e.contains("unknown type \"sprint\""), "{e}");
        let e = error_of(
            "[[steps]]\ntype = \"free\"\nduration = 60\n[[steps]]\ntype = \"steady\"\nduration = \"1:7x\"\npower = 80\n",
        );
        assert!(e.starts_with("step 2 (steady): duration \"1:7x\""), "{e}");
        let e = error_of("[[steps]]\ntype = \"intervals\"\nrepeat = 3\n");
        assert!(e.contains("missing `on`"), "{e}");
        let e = error_of(
            "[[steps]]\ntype = \"intervals\"\nrepeat = 0\non = { duration = 30, power = 120 }\n",
        );
        assert!(e.contains("repeat = 0"), "{e}");
        let e = error_of(
            "[[steps]]\ntype = \"intervals\"\nrepeat = 2\non = { duration = 30, power = \"lots\" }\n",
        );
        assert!(e.contains("on: power \"lots\""), "{e}");
        let e = error_of("[[steps]]\ntype = \"steady\"\nduration = 60\npower = 80\nrepeat = 2\n");
        assert!(e.contains("`repeat` belongs to"), "{e}");
        let e = error_of("[[steps]]\ntype = \"ramp\"\nduration = 60\npower = 80\n");
        assert!(e.contains("`from` and `to`"), "{e}");
        let e = error_of("[[steps]]\ntype = \"free\"\nduration = 60\npower = 80\n");
        assert!(e.contains("no power target"), "{e}");
        let e = error_of("[[steps]]\ntype = \"steady\"\nduration = 60\npower = 80\ncadence = 5\n");
        assert!(e.contains("cadence 5"), "{e}");
        // Typos are reported by TOML with the line.
        let e = error_of("[[steps]]\ntype = \"steady\"\ndurration = 60\npower = 80\n");
        assert!(e.contains("unknown field `durration`"), "{e}");
        assert!(e.contains("line 4") || e.contains(":4:"), "{e}");
        assert!(error_of("units = \"kcal\"").contains("units"));
        assert_eq!(error_of(""), "the workout has no steps");
        assert!(
            parse_toml("description = \"no name\"")
                .unwrap_err()
                .0
                .contains("name")
        );
    }
}
