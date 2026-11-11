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
