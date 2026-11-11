//! Benchmark statistics and the JSON report (spec §3.1.1, step 4).

/// Summary of a series of samples (milliseconds).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Nearest-rank percentile of sorted data (`p` in 0–100).
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub fn summarize(samples: &[f64]) -> Summary {
    if samples.is_empty() {
        return Summary::default();
    }
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    Summary {
        count: s.len(),
        mean: s.iter().sum::<f64>() / s.len() as f64,
        p50: percentile(&s, 50.0),
        p95: percentile(&s, 95.0),
        p99: percentile(&s, 99.0),
        max: *s.last().expect("non-empty"),
    }
}

/// Frames slower than `ms`.
pub fn count_over(samples: &[f64], ms: f64) -> usize {
    samples.iter().filter(|&&v| v > ms).count()
}

/// Longest run of consecutive frames slower than `ms` (sustained drops).
pub fn longest_run_over(samples: &[f64], ms: f64) -> usize {
    let (mut best, mut cur) = (0, 0);
    for &v in samples {
        cur = if v > ms { cur + 1 } else { 0 };
        best = best.max(cur);
    }
    best
}

/// Minimal JSON object writer (no extra dependency for a spike).
#[derive(Default)]
pub struct Json {
    fields: Vec<String>,
}

fn escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect()
}

impl Json {
    pub fn str(mut self, key: &str, v: &str) -> Self {
        self.fields
            .push(format!("\"{}\": \"{}\"", escape(key), escape(v)));
        self
    }
    pub fn num(mut self, key: &str, v: f64) -> Self {
        let v = if v.is_finite() {
            format!("{v:.3}")
        } else {
            "null".into()
        };
        self.fields.push(format!("\"{}\": {v}", escape(key)));
        self
    }
    pub fn int(mut self, key: &str, v: u64) -> Self {
        self.fields.push(format!("\"{}\": {v}", escape(key)));
        self
    }
    pub fn bool(mut self, key: &str, v: bool) -> Self {
        self.fields.push(format!("\"{}\": {v}", escape(key)));
        self
    }
    pub fn obj(mut self, key: &str, v: Json) -> Self {
        self.fields
            .push(format!("\"{}\": {}", escape(key), v.finish()));
        self
    }
    pub fn summary(self, key: &str, s: Summary) -> Self {
        let o = Json::default()
            .int("count", s.count as u64)
            .num("mean", s.mean)
            .num("p50", s.p50)
            .num("p95", s.p95)
            .num("p99", s.p99)
            .num("max", s.max);
        self.obj(key, o)
    }
    pub fn finish(self) -> String {
        format!("{{{}}}", self.fields.join(", "))
    }
}

/// Resident memory of this process in MiB (Linux; `None` elsewhere).
pub fn rss_mib() -> Option<f64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096.0 / (1024.0 * 1024.0))
}

/// VRAM in use on the GPU in MiB (Linux amdgpu sysfs; system-wide, so it
/// includes other applications). `None` if not available.
pub fn vram_used_mib() -> Option<f64> {
    let mut best: Option<f64> = None;
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let p = entry.path().join("device/mem_info_vram_used");
        if let Ok(s) = std::fs::read_to_string(&p)
            && let Ok(bytes) = s.trim().parse::<f64>()
        {
            let mib = bytes / (1024.0 * 1024.0);
            best = Some(best.map_or(mib, |b: f64| b.max(mib)));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_summary() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = summarize(&v);
        assert_eq!(
            (s.count, s.p50, s.p95, s.p99, s.max),
            (100, 50.0, 95.0, 99.0, 100.0)
        );
        assert!((s.mean - 50.5).abs() < 1e-9);
        assert_eq!(summarize(&[]), Summary::default());
        assert_eq!(percentile(&[7.0], 99.0), 7.0);
        // Unsorted input.
        assert_eq!(summarize(&[5.0, 1.0, 3.0]).p50, 3.0);
    }

    #[test]
    fn hitch_counting() {
        let v = [10.0, 30.0, 40.0, 12.0, 26.0, 27.0, 28.0, 5.0];
        assert_eq!(count_over(&v, 25.0), 5);
        assert_eq!(count_over(&v, 33.4), 1);
        assert_eq!(longest_run_over(&v, 25.0), 3);
        assert_eq!(longest_run_over(&[], 25.0), 0);
    }

    #[test]
    fn json_output() {
        let j = Json::default()
            .str("gpu", "AMD \"Vega\"")
            .num("fps", 61.23456)
            .num("bad", f64::NAN)
            .int("frames", 3)
            .bool("vsync", false)
            .summary("ft", summarize(&[1.0, 2.0]))
            .finish();
        assert_eq!(
            j,
            r#"{"gpu": "AMD \"Vega\"", "fps": 61.235, "bad": null, "frames": 3, "vsync": false, "ft": {"count": 2, "mean": 1.500, "p50": 1.000, "p95": 2.000, "p99": 2.000, "max": 2.000}}"#
        );
    }

    #[test]
    fn memory_probes_do_not_panic() {
        // Values depend on the machine; only check they are sane when present.
        if let Some(r) = rss_mib() {
            assert!(r > 0.0);
        }
        let _ = vram_used_mib();
    }
}
