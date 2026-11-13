//! The region a world package covers (spec §4.4 step 4, §4.7): the
//! latitude / longitude box and, optionally, the polygon of the extract
//! (a Geofabrik `.poly` file) it was cut to. Package section `REGN`.
//!
//! The builder cuts every road where it leaves the region, so two packages
//! whose regions share a border line meet there exactly: their boundary
//! vertices lie at the same points (spec §2.2.2, [`crate::border`]). At
//! runtime the region decides which package draws a piece of ground where
//! the tiles of neighbouring packages overlap ([`crate::world_set`]).
//!
//! Section layout (little-endian):
//!
//! ```text
//! layout version: u32 (1)
//! south, west, north, east: f64 × 4 (degrees)
//! outer ring count: u32, then per ring: point count u32, (lat, lon) f64 pairs
//! hole ring count: u32, then the same
//! ```

use crate::package::Bytes;

/// Section tag of the region.
pub const REGION_TAG: &[u8; 4] = b"REGN";
const REGION_VERSION: u32 = 1;

/// A latitude / longitude box, optionally narrowed to polygons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackageRegion {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
    /// Outer rings as (lat, lon); empty = the box only.
    pub outer: Vec<Vec<(f64, f64)>>,
    /// Holes as (lat, lon).
    pub holes: Vec<Vec<(f64, f64)>>,
}

/// Even-odd point-in-ring test (ray casting along longitude). Works for
/// (lat, lon) rings and, with the same arguments, for metric (z, x) rings.
pub fn in_ring(ring: &[(f64, f64)], (lat, lon): (f64, f64)) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + n - 1) % n]);
        if (a.0 > lat) != (b.0 > lat) && lon < (b.1 - a.1) * (lat - a.0) / (b.0 - a.0) + a.1 {
            inside = !inside;
        }
    }
    inside
}

impl PackageRegion {
    /// A plain box.
    pub fn bbox(south: f64, west: f64, north: f64, east: f64) -> Self {
        Self {
            south,
            west,
            north,
            east,
            ..Self::default()
        }
    }

    /// Inside the box and (if there are polygons) inside an outer ring and
    /// no hole. The same rule the builder cuts roads with.
    pub fn contains(&self, (lat, lon): (f64, f64)) -> bool {
        (self.south..=self.north).contains(&lat)
            && (self.west..=self.east).contains(&lon)
            && (self.outer.is_empty()
                || (self.outer.iter().any(|r| in_ring(r, (lat, lon)))
                    && !self.holes.iter().any(|r| in_ring(r, (lat, lon)))))
    }

    pub fn centre(&self) -> (f64, f64) {
        (
            (self.south + self.north) / 2.0,
            (self.west + self.east) / 2.0,
        )
    }

    /// The boxes overlap or touch (within `tolerance_deg`).
    pub fn boxes_touch(&self, other: &PackageRegion, tolerance_deg: f64) -> bool {
        self.south <= other.north + tolerance_deg
            && other.south <= self.north + tolerance_deg
            && self.west <= other.east + tolerance_deg
            && other.west <= self.east + tolerance_deg
    }

    /// The area of the region (the polygons, else the box), km², on a
    /// sphere of the earth's mean radius (equirectangular per ring, fine
    /// for regions of a few hundred km).
    pub fn area_km2(&self) -> f64 {
        const R_KM: f64 = 6371.0;
        let ring_km2 = |ring: &[(f64, f64)]| {
            if ring.len() < 3 {
                return 0.0;
            }
            let lat0 = (ring.iter().map(|p| p.0).sum::<f64>() / ring.len() as f64).to_radians();
            let xy = |(lat, lon): (f64, f64)| {
                (
                    lon.to_radians() * lat0.cos() * R_KM,
                    lat.to_radians() * R_KM,
                )
            };
            let n = ring.len();
            let twice: f64 = (0..n)
                .map(|i| {
                    let (a, b) = (xy(ring[i]), xy(ring[(i + 1) % n]));
                    a.0 * b.1 - b.0 * a.1
                })
                .sum();
            twice.abs() / 2.0
        };
        if self.outer.is_empty() {
            let (s, w, n, e) = (self.south, self.west, self.north, self.east);
            return ring_km2(&[(s, w), (s, e), (n, e), (n, w)]);
        }
        let outer: f64 = self.outer.iter().map(|r| ring_km2(r)).sum();
        let holes: f64 = self.holes.iter().map(|r| ring_km2(r)).sum();
        (outer - holes).max(0.0)
    }

    /// How much of the smaller of the two regions lies inside both (0 – 1),
    /// sampled on a 48 × 48 grid over the boxes' intersection. Neighbours
    /// that share a border give about 0, two cuts of the same area about 1.
    pub fn shared_fraction(&self, other: &PackageRegion) -> f64 {
        let (s, n) = (self.south.max(other.south), self.north.min(other.north));
        let (w, e) = (self.west.max(other.west), self.east.min(other.east));
        if s >= n || w >= e {
            return 0.0;
        }
        const N: usize = 48;
        let cell_km2 = PackageRegion::bbox(s, w, n, e).area_km2() / (N * N) as f64;
        let mut both = 0usize;
        for i in 0..N {
            for j in 0..N {
                let p = (
                    s + (n - s) * (i as f64 + 0.5) / N as f64,
                    w + (e - w) * (j as f64 + 0.5) / N as f64,
                );
                if self.contains(p) && other.contains(p) {
                    both += 1;
                }
            }
        }
        let smaller = self.area_km2().min(other.area_km2());
        if smaller <= 0.0 {
            return 0.0;
        }
        (both as f64 * cell_km2 / smaller).clamp(0.0, 1.0)
    }

    /// The box as [south, west, north, east].
    pub fn bounds(&self) -> [f64; 4] {
        [self.south, self.west, self.north, self.east]
    }
}

pub fn write_region(r: &PackageRegion, b: &mut Vec<u8>) {
    b.extend(REGION_VERSION.to_le_bytes());
    for v in [r.south, r.west, r.north, r.east] {
        b.extend(v.to_le_bytes());
    }
    for rings in [&r.outer, &r.holes] {
        b.extend((rings.len() as u32).to_le_bytes());
        for ring in rings {
            b.extend((ring.len() as u32).to_le_bytes());
            for &(lat, lon) in ring {
                b.extend(lat.to_le_bytes());
                b.extend(lon.to_le_bytes());
            }
        }
    }
}

pub fn read_region(bytes: &[u8]) -> Result<PackageRegion, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u32()?;
    if version != REGION_VERSION {
        return Err(format!("region section layout {version} is not supported"));
    }
    let mut region = PackageRegion::bbox(r.f64()?, r.f64()?, r.f64()?, r.f64()?);
    for holes in [false, true] {
        let count = r.u32()?;
        for _ in 0..count {
            let n = r.u32()? as usize;
            if n > bytes.len() / 16 {
                return Err("region section: ring too long".into());
            }
            let ring = (0..n)
                .map(|_| Ok((r.f64()?, r.f64()?)))
                .collect::<Result<Vec<_>, String>>()?;
            if holes {
                region.holes.push(ring);
            } else {
                region.outer.push(ring);
            }
        }
    }
    r.finish("region section")?;
    Ok(region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_and_shared_parts_of_regions() {
        // About 0.3° x 0.3° at 47° N: 33.4 km x 22.8 km.
        let a = PackageRegion::bbox(47.0, 9.4, 47.3, 9.7);
        let km2 = a.area_km2();
        assert!((km2 - 33.36 * 22.75).abs() < 15.0, "{km2}");
        // A neighbour sharing the eastern border: nothing shared.
        let east = PackageRegion::bbox(47.0, 9.7, 47.3, 10.0);
        assert!(a.shared_fraction(&east) < 0.01);
        assert!(a.boxes_touch(&east, 0.01));
        // Another cut of the same area: all of it.
        let same = PackageRegion::bbox(47.05, 9.45, 47.25, 9.65);
        assert!(
            a.shared_fraction(&same) > 0.95,
            "{}",
            a.shared_fraction(&same)
        );
        // Polygons: a triangle is half its box; holes are taken out.
        let mut t = a.clone();
        t.outer = vec![vec![(47.0, 9.4), (47.0, 9.7), (47.3, 9.7)]];
        assert!((t.area_km2() / km2 - 0.5).abs() < 0.01);
        t.holes = vec![vec![(47.0, 9.65), (47.0, 9.7), (47.05, 9.7)]];
        assert!(t.area_km2() < km2 / 2.0);
        // Far apart: nothing.
        let far = PackageRegion::bbox(48.0, 7.8, 48.2, 8.2);
        assert_eq!(a.shared_fraction(&far), 0.0);
    }

    fn diamond() -> PackageRegion {
        let mut r = PackageRegion::bbox(47.0, 9.0, 48.0, 10.0);
        r.outer
            .push(vec![(47.0, 9.5), (47.5, 10.0), (48.0, 9.5), (47.5, 9.0)]);
        r.holes.push(vec![
            (47.45, 9.45),
            (47.45, 9.55),
            (47.55, 9.55),
            (47.55, 9.45),
        ]);
        r
    }

    #[test]
    fn contains_box_polygon_and_holes() {
        let b = PackageRegion::bbox(47.0, 9.0, 48.0, 10.0);
        assert!(b.contains((47.1, 9.1)));
        assert!(!b.contains((46.9, 9.1)));
        let d = diamond();
        assert!(!d.contains((47.1, 9.1)), "in the box, outside the diamond");
        assert!(d.contains((47.3, 9.5)));
        assert!(!d.contains((47.5, 9.5)), "in the hole");
        assert_eq!(d.centre(), (47.5, 9.5));
    }

    #[test]
    fn boxes_touch_along_a_shared_border() {
        let south = PackageRegion::bbox(47.0, 9.0, 47.5, 10.0);
        let north = PackageRegion::bbox(47.5, 9.0, 48.0, 10.0);
        let far = PackageRegion::bbox(49.0, 9.0, 50.0, 10.0);
        assert!(south.boxes_touch(&north, 0.0));
        assert!(north.boxes_touch(&south, 0.0));
        assert!(!south.boxes_touch(&far, 0.01));
    }

    #[test]
    fn region_section_round_trip() {
        let d = diamond();
        let mut b = Vec::new();
        write_region(&d, &mut b);
        assert_eq!(read_region(&b).unwrap(), d);
        assert!(read_region(&b[..b.len() - 1]).is_err());
        let mut longer = b.clone();
        longer.push(0);
        assert!(read_region(&longer).is_err());
        let mut v2 = b.clone();
        v2[0] = 2;
        assert!(read_region(&v2).is_err());
    }
}
