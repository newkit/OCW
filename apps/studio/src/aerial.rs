//! The aerial-photo layer of the studio's 2D map (spec 14 Phase 16,
//! 16H Q15 (a)): the package's orthophoto corridor (section `ORTC`, 4.7 –
//! the open orthophotos along the roads, ±50 m) drawn over the land use,
//! where the package has one. **Off by default**; View → Aerial photo, the
//! key O or `studio layer aerial on|off|toggle` switch it.
//!
//! Cheap: nothing is decoded while the layer is off. When it is on, only
//! the corridor tiles in view are decoded – in the background, nearest the
//! map's middle first – into one texture per 1 km tile at about one texel
//! per screen point ([`aerial_level`]: 64 … 2048 texels a side), the
//! corridor weight as alpha (outside the corridor the photo is not
//! stored), and kept while the memory budget allows (least recently drawn
//! go first). Above [`AERIAL_MAX_M_PER_PX`] the map says "zoom in".

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use bevy_egui::egui;
use ocw_world::ortho::OrthoCorridor;
use ocw_world::package::{TileKey, WorldPackage};

/// The coarsest zoom (metres per point) the photo is drawn at.
pub const AERIAL_MAX_M_PER_PX: f64 = 8.0;
/// Texture memory kept for the layer (bytes, RGBA8).
pub const AERIAL_BUDGET_BYTES: usize = 192 << 20;
/// Background decoders.
const WORKERS: usize = 2;

/// The texture size (texels a side) for a tile of `tile_size_m` at
/// `m_per_px`: the smallest power of two giving a texel per point, from
/// 64 up to the tile's own pixels (and egui's 2048).
pub fn aerial_level(m_per_px: f64, tile_size_m: f64, tile_px: u32) -> u32 {
    let want = (tile_size_m / m_per_px.max(1e-3)).ceil().max(1.0) as u32;
    want.next_power_of_two()
        .clamp(64, 2048)
        .min(tile_px.clamp(1, 2048))
}

/// Tile `key` of the corridor decoded to `size`² RGBA (unmultiplied, rows
/// along +z): the chunks' pixels averaged into each texel, the corridor
/// weight as alpha (times the share of the texel the stored chunks cover).
/// `None` without the tile.
pub fn tile_rgba(
    c: &OrthoCorridor,
    key: TileKey,
    tile_size: f32,
    size: u32,
) -> Option<Result<Vec<u8>, String>> {
    let tile = c.tiles.get(&key)?;
    Some((|| {
        let weights = c.weights(key).expect("tile exists")?;
        let corners = usize::from(c.weight_cells) + 1;
        let cpx = c.chunk_px(tile_size) as usize;
        let n = c.tile_px(tile_size) as usize;
        if n == 0 || corners < 2 || weights.len() < corners * corners {
            return Err("orthophoto: empty tile".into());
        }
        let size = (size as usize).clamp(1, n);
        // Source pixel → texel.
        let to = |p: usize| p * size / n;
        let mut sum = vec![0u32; size * size * 3];
        let mut count = vec![0u32; size * size];
        for (&(cx, cz), bytes) in &tile.chunks {
            let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
                .map_err(|e| format!("orthophoto chunk {cx} {cz}: {e}"))?
                .to_rgb8();
            if img.dimensions() != (cpx as u32, cpx as u32) {
                return Err(format!("orthophoto chunk {cx} {cz}: wrong size"));
            }
            let (x0, z0) = (usize::from(cx) * cpx, usize::from(cz) * cpx);
            for (i, p) in img.pixels().enumerate() {
                let (x, z) = (x0 + i % cpx, z0 + i / cpx);
                if x >= n || z >= n {
                    continue;
                }
                let t = to(z) * size + to(x);
                for k in 0..3 {
                    sum[t * 3 + k] += u32::from(p.0[k]);
                }
                count[t] += 1;
            }
        }
        // Source pixels per texel along one side.
        let span: Vec<u32> = (0..size)
            .map(|u| ((u + 1) * n).div_ceil(size) as u32 - (u * n).div_ceil(size) as u32)
            .collect();
        let per = (n - 1) as f64 / (corners - 1) as f64;
        let last = (corners - 1) as f64;
        let mut rgba = vec![0u8; size * size * 4];
        for v in 0..size {
            for u in 0..size {
                let t = v * size + u;
                if count[t] == 0 {
                    continue;
                }
                for k in 0..3 {
                    rgba[t * 4 + k] = (sum[t * 3 + k] / count[t]) as u8;
                }
                // The weight at the texel's middle (bilinear over the
                // corners, as the renderer samples it).
                let gx = (u as f64 + 0.5) * n as f64 / size as f64;
                let gz = (v as f64 + 0.5) * n as f64 / size as f64;
                let fx = (gx / per).min(last);
                let fz = (gz / per).min(last);
                let (x0, z0) = (fx as usize, fz as usize);
                let (x1, z1) = ((x0 + 1).min(corners - 1), (z0 + 1).min(corners - 1));
                let (tx, tz) = (fx - x0 as f64, fz - z0 as f64);
                let w = |c: usize, r: usize| f64::from(weights[r * corners + c]);
                let a = w(x0, z0) * (1.0 - tx) + w(x1, z0) * tx;
                let b = w(x0, z1) * (1.0 - tx) + w(x1, z1) * tx;
                let cover = f64::from(count[t]) / f64::from(span[u] * span[v]).max(1.0);
                rgba[t * 4 + 3] = ((a * (1.0 - tz) + b * tz) * cover.min(1.0)).round() as u8;
            }
        }
        Ok(rgba)
    })())
}

type Job = (TileKey, u32);
/// A decoded tile (or why not).
type Done = (Job, Result<Vec<u8>, String>);

/// The decoders' shared queue: what is still wanted, nearest first.
#[derive(Default)]
struct Queue {
    wanted: Vec<Job>,
    stop: bool,
}

struct Entry {
    texture: egui::TextureHandle,
    bytes: usize,
    used: u64,
}

/// What the layer draws this frame: textures with their package-metre
/// rectangles, and a note for the map ("zoom in", "loading").
#[derive(Debug, Clone, Default)]
pub struct AerialFrame {
    pub tiles: Vec<(egui::TextureId, [f64; 2], [f64; 2])>,
    pub note: Option<String>,
}

/// The aerial layer of one open package: its decoded tiles and decoders.
pub struct AerialLayer {
    pkg: Arc<WorldPackage>,
    textures: BTreeMap<Job, Entry>,
    in_flight: BTreeSet<Job>,
    failed: BTreeSet<Job>,
    queue: Option<Arc<(Mutex<Queue>, Condvar)>>,
    // In a mutex: a Bevy resource must be `Sync`.
    results: Option<Mutex<Receiver<Done>>>,
    frame: u64,
    /// The egui context the textures were made in: a new one (the console
    /// moved the primary context to another camera) has none of them.
    made_in: Option<egui::Context>,
}

impl AerialLayer {
    /// The layer of `pkg`, if it has an orthophoto corridor.
    pub fn of(pkg: Arc<WorldPackage>) -> Option<Self> {
        pkg.look.ortho.as_ref()?;
        Some(Self {
            pkg,
            textures: BTreeMap::new(),
            in_flight: BTreeSet::new(),
            failed: BTreeSet::new(),
            queue: None,
            results: None,
            frame: 0,
            made_in: None,
        })
    }

    fn ortho(&self) -> &OrthoCorridor {
        self.pkg.look.ortho.as_ref().expect("checked in of")
    }

    /// Tiles decoded and kept.
    #[cfg(test)]
    pub fn kept(&self) -> usize {
        self.textures.len()
    }

    fn start(&mut self) {
        if self.queue.is_some() {
            return;
        }
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let (tx, rx): (Sender<_>, Receiver<_>) = channel();
        for _ in 0..WORKERS {
            let (queue, tx, pkg) = (queue.clone(), tx.clone(), self.pkg.clone());
            std::thread::Builder::new()
                .name("studio-aerial".into())
                .spawn(move || {
                    let tile_size = pkg.tile_size_m;
                    let Some(ortho) = pkg.look.ortho.as_ref() else {
                        return;
                    };
                    loop {
                        let job = {
                            let (lock, cv) = &*queue;
                            let mut q = lock.lock().expect("aerial queue");
                            while q.wanted.is_empty() && !q.stop {
                                q = cv.wait(q).expect("aerial queue");
                            }
                            if q.stop {
                                return;
                            }
                            q.wanted.remove(0)
                        };
                        let r = tile_rgba(ortho, job.0, tile_size, job.1)
                            .unwrap_or_else(|| Err("no such tile".into()));
                        if tx.send((job, r)).is_err() {
                            return;
                        }
                    }
                })
                .expect("aerial decoder thread");
        }
        self.queue = Some(queue);
        self.results = Some(Mutex::new(rx));
    }

    /// The tiles to draw for `view` over a map of `size` points; asks the
    /// decoders for the missing ones (nearest the middle first) and takes
    /// what they finished.
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        view: &crate::map::MapView,
        size: [f32; 2],
    ) -> AerialFrame {
        self.frame += 1;
        self.unqueue();
        if self.made_in.as_ref() != Some(ctx) {
            self.textures.clear();
            self.made_in = Some(ctx.clone());
        }
        if view.m_per_px > AERIAL_MAX_M_PER_PX {
            return AerialFrame {
                tiles: Vec::new(),
                note: Some(format!(
                    "Aerial photo: zoom in (below {AERIAL_MAX_M_PER_PX:.0} m per point)"
                )),
            };
        }
        self.receive(ctx);
        let ts = f64::from(self.pkg.tile_size_m);
        let tile_px = self.ortho().tile_px(self.pkg.tile_size_m);
        let level = aerial_level(view.m_per_px, ts, tile_px);
        let (lo, hi) = view.visible(size);
        let mut visible: Vec<(TileKey, f64)> = self
            .ortho()
            .tiles
            .keys()
            .filter(|k| {
                let o = k.origin(ts);
                o[0] < hi[0] && o[0] + ts > lo[0] && o[1] < hi[1] && o[1] + ts > lo[1]
            })
            .map(|k| {
                let o = k.origin(ts);
                let d = (o[0] + ts * 0.5 - view.centre[0]).hypot(o[1] + ts * 0.5 - view.centre[1]);
                (*k, d)
            })
            .collect();
        visible.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut out = AerialFrame::default();
        let mut missing = Vec::new();
        for (k, _) in &visible {
            let job = (*k, level);
            // This level, else any other one already there meanwhile.
            let have = if self.textures.contains_key(&job) {
                Some(job)
            } else {
                self.textures.keys().find(|j| j.0 == *k).copied()
            };
            if have != Some(job) && !self.failed.contains(&job) && !self.in_flight.contains(&job) {
                missing.push(job);
            }
            if let Some(j) = have
                && let Some(e) = self.textures.get_mut(&j)
            {
                e.used = self.frame;
                let o = k.origin(ts);
                out.tiles.push((e.texture.id(), o, [o[0] + ts, o[1] + ts]));
            }
        }
        let loading = missing.len() + self.in_flight.len();
        if loading > 0 {
            out.note = Some(format!("Aerial photo: loading {loading} tiles"));
        }
        self.want(missing);
        self.evict();
        out
    }

    /// The layer is off: the decoders stop after the tiles they hold.
    pub fn idle(&mut self) {
        self.unqueue();
    }

    /// Takes back what is queued but not decoding yet: afterwards
    /// `in_flight` holds only the tiles being decoded.
    fn unqueue(&mut self) {
        let Some(q) = &self.queue else {
            return;
        };
        let (lock, _) = &**q;
        let mut q = lock.lock().expect("aerial queue");
        for j in q.wanted.drain(..) {
            self.in_flight.remove(&j);
        }
    }

    /// Queues `jobs` for the decoders (in that order).
    fn want(&mut self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        self.start();
        self.in_flight.extend(jobs.iter().copied());
        let (lock, cv) = &**self.queue.as_ref().expect("started");
        lock.lock().expect("aerial queue").wanted = jobs;
        cv.notify_all();
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.results else {
            return;
        };
        let done: Vec<_> = rx.lock().expect("aerial results").try_iter().collect();
        for (job, r) in done {
            self.in_flight.remove(&job);
            match r {
                Ok(rgba) => {
                    let s = (rgba.len() / 4).isqrt();
                    let img = egui::ColorImage::from_rgba_unmultiplied([s, s], &rgba);
                    let texture = ctx.load_texture(
                        format!("studio-aerial-{}-{}-{}", job.0.x, job.0.z, job.1),
                        img,
                        egui::TextureOptions::LINEAR,
                    );
                    // A finer or coarser level of the same tile goes.
                    self.textures.retain(|j, _| j.0 != job.0);
                    self.textures.insert(
                        job,
                        Entry {
                            texture,
                            bytes: rgba.len(),
                            used: self.frame,
                        },
                    );
                }
                Err(e) => {
                    bevy::log::warn!("aerial photo: {e}");
                    self.failed.insert(job);
                }
            }
        }
    }

    /// Drops the least recently drawn tiles above the budget (never one
    /// drawn this frame).
    fn evict(&mut self) {
        let mut total: usize = self.textures.values().map(|e| e.bytes).sum();
        if total <= AERIAL_BUDGET_BYTES {
            return;
        }
        let mut order: Vec<(u64, Job)> = self.textures.iter().map(|(j, e)| (e.used, *j)).collect();
        order.sort();
        for (used, j) in order {
            if total <= AERIAL_BUDGET_BYTES || used == self.frame {
                break;
            }
            if let Some(e) = self.textures.remove(&j) {
                total -= e.bytes;
            }
        }
    }
}

impl Drop for AerialLayer {
    fn drop(&mut self) {
        if let Some(q) = &self.queue {
            let (lock, cv) = &**q;
            if let Ok(mut q) = lock.lock() {
                q.stop = true;
                q.wanted.clear();
            }
            cv.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::ortho::{OrthoEncoding, OrthoTile, encode_weights};

    fn jpeg(rgb: [u8; 3], n: u32) -> Vec<u8> {
        let buf: Vec<u8> = (0..n * n).flat_map(|_| rgb).collect();
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
            .encode(&buf, n, n, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    /// A made-up corridor: 100 m tiles of 2 × 2 chunks of 50 px (1 m);
    /// chunk (0, 0) red, (1, 0) blue, the lower row not stored; the weight
    /// full on the left half, none on the right.
    fn corridor() -> OrthoCorridor {
        let corners = 5usize;
        let weights: Vec<u8> = (0..corners * corners)
            .map(|i| if i % corners <= 2 { 255 } else { 0 })
            .collect();
        let mut chunks = BTreeMap::new();
        chunks.insert((0u8, 0u8), jpeg([200, 30, 30], 50));
        chunks.insert((1, 0), jpeg([30, 30, 200], 50));
        OrthoCorridor {
            pixel_m: 1.0,
            half_width_m: 50.0,
            chunks: 2,
            weight_cells: 4,
            encoding: OrthoEncoding::Jpeg,
            sources: vec![0],
            tiles: [(
                TileKey { x: 0, z: 0 },
                OrthoTile {
                    weights: encode_weights(&weights, corners as u32),
                    chunks,
                },
            )]
            .into(),
        }
    }

    #[test]
    fn a_tile_decodes_with_its_corridor_weight_as_alpha() {
        let c = corridor();
        assert!(tile_rgba(&c, TileKey { x: 5, z: 5 }, 100.0, 64).is_none());
        let px = tile_rgba(&c, TileKey { x: 0, z: 0 }, 100.0, 4)
            .unwrap()
            .unwrap();
        assert_eq!(px.len(), 4 * 4 * 4);
        let at = |u: usize, v: usize| &px[(v * 4 + u) * 4..(v * 4 + u) * 4 + 4];
        // Upper left: the red chunk, in the corridor.
        assert!(at(0, 0)[0] > 180 && at(0, 0)[2] < 60, "{:?}", at(0, 0));
        assert_eq!(at(0, 0)[3], 255);
        // Upper right: the blue chunk, outside the corridor.
        assert!(at(3, 0)[2] > 180, "{:?}", at(3, 0));
        assert_eq!(at(3, 0)[3], 0);
        // The lower row has no chunks: transparent.
        assert_eq!(at(0, 3)[3], 0);
        // Asked larger than the tile's pixels: the tile's own 100 px.
        let full = tile_rgba(&c, TileKey { x: 0, z: 0 }, 100.0, 4096)
            .unwrap()
            .unwrap();
        assert_eq!(full.len(), 100 * 100 * 4);
    }

    #[test]
    fn the_level_gives_a_texel_per_point() {
        // A 1 km tile of 2000 px (0.5 m).
        assert_eq!(aerial_level(8.0, 1000.0, 2000), 128);
        assert_eq!(aerial_level(4.0, 1000.0, 2000), 256);
        assert_eq!(aerial_level(1.0, 1000.0, 2000), 1024);
        assert_eq!(aerial_level(0.2, 1000.0, 2000), 2000);
        assert_eq!(aerial_level(100.0, 1000.0, 2000), 64);
        // 1 m packages (1000 px) stop at their own pixels.
        assert_eq!(aerial_level(0.2, 1000.0, 1000), 1000);
    }

    #[test]
    fn only_a_package_with_a_corridor_has_the_layer() {
        let mut pkg = crate::map::tests::package();
        assert!(AerialLayer::of(Arc::new(pkg.clone())).is_none());
        pkg.look.ortho = Some(corridor());
        let layer = AerialLayer::of(Arc::new(pkg)).unwrap();
        assert_eq!(layer.kept(), 0, "nothing decoded before it is drawn");
        assert!(layer.queue.is_none(), "no decoder started");
    }

    /// Regression (spec 16A *Fixed*, 6.03): tiles made in one egui context
    /// are not drawn from another one (the console gives the primary
    /// context a fresh one when it moves to another camera).
    #[test]
    fn a_new_egui_context_drops_the_tiles_of_the_old_one() {
        let mut pkg = crate::map::tests::package();
        pkg.look.ortho = Some(corridor());
        let mut layer = AerialLayer::of(Arc::new(pkg)).unwrap();
        let (old, new) = (egui::Context::default(), egui::Context::default());
        let texture = old.load_texture(
            "tile",
            egui::ColorImage::filled([2, 2], egui::Color32::RED),
            egui::TextureOptions::LINEAR,
        );
        layer.made_in = Some(old.clone());
        layer.textures.insert(
            (TileKey { x: 0, z: 0 }, 64),
            Entry {
                texture,
                bytes: 16,
                used: 0,
            },
        );
        let view = crate::map::MapView::fit([0.0, 0.0], [100.0, 100.0], [200.0, 200.0]);
        let kept = layer.frame(&old, &view, [200.0, 200.0]);
        assert_eq!(kept.tiles.len(), 1, "the same context keeps its tile");
        let fresh = layer.frame(&new, &view, [200.0, 200.0]);
        assert!(fresh.tiles.is_empty(), "no tile of the old context");
        assert_eq!(layer.kept(), 0);
        assert!(layer.made_in.as_ref() == Some(&new));
    }
}
