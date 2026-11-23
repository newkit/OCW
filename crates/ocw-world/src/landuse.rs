//! Land use per tile (spec §4.6): a class per cell of a regular grid over
//! each 1 km tile (the splat map), plus the water surface level at the
//! terrain grid vertices in and around water. Built by `world-builder` from
//! OSM `landuse` / `natural` / `leisure` / `waterway` data and stored in the
//! package section `LUSE` (spec §4.7).
//!
//! Cell `(c, r)` of a tile covers local metres
//! `[o.x + c·s, o.x + (c+1)·s] × [o.z + r·s, o.z + (r+1)·s]` with
//! `s = tile size / cells`; classes are row-major along +z like the heights.

use std::collections::BTreeMap;

use crate::package::{Bytes, TileKey};

/// Section tag of the land use.
pub const LANDUSE_TAG: &[u8; 4] = b"LUSE";
/// Layout version inside the land-use section.
const LANDUSE_VERSION: u32 = 1;

/// Surface class of a land-use cell. The codes are stored in packages:
/// only add new classes at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum LandClass {
    /// No OSM land use: open grass (the renderer's default ground).
    #[default]
    Default = 0,
    Meadow = 1,
    Farmland = 2,
    Forest = 3,
    Orchard = 4,
    Vineyard = 5,
    Residential = 6,
    Commercial = 7,
    Industrial = 8,
    Park = 9,
    Water = 10,
    Wetland = 11,
    Rock = 12,
    Scree = 13,
    Glacier = 14,
    /// Sand, quarries, landfill, construction sites.
    Bare = 15,
    Scrub = 16,
}

impl LandClass {
    pub const ALL: [LandClass; 17] = [
        LandClass::Default,
        LandClass::Meadow,
        LandClass::Farmland,
        LandClass::Forest,
        LandClass::Orchard,
        LandClass::Vineyard,
        LandClass::Residential,
        LandClass::Commercial,
        LandClass::Industrial,
        LandClass::Park,
        LandClass::Water,
        LandClass::Wetland,
        LandClass::Rock,
        LandClass::Scree,
        LandClass::Glacier,
        LandClass::Bare,
        LandClass::Scrub,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }

    /// Where polygons overlap, the higher priority wins. Specific natural
    /// surfaces beat the large built-up areas they lie in (a park or a
    /// wood inside a residential area), water beats everything.
    pub fn priority(self) -> u8 {
        match self {
            LandClass::Default => 0,
            LandClass::Residential => 10,
            LandClass::Commercial | LandClass::Industrial => 12,
            LandClass::Farmland => 20,
            LandClass::Meadow => 22,
            LandClass::Orchard | LandClass::Vineyard => 30,
            LandClass::Scrub => 32,
            LandClass::Forest => 40,
            LandClass::Park => 45,
            LandClass::Bare => 50,
            LandClass::Scree => 55,
            LandClass::Rock => 60,
            LandClass::Wetland => 65,
            LandClass::Glacier => 80,
            LandClass::Water => 100,
        }
    }
}

/// Land use of one tile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LandUseTile {
    /// `cells × cells` classes, row-major along +z.
    pub classes: Vec<LandClass>,
    /// Water surface level (m) at terrain grid vertices (index `k·n + i`
    /// into the tile's heights), sorted by index. Covers the vertices in
    /// water and their neighbours, so every grid cell touching water has a
    /// level at all four corners.
    pub water: Vec<(u32, f32)>,
}

impl LandUseTile {
    /// Water level at terrain vertex `index`, if any.
    pub fn water_level(&self, index: u32) -> Option<f32> {
        self.water
            .binary_search_by_key(&index, |w| w.0)
            .ok()
            .map(|i| self.water[i].1)
    }
}

/// The land use of a package: one grid per tile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LandUse {
    /// Cells per tile side.
    pub cells: u32,
    pub tiles: BTreeMap<TileKey, LandUseTile>,
}

impl LandUse {
    /// Class at local (x, z), if its tile has land use.
    pub fn class_at(&self, tile_size: f64, x: f64, z: f64) -> Option<LandClass> {
        let key = TileKey::containing([x, z], tile_size);
        let t = self.tiles.get(&key)?;
        let o = key.origin(tile_size);
        let n = self.cells as usize;
        let s = tile_size / f64::from(self.cells);
        let c = (((x - o[0]) / s) as usize).min(n - 1);
        let r = (((z - o[1]) / s) as usize).min(n - 1);
        t.classes.get(r * n + c).copied()
    }

    /// Number of cells of each class over all tiles (indexed by code).
    pub fn histogram(&self) -> [usize; LandClass::ALL.len()] {
        let mut h = [0; LandClass::ALL.len()];
        for t in self.tiles.values() {
            for c in &t.classes {
                h[usize::from(c.code())] += 1;
            }
        }
        h
    }
}

/// Writes the land-use section payload:
///
/// ```text
/// layout version: u32 (1)    cells per tile side: u32
/// tile count: u32, then per tile:
///   tile x, z: i32 i32
///   run count: u32, runs: (length: u32, class: u8), row-major
///   water vertex count: u32, (vertex index: u32, level: f32)
/// ```
pub fn write_landuse(l: &LandUse, b: &mut Vec<u8>) {
    b.extend(LANDUSE_VERSION.to_le_bytes());
    b.extend(l.cells.to_le_bytes());
    b.extend((l.tiles.len() as u32).to_le_bytes());
    for (key, t) in &l.tiles {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        let mut runs: Vec<(u32, LandClass)> = Vec::new();
        for &c in &t.classes {
            match runs.last_mut() {
                Some((n, last)) if *last == c => *n += 1,
                _ => runs.push((1, c)),
            }
        }
        b.extend((runs.len() as u32).to_le_bytes());
        for (n, c) in runs {
            b.extend(n.to_le_bytes());
            b.push(c.code());
        }
        b.extend((t.water.len() as u32).to_le_bytes());
        for (i, h) in &t.water {
            b.extend(i.to_le_bytes());
            b.extend(h.to_le_bytes());
        }
    }
}

/// Reads a land-use section payload written by [`write_landuse`];
/// `samples` is the package's terrain grid size (for the water indices).
pub fn read_landuse(bytes: &[u8], samples: u32) -> Result<LandUse, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u32()?;
    if version != LANDUSE_VERSION {
        return Err(format!("land-use layout {version} is not supported"));
    }
    let cells = r.u32()?;
    if cells == 0 || cells > 4096 {
        return Err(format!("land-use grid of {cells} cells is not supported"));
    }
    let mut tiles = BTreeMap::new();
    for _ in 0..r.u32()? {
        let (key, t) = read_tile(&mut r, cells, samples)?;
        tiles.insert(key, t);
    }
    r.finish("land-use section")?;
    Ok(LandUse { cells, tiles })
}

/// One tile's land use (after the section's header).
fn read_tile(r: &mut Bytes, cells: u32, samples: u32) -> Result<(TileKey, LandUseTile), String> {
    let total = cells as usize * cells as usize;
    let vertices = samples as usize * samples as usize;
    let key = TileKey {
        x: r.i32()?,
        z: r.i32()?,
    };
    let mut classes = Vec::with_capacity(total);
    for _ in 0..r.u32()? {
        let n = r.u32()? as usize;
        let code = r.u8()?;
        let c = LandClass::from_code(code).ok_or(format!("unknown land class {code}"))?;
        if classes.len() + n > total {
            return Err("land-use runs overflow the tile".into());
        }
        classes.extend(std::iter::repeat_n(c, n));
    }
    if classes.len() != total {
        return Err("land-use runs do not fill the tile".into());
    }
    let nw = r.u32()? as usize;
    let mut water = Vec::with_capacity(nw.min(vertices));
    for _ in 0..nw {
        let (i, h) = (r.u32()?, r.f32()?);
        if i as usize >= vertices || water.last().is_some_and(|w: &(u32, f32)| w.0 >= i) {
            return Err(format!("water vertex {i} out of range or order"));
        }
        water.push((i, h));
    }
    Ok((key, LandUseTile { classes, water }))
}

/// A land-use section kept encoded, its tiles decoded one by one when
/// asked for (spec §14 Phase 10 B2, the memory per resident tile): a
/// streamed package needs the land use of the few tiles around the rider,
/// and the decoded section of Tenerife takes 169 MB against its 75 MB of
/// runs. Every tile is checked when indexed, as [`read_landuse`] does.
#[derive(Debug, Clone, PartialEq)]
pub struct LandUseIndex {
    pub cells: u32,
    samples: u32,
    bytes: Vec<u8>,
    /// Where each tile's record starts in `bytes`.
    tiles: BTreeMap<TileKey, usize>,
    /// A tile whose water has sea (level 0 m), if any.
    pub sea_tile: Option<TileKey>,
}

impl LandUseIndex {
    /// Indexes a section payload (copied) written by [`write_landuse`].
    pub fn new(bytes: &[u8], samples: u32) -> Result<Self, String> {
        Self::new_kept(bytes, samples, None)
    }

    /// [`LandUseIndex::new`] copying only the records of the tiles in
    /// `keep` (and the sea tile; `None`: all) – a phone's course, without
    /// a copy of the whole section first (Tenerife: 75 MB; v5.42).
    pub fn new_kept(
        bytes: &[u8],
        samples: u32,
        keep: Option<&std::collections::BTreeSet<TileKey>>,
    ) -> Result<Self, String> {
        let mut r = Bytes::new(bytes);
        let version = r.u32()?;
        if version != LANDUSE_VERSION {
            return Err(format!("land-use layout {version} is not supported"));
        }
        let cells = r.u32()?;
        if cells == 0 || cells > 4096 {
            return Err(format!("land-use grid of {cells} cells is not supported"));
        }
        let mut records = Vec::new();
        let mut sea_tile = None;
        for _ in 0..r.u32()? {
            let at = r.position();
            let (key, t) = read_tile(&mut r, cells, samples)?;
            let sea = sea_tile.is_none() && t.water.iter().any(|&(_, level)| level == 0.0);
            if sea {
                sea_tile = Some(key);
            }
            if sea || keep.is_none_or(|k| k.contains(&key)) {
                records.push((key, at, r.position()));
            }
        }
        r.finish("land-use section")?;
        let Some(_) = keep else {
            return Ok(Self {
                cells,
                samples,
                bytes: bytes.to_vec(),
                tiles: records.into_iter().map(|(k, at, _)| (k, at)).collect(),
                sea_tile,
            });
        };
        let mut kept = Vec::with_capacity(records.iter().map(|r| r.2 - r.1).sum());
        let mut tiles = BTreeMap::new();
        for (k, a, b) in records {
            tiles.insert(k, kept.len());
            kept.extend_from_slice(&bytes[a..b]);
        }
        Ok(Self {
            cells,
            samples,
            bytes: kept,
            tiles,
            sea_tile,
        })
    }

    /// The tile's land use, decoded.
    pub fn tile(&self, key: TileKey) -> Option<LandUseTile> {
        let at = *self.tiles.get(&key)?;
        let mut r = Bytes::new(&self.bytes[at..]);
        // Checked when indexed.
        read_tile(&mut r, self.cells, self.samples)
            .ok()
            .map(|(_, t)| t)
    }

    /// The land use of the tiles in `keys` that have any.
    pub fn land_use(&self, keys: impl IntoIterator<Item = TileKey>) -> LandUse {
        LandUse {
            cells: self.cells,
            tiles: keys
                .into_iter()
                .filter_map(|k| self.tile(k).map(|t| (k, t)))
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Keeps only the records of the tiles in `keep` (and the sea tile,
    /// for the far terrain's sea), copied into a buffer of their size (the
    /// phone's course, spec 14 Phase 10 B7, v5.42).
    pub fn retain(&mut self, keep: &std::collections::BTreeSet<TileKey>) {
        let mut starts: Vec<usize> = self.tiles.values().copied().collect();
        starts.sort_unstable();
        let end = |at: usize| {
            let i = starts.partition_point(|&s| s <= at);
            starts.get(i).copied().unwrap_or(self.bytes.len())
        };
        let kept: Vec<(TileKey, usize, usize)> = self
            .tiles
            .iter()
            .filter(|(k, _)| keep.contains(k) || self.sea_tile == Some(**k))
            .map(|(k, &at)| (*k, at, end(at)))
            .collect();
        let mut bytes = Vec::with_capacity(kept.iter().map(|k| k.2 - k.1).sum());
        let mut tiles = BTreeMap::new();
        for (k, a, b) in kept {
            tiles.insert(k, bytes.len());
            bytes.extend_from_slice(&self.bytes[a..b]);
        }
        self.bytes = bytes;
        self.tiles = tiles;
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The tiles that have land use.
    pub fn keys(&self) -> impl Iterator<Item = TileKey> + '_ {
        self.tiles.keys().copied()
    }

    /// The bytes it keeps.
    pub fn bytes(&self) -> usize {
        self.bytes.len() + self.tiles.len() * 24
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn landuse() -> LandUse {
        let mut classes = vec![LandClass::Meadow; 16];
        classes[5] = LandClass::Forest;
        classes[6] = LandClass::Forest;
        classes[15] = LandClass::Water;
        LandUse {
            cells: 4,
            tiles: BTreeMap::from([
                (
                    TileKey { x: 0, z: 0 },
                    LandUseTile {
                        classes,
                        water: vec![(7, 451.5), (8, 451.25)],
                    },
                ),
                (
                    TileKey { x: -1, z: 2 },
                    LandUseTile {
                        classes: vec![LandClass::Default; 16],
                        water: vec![],
                    },
                ),
            ]),
        }
    }

    #[test]
    fn codes_round_trip_and_priorities_order_the_classes() {
        for c in LandClass::ALL {
            assert_eq!(LandClass::from_code(c.code()), Some(c));
        }
        assert_eq!(LandClass::from_code(200), None);
        assert!(LandClass::Water.priority() > LandClass::Forest.priority());
        assert!(LandClass::Park.priority() > LandClass::Residential.priority());
        assert!(LandClass::Forest.priority() > LandClass::Farmland.priority());
        assert!(LandClass::Default.priority() < LandClass::Residential.priority());
    }

    #[test]
    fn section_round_trip_and_lookups() {
        let l = landuse();
        let mut b = Vec::new();
        write_landuse(&l, &mut b);
        // Run-length coded: the 16 cells of the default tile are one run.
        assert!(b.len() < 120, "{}", b.len());
        assert_eq!(read_landuse(&b, 3).unwrap(), l);
        let t = &l.tiles[&TileKey { x: 0, z: 0 }];
        assert_eq!(t.water_level(8), Some(451.25));
        assert_eq!(t.water_level(3), None);
        // 1000 m tile, 4 cells of 250 m: (300, 300) is cell (1, 1) = 5.
        assert_eq!(l.class_at(1000.0, 300.0, 300.0), Some(LandClass::Forest));
        assert_eq!(l.class_at(1000.0, 999.0, 999.0), Some(LandClass::Water));
        assert_eq!(l.class_at(1000.0, 5000.0, 0.0), None);
        assert_eq!(l.histogram()[LandClass::Forest.code() as usize], 2);
        // Broken payloads are rejected.
        assert!(read_landuse(&b[..b.len() - 1], 3).is_err());
        assert!(read_landuse(&b, 2).unwrap_err().contains("water vertex"));
        let mut bad = b.clone();
        bad[0] = 9;
        assert!(read_landuse(&bad, 3).is_err());
    }

    /// The memory per resident tile (spec §14 Phase 10 B2): the encoded
    /// section gives each tile as the whole decode does, only the asked
    /// ones; it is checked like the decode; it finds a tile with sea.
    #[test]
    fn the_index_decodes_tiles_when_asked_for() {
        let mut l = landuse();
        let mut b = Vec::new();
        write_landuse(&l, &mut b);
        let ix = LandUseIndex::new(&b, 3).unwrap();
        assert_eq!((ix.len(), ix.cells, ix.sea_tile), (2, 4, None));
        let k = TileKey { x: 0, z: 0 };
        assert_eq!(ix.tile(k).as_ref(), l.tiles.get(&k));
        assert_eq!(ix.tile(TileKey { x: 5, z: 5 }), None);
        assert_eq!(ix.land_use(l.tiles.keys().copied()), l);
        let one = ix.land_use([k, TileKey { x: 9, z: 9 }]);
        assert_eq!(one.tiles.keys().copied().collect::<Vec<_>>(), vec![k]);
        assert!(ix.bytes() < 200);
        // Damaged payloads fail as in read_landuse.
        assert!(LandUseIndex::new(&b[..b.len() - 1], 3).is_err());
        assert!(
            LandUseIndex::new(&b, 2)
                .unwrap_err()
                .contains("water vertex")
        );
        // The sea: water at 0 m.
        let far = TileKey { x: -1, z: 2 };
        l.tiles.get_mut(&far).unwrap().water = vec![(0, 0.0)];
        let mut b = Vec::new();
        write_landuse(&l, &mut b);
        let mut ix = LandUseIndex::new(&b, 3).unwrap();
        assert_eq!(ix.sea_tile, Some(far));
        // A phone's course (v5.42): only the kept tiles and the sea tile,
        // decoded as before, in fewer bytes.
        let before = ix.bytes();
        let k = TileKey { x: 0, z: 0 };
        let want = ix.tile(k);
        ix.retain(&[k].into());
        assert_eq!(ix.tile(k), want);
        assert!(ix.tile(far).is_some(), "the sea tile stays");
        let mut none = LandUseIndex::new(&b, 3).unwrap();
        none.retain(&[TileKey { x: 7, z: 7 }].into());
        assert_eq!(none.len(), 1, "only the sea tile");
        assert!(none.bytes() < before);
        // … the same when indexed with the kept tiles at once.
        let at_once = LandUseIndex::new_kept(&b, 3, Some(&[k].into())).unwrap();
        assert_eq!(at_once.tile(k), want);
        assert_eq!((at_once.len(), at_once.sea_tile), (2, Some(far)));
        assert_eq!(at_once.bytes(), ix.bytes());
    }
}
