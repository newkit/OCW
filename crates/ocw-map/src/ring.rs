//! The tile cache of the navigation map: a ring of `n × n` slots of
//! [`TILE_PX`]² texels in one texture. Map tile `(ix, iz)` lives in slot
//! `(ix mod n, iz mod n)`, so the texture is sampled with wrap-around
//! addressing at the texel `world / m_per_texel` and the window of tiles
//! around the rider moves with them: only the tiles that come in at the
//! leading edge are drawn, never the whole map again.

use crate::raster::TILE_PX;

/// The cache of one texture (one zoom level and look).
#[derive(Debug, Clone, PartialEq)]
pub struct Ring {
    pub n: i32,
    pub m_per_texel: f64,
    /// The map tile each slot holds, once drawn.
    slots: Vec<Option<(i32, i32)>>,
    /// The window's lowest tile (x, z); `None` before the first follow.
    window: Option<(i32, i32)>,
}

/// The slots per side for a map that reaches `reach_texels` from the
/// rider: room for the reach and a tile of lead on each side (even, 4 …
/// 16).
pub fn slots_for(reach_texels: f64) -> i32 {
    let t = f64::from(TILE_PX);
    let n = (2.0 * (reach_texels + t) / t).ceil() as i32;
    let n = n + (n & 1);
    n.clamp(4, 16)
}

impl Ring {
    pub fn new(n: i32, m_per_texel: f64) -> Self {
        let n = n.max(2);
        Self {
            n,
            m_per_texel,
            slots: vec![None; (n * n) as usize],
            window: None,
        }
    }

    /// The texture's side in texels.
    pub fn size_texels(&self) -> u32 {
        self.n as u32 * TILE_PX
    }

    /// The slot of map tile `key`.
    pub fn slot(&self, key: (i32, i32)) -> usize {
        (key.0.rem_euclid(self.n) + key.1.rem_euclid(self.n) * self.n) as usize
    }

    /// The slot's top-left texel in the texture.
    pub fn slot_origin(&self, slot: usize) -> (u32, u32) {
        let s = slot as i32;
        ((s % self.n) as u32 * TILE_PX, (s / self.n) as u32 * TILE_PX)
    }

    /// The rider's position in tiles (fractional).
    fn tiles_of(&self, p: [f64; 2]) -> [f64; 2] {
        let s = f64::from(TILE_PX) * self.m_per_texel;
        [p[0] / s, p[1] / s]
    }

    /// The map tile under home point `p`.
    pub fn tile_at(&self, p: [f64; 2]) -> (i32, i32) {
        let t = self.tiles_of(p);
        (t[0].floor() as i32, t[1].floor() as i32)
    }

    /// Centres the window on `rider`; returns the slots whose tiles left
    /// it (they are stale now: the caller clears them).
    pub fn follow(&mut self, rider: [f64; 2]) -> Vec<usize> {
        let t = self.tiles_of(rider);
        let half = f64::from(self.n) / 2.0;
        let min = (
            (t[0] - half + 0.5).floor() as i32,
            (t[1] - half + 0.5).floor() as i32,
        );
        if self.window == Some(min) {
            return Vec::new();
        }
        self.window = Some(min);
        let mut stale = Vec::new();
        for (i, s) in self.slots.iter_mut().enumerate() {
            if let Some(k) = *s
                && !(k.0 >= min.0 && k.0 < min.0 + self.n && k.1 >= min.1 && k.1 < min.1 + self.n)
            {
                *s = None;
                stale.push(i);
            }
        }
        stale
    }

    /// The window's tiles that are not drawn yet, nearest `rider` first.
    pub fn missing(&self, rider: [f64; 2]) -> Vec<(i32, i32)> {
        let Some(min) = self.window else {
            return Vec::new();
        };
        let t = self.tiles_of(rider);
        let mut out: Vec<((i32, i32), f64)> = Vec::new();
        for kz in min.1..min.1 + self.n {
            for kx in min.0..min.0 + self.n {
                if !self.holds((kx, kz)) {
                    let dx = f64::from(kx) + 0.5 - t[0];
                    let dz = f64::from(kz) + 0.5 - t[1];
                    out.push(((kx, kz), dx * dx + dz * dz));
                }
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out.into_iter().map(|(k, _)| k).collect()
    }

    /// Whether every tile within `radius_texels` of `rider` is drawn.
    pub fn complete_within(&self, rider: [f64; 2], radius_texels: f64) -> bool {
        let r = radius_texels / f64::from(TILE_PX);
        let t = self.tiles_of(rider);
        let (x0, x1) = ((t[0] - r).floor() as i32, (t[0] + r).floor() as i32);
        let (z0, z1) = ((t[1] - r).floor() as i32, (t[1] + r).floor() as i32);
        (z0..=z1).all(|z| (x0..=x1).all(|x| self.holds((x, z))))
    }

    /// Marks map tile `key` drawn into its slot.
    pub fn mark(&mut self, key: (i32, i32)) {
        let s = self.slot(key);
        self.slots[s] = Some(key);
    }

    pub fn holds(&self, key: (i32, i32)) -> bool {
        self.slots[self.slot(key)] == Some(key)
    }

    /// The tiles drawn now (for redrawing the route layer).
    pub fn drawn(&self) -> Vec<(i32, i32)> {
        self.slots.iter().flatten().copied().collect()
    }

    /// The window's tiles (drawn or not).
    pub fn window_tiles(&self) -> Vec<(i32, i32)> {
        let Some(min) = self.window else {
            return Vec::new();
        };
        let mut v = Vec::new();
        for kz in min.1..min.1 + self.n {
            for kx in min.0..min.0 + self.n {
                v.push((kx, kz));
            }
        }
        v
    }

    /// The window in texels: (min, max) corner.
    pub fn window_texels(&self) -> Option<([f64; 2], [f64; 2])> {
        let min = self.window?;
        let t = f64::from(TILE_PX);
        let lo = [f64::from(min.0) * t, f64::from(min.1) * t];
        let span = f64::from(self.n) * t;
        Some((lo, [lo[0] + span, lo[1] + span]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_follows_the_rider_and_only_new_tiles_are_missing() {
        let mut r = Ring::new(6, 1.0);
        assert_eq!(r.size_texels(), 6 * TILE_PX);
        let t = f64::from(TILE_PX);
        let rider = [10.5 * t, -3.2 * t];
        assert!(r.follow(rider).is_empty(), "nothing was drawn");
        let missing = r.missing(rider);
        assert_eq!(missing.len(), 36);
        assert_eq!(missing[0], (10, -4), "the rider's tile first");
        for k in &missing {
            r.mark(*k);
        }
        assert!(r.missing(rider).is_empty());
        assert!(r.complete_within(rider, 2.0 * t));
        // The rider rides one tile east: one column of six leaves and
        // comes in at the other side.
        let rider2 = [11.6 * t, -3.2 * t];
        let stale = r.follow(rider2);
        assert_eq!(stale.len(), 6);
        let missing = r.missing(rider2);
        assert_eq!(missing.len(), 6);
        assert!(missing.iter().all(|k| k.0 == 14), "{missing:?}");
        // The new tiles take the slots of the old column (wrap-around).
        assert!(stale.contains(&r.slot(missing[0])));
        // Slots are unique within the window.
        let mut slots: Vec<usize> = r.window_tiles().iter().map(|k| r.slot(*k)).collect();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), 36);
        let (lo, hi) = r.window_texels().unwrap();
        assert_eq!(hi[0] - lo[0], 6.0 * t);
        assert!(lo[0] <= rider2[0] && rider2[0] <= hi[0]);
        assert_eq!(r.slot_origin(r.slot((14, -3))), (2 * TILE_PX, 3 * TILE_PX));
    }

    #[test]
    fn enough_slots_for_the_reach() {
        assert_eq!(slots_for(10.0), 4);
        assert_eq!(slots_for(300.0), 8);
        assert_eq!(slots_for(1e6), 16);
        assert_eq!(slots_for(330.0) % 2, 0);
    }
}
