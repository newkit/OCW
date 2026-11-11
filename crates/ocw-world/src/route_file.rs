//! Route file (`ocw-route 1`): a ridable route with its elevation and the
//! terrain around it, written by `world-builder route`. A first, minimal
//! world-package format (spec §4.7): plain text, one route, one grid.
//!
//! ```text
//! ocw-route 1
//! name Vaduz - Malbun
//! origin 47.15 9.55          # projection centre (lat, lon) of the local metres
//! width 6.5                  # road width in metres
//! points 2
//! 0.0 455.2 0.0              # x (east) y (up) z (south), metres
//! 5.0 455.4 -0.2
//! grid -100 -100 20 3 2      # origin x z, cell size, columns, rows
//! 450 451 452                # rows of heights, z increasing
//! 453 454 455
//! ```

/// Terrain heights on a regular grid (local metres, rows along +z).
#[derive(Debug, Clone, PartialEq)]
pub struct ElevationGrid {
    pub origin: [f64; 2],
    pub cell: f64,
    pub nx: usize,
    pub nz: usize,
    pub heights: Vec<f32>,
}

impl ElevationGrid {
    pub fn max(&self) -> [f64; 2] {
        [
            self.origin[0] + self.cell * (self.nx - 1) as f64,
            self.origin[1] + self.cell * (self.nz - 1) as f64,
        ]
    }

    /// Bilinear height; outside the grid the border values continue.
    pub fn sample(&self, x: f64, z: f64) -> f64 {
        let fx = ((x - self.origin[0]) / self.cell).clamp(0.0, (self.nx - 1) as f64);
        let fz = ((z - self.origin[1]) / self.cell).clamp(0.0, (self.nz - 1) as f64);
        let (i, k) = (
            (fx as usize).min(self.nx - 2),
            (fz as usize).min(self.nz - 2),
        );
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let h = |i: usize, k: usize| f64::from(self.heights[k * self.nx + i]);
        let top = h(i, k) * (1.0 - tx) + h(i + 1, k) * tx;
        let bottom = h(i, k + 1) * (1.0 - tx) + h(i + 1, k + 1) * tx;
        top * (1.0 - tz) + bottom * tz
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteFile {
    pub name: String,
    pub origin: (f64, f64),
    pub width_m: f64,
    pub points: Vec<[f64; 3]>,
    pub grid: Option<ElevationGrid>,
}

impl RouteFile {
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "ocw-route 1\nname {}\norigin {} {}\nwidth {}\npoints {}\n",
            self.name.replace('\n', " "),
            self.origin.0,
            self.origin.1,
            self.width_m,
            self.points.len()
        );
        for p in &self.points {
            s.push_str(&format!("{:.2} {:.2} {:.2}\n", p[0], p[1], p[2]));
        }
        if let Some(g) = &self.grid {
            s.push_str(&format!(
                "grid {} {} {} {} {}\n",
                g.origin[0], g.origin[1], g.cell, g.nx, g.nz
            ));
            for row in g.heights.chunks(g.nx) {
                let line: Vec<String> = row.iter().map(|h| format!("{h:.1}")).collect();
                s.push_str(&line.join(" "));
                s.push('\n');
            }
        }
        s
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .filter(|l| !l.is_empty());
        if lines.next() != Some("ocw-route 1") {
            return Err("not an ocw-route 1 file".into());
        }
        let nums = |l: &str, n: usize| -> Result<Vec<f64>, String> {
            let v: Vec<f64> = l
                .split_whitespace()
                .map(|t| t.parse::<f64>().map_err(|e| format!("{t}: {e}")))
                .collect::<Result<_, _>>()?;
            if v.len() == n {
                Ok(v)
            } else {
                Err(format!("expected {n} numbers: {l}"))
            }
        };
        let mut f = RouteFile {
            name: String::new(),
            origin: (0.0, 0.0),
            width_m: 6.0,
            points: Vec::new(),
            grid: None,
        };
        while let Some(line) = lines.next() {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "name" => f.name = rest.to_string(),
                "origin" => {
                    let v = nums(rest, 2)?;
                    f.origin = (v[0], v[1]);
                }
                "width" => f.width_m = nums(rest, 1)?[0],
                "points" => {
                    let n = nums(rest, 1)?[0] as usize;
                    for _ in 0..n {
                        let v = nums(lines.next().ok_or("missing points")?, 3)?;
                        f.points.push([v[0], v[1], v[2]]);
                    }
                }
                "grid" => {
                    let v = nums(rest, 5)?;
                    let (nx, nz) = (v[3] as usize, v[4] as usize);
                    if nx < 2 || nz < 2 {
                        return Err("grid needs at least 2 × 2 cells".into());
                    }
                    let mut heights = Vec::with_capacity(nx * nz);
                    for _ in 0..nz {
                        let row = nums(lines.next().ok_or("missing grid rows")?, nx)?;
                        heights.extend(row.iter().map(|h| *h as f32));
                    }
                    f.grid = Some(ElevationGrid {
                        origin: [v[0], v[1]],
                        cell: v[2],
                        nx,
                        nz,
                        heights,
                    });
                }
                other => return Err(format!("unknown key: {other}")),
            }
        }
        if f.points.len() < 2 {
            return Err("a route needs at least two points".into());
        }
        Ok(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> RouteFile {
        RouteFile {
            name: "Vaduz - Malbun".into(),
            origin: (47.15, 9.55),
            width_m: 6.5,
            points: vec![[0.0, 455.2, 0.0], [5.0, 455.4, -0.2], [10.0, 456.0, -0.5]],
            grid: Some(ElevationGrid {
                origin: [-100.0, -100.0],
                cell: 20.0,
                nx: 3,
                nz: 2,
                heights: vec![450.0, 451.0, 452.0, 453.0, 454.0, 455.0],
            }),
        }
    }

    #[test]
    fn round_trip() {
        let f = file();
        let text = f.to_text();
        assert!(text.starts_with("ocw-route 1\nname Vaduz - Malbun\n"));
        assert_eq!(RouteFile::parse(&text).unwrap(), f);
    }

    #[test]
    fn grid_sampling() {
        let g = file().grid.unwrap();
        assert_eq!(g.max(), [-60.0, -80.0]);
        assert_eq!(g.sample(-100.0, -100.0), 450.0);
        assert_eq!(g.sample(-80.0, -90.0), 452.5);
        // Outside: border continues.
        assert_eq!(g.sample(-1000.0, -100.0), 450.0);
        assert_eq!(g.sample(1000.0, 1000.0), 455.0);
    }

    #[test]
    fn errors() {
        assert!(RouteFile::parse("hello").is_err());
        assert!(RouteFile::parse("ocw-route 1\npoints 1\n0 0 0\n").is_err());
        assert!(RouteFile::parse("ocw-route 1\nbogus 3\n").is_err());
        assert!(RouteFile::parse("ocw-route 1\npoints 2\n0 0 0\n1 x 0\n").is_err());
        assert!(
            RouteFile::parse("ocw-route 1\npoints 2\n0 0 0\n1 0 0\ngrid 0 0 1 1 1\n0\n").is_err()
        );
        // Comments and blank lines are fine.
        assert!(RouteFile::parse("ocw-route 1 \n# c\n\npoints 2\n0 0 0 # a\n1 0 0\n").is_ok());
    }
}
