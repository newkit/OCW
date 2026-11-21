use std::collections::BTreeMap;

use ocw_world::graph::RoadClass;
use ocw_world::landuse::{LandUse, LandUseTile};
use ocw_world::package::RoadStrip;
use ocw_world::{RoadDress, RoadLit, Tile};

use super::*;
use crate::quality::{QualityLevel, QualityPreset};

const KEY: TileKey = TileKey { x: 0, z: 0 };
const N: u32 = 101;

const TENERIFE: (f64, f64) = (28.33, -16.80);
const BLACK_FOREST: (f64, f64) = (47.99, 7.85);
const LIECHTENSTEIN: (f64, f64) = (47.14, 9.52);

/// A 1 km tile at `at` (10 m terrain grid, ground `height(i, k)` at its
/// vertices), all of `class`, with one road along x at z = 500 m on the
/// ground (and one along z at x = 300 m).
fn package(at: (f64, f64), class: LandClass, height: impl Fn(u32, u32) -> f32) -> WorldPackage {
    let heights: Vec<f32> = (0..N * N).map(|v| height(v % N, v / N)).collect();
    let mut pkg = WorldPackage {
        name: "Verge".into(),
        origin: at,
        tile_size_m: 1000.0,
        samples: N,
        tiles: BTreeMap::from([(
            KEY,
            Tile {
                key: KEY,
                heights,
                roads: Vec::new(),
            },
        )]),
        graph: None,
        landuse: None,
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
    };
    let cells = 200usize;
    pkg.landuse = Some(LandUse {
        cells: cells as u32,
        tiles: BTreeMap::from([(
            KEY,
            LandUseTile {
                classes: vec![class; cells * cells],
                water: Vec::new(),
            },
        )]),
    });
    let road = |pts: Vec<[f32; 2]>, pkg: &WorldPackage| RoadStrip {
        class: RoadClass::Tertiary,
        width_m: 6.0,
        points: pts
            .iter()
            .map(|p| {
                // (The tile's last edge belongs to the next tile.)
                let (x, z) = (p[0].min(999.9), p[1].min(999.9));
                let y = pkg.surface_height(f64::from(x), f64::from(z)).unwrap() as f32;
                [x, y, z]
            })
            .collect(),
        surface: RoadSurface::Asphalt,
        dress: RoadDress::of_class(RoadClass::Tertiary, 6.0),
        lit: RoadLit::Unknown,
        condition: Default::default(),
    };
    let a = road((0..=100).map(|i| [i as f32 * 10.0, 500.0]).collect(), &pkg);
    let b = road(
        (0..=40).map(|i| [300.0, 600.0 + i as f32 * 10.0]).collect(),
        &pkg,
    );
    pkg.tiles.get_mut(&KEY).unwrap().roads = vec![a, b];
    pkg
}

fn flat(_: u32, _: u32) -> f32 {
    300.0
}

/// The tile's terrain mesh as the renderer draws it.
fn terrain(pkg: &WorldPackage) -> MeshArrays {
    crate::package::terrain_arrays(pkg, &pkg.tiles[&KEY])
}

fn bits_with(
    pkg: &WorldPackage,
    mesh: &MeshArrays,
    solid: &SolidFootprints,
    season: Option<&SeasonDay>,
    p: &Params,
) -> Vec<Bit> {
    let ground = TileGround::new(pkg, KEY, Some(mesh));
    let keep = |_: f64, _: f64| true;
    let ctx = Ctx {
        pkg,
        key: KEY,
        ground: &ground,
        solid,
        season,
        keep: &keep,
    };
    tile_bits(&ctx, p)
}

fn bits(pkg: &WorldPackage, season: Option<&SeasonDay>, p: &Params) -> Vec<Bit> {
    bits_with(pkg, &terrain(pkg), &SolidFootprints::default(), season, p)
}

fn count(bits: &[Bit], k: Kind) -> usize {
    bits.iter().filter(|b| b.kind == k).count()
}

fn day(at: (f64, f64), d: f64) -> SeasonDay {
    ocw_sky::season::SeasonModel::for_place(at.0, at.1).day(d)
}

#[test]
fn the_same_package_gives_the_same_bits_and_chunks_add_up() {
    let pkg = package(BLACK_FOREST, LandClass::Forest, |i, k| {
        400.0 + 0.3 * i as f32 + 0.1 * k as f32
    });
    let a = bits(&pkg, None, &Params::FULL);
    let b = bits(&pkg, None, &Params::FULL);
    assert!(a.len() > 10_000, "{} bits", a.len());
    assert_eq!(a, b);
    // The chunks (built in any order, on any thread) give the same bits
    // as the whole tile.
    let mesh = terrain(&pkg);
    let detail = tile_detail(&pkg, KEY, None, &mesh, &SolidFootprints::default(), None);
    assert!(!detail.chunks.is_empty() && detail.chunks.len() < 400);
    let chunks = detail.chunks.clone();
    let pkg = Arc::new(pkg);
    let src = DetailSource {
        pkg: Arc::downgrade(&pkg),
        member: 0,
        tile: detail,
    };
    let mut joined: Vec<Bit> = chunks
        .iter()
        .rev()
        .flat_map(|c| src.bits(&pkg, c, &Params::FULL))
        .collect();
    let key = |b: &Bit| {
        (
            b.kind,
            (b.pos.x * 1e4) as i64,
            (b.pos.z * 1e4) as i64,
            (b.size * 1e6) as i64,
        )
    };
    let mut whole = a.clone();
    whole.sort_by_key(key);
    joined.sort_by_key(key);
    assert_eq!(whole.len(), joined.len());
    assert_eq!(whole, joined);
}

#[test]
fn the_bits_are_seeded_by_world_position() {
    // The same road in two packages with different heights elsewhere:
    // the bits beside it are the same.
    let a = package(LIECHTENSTEIN, LandClass::Meadow, flat);
    let mut b = package(LIECHTENSTEIN, LandClass::Meadow, flat);
    // A second road only in b, far from the first.
    let extra = b.tiles[&KEY].roads[0].clone();
    let mut extra = extra;
    for p in &mut extra.points {
        p[2] = 100.0;
    }
    b.tiles.get_mut(&KEY).unwrap().roads.push(extra);
    let near = |v: Vec<Bit>| -> Vec<Bit> { v.into_iter().filter(|b| b.pos.z > 300.0).collect() };
    assert_eq!(
        near(bits(&a, None, &Params::FULL)),
        near(bits(&b, None, &Params::FULL))
    );
}

/// The drawn surface beside the road at a bit: the ground as drawn, or
/// the road's shoulder where it lies higher (on this flat road just under
/// the ground).
fn check_on_ground(pkg: &WorldPackage, mesh: &MeshArrays, all: &[Bit]) {
    let ground = TileGround::new(pkg, KEY, Some(mesh));
    let mut checked = 0;
    for b in all {
        let (x, z) = (f64::from(b.pos.x), f64::from(b.pos.z));
        let g = ground.height(x, z).unwrap() as f32;
        let off = b.pos.y - g;
        // Within the widest shoulder of the road (2.7 m beyond its edge)
        // a bit may lie on the shoulder over the ground, never over the
        // road's lip.
        let (centre, road) = if z > 597.0 && (x - 300.0).abs() < 12.0 {
            ((x - 300.0).abs(), pkg.road_heights(300.0, z.min(999.0)))
        } else {
            ((z - 500.0).abs(), pkg.road_heights(x.min(999.0), 500.0))
        };
        // Edge gravel on the road surface lies on the road, and so do the
        // road's own bits.
        if b.road || (b.kind == Kind::Gravel && centre < 3.0 + 1e-3) {
            continue;
        }
        let top = road.iter().copied().fold(f64::MIN, f64::max) as f32 + crate::ROAD_SURFACE_LIFT_M;
        let shoulder = centre < 3.0 + 2.7 && off > 0.03;
        let g = if shoulder {
            assert!(b.pos.y <= top + 0.01, "{b:?}: over the road's edge {top}");
            b.pos.y
        } else {
            match b.kind {
                // The lowest ground under the rock.
                Kind::Rock => assert!(off <= 0.02 && off > -b.size, "{b:?}: {off}"),
                _ => assert!(off.abs() < 0.03, "{b:?}: {off} over the drawn ground"),
            }
            g
        };
        let lifted = (b.pos.y - ground.height(x, z).unwrap() as f32).max(0.0);
        // The model: never floating (its lowest point at or under the
        // ground), never buried (its top over it).
        let (mut solid, mut patches) = (MeshArrays::default(), MeshArrays::default());
        add_bit(&mut solid, &mut patches, b, Vec3::ZERO);
        let m = if b.kind.patch() { &patches } else { &solid };
        assert!(!m.positions.is_empty(), "{b:?}");
        if b.kind.patch() {
            for p in &m.positions {
                let g = ground.height(f64::from(p[0]), f64::from(p[2])).unwrap() as f32;
                let d = p[1] - g;
                assert!(
                    (-0.01..0.25 + lifted).contains(&d),
                    "{b:?}: patch point {d} over the ground"
                );
            }
        } else {
            let low = m.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            let top = m.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            let lg = m
                .positions
                .iter()
                .min_by(|p, q| p[1].total_cmp(&q[1]))
                .map(|p| ground.height(f64::from(p[0]), f64::from(p[2])).unwrap() as f32 + lifted)
                .unwrap();
            assert!(low <= lg.max(g) + 0.025, "{b:?}: floats {} m", low - lg);
            assert!(top > g, "{b:?}: buried");
        }
        checked += 1;
    }
    assert!(checked > 1_000, "{checked}");
}

#[test]
fn bits_lie_on_the_drawn_ground_on_slopes_and_terrace_edges() {
    // A slope rising 0.6 m per 10 m and terraces 1 m high on the verge
    // (twisted cells: the bilinear ground and the drawn one differ).
    let slope = package(BLACK_FOREST, LandClass::Forest, |i, k| {
        400.0 + 0.6 * i as f32 + 0.2 * k as f32
    });
    check_on_ground(&slope, &terrain(&slope), &bits(&slope, None, &Params::FULL));
    let terraces = package(TENERIFE, LandClass::Default, |i, k| {
        let near_road = (k as i32 - 50).abs() <= 1;
        300.0
            + if near_road {
                0.0
            } else {
                ((i * 7 + k * 13) % 3) as f32 * 0.5
            }
    });
    let mesh = terrain(&terraces);
    let all = bits(&terraces, None, &Params::FULL);
    check_on_ground(&terraces, &mesh, &all);
}

#[test]
fn bits_follow_the_ground_lowered_at_junctions() {
    // The renderer lowers the terrain under the junction surfaces: the
    // bits follow the lowered mesh, not the package's heights.
    let pkg = package(LIECHTENSTEIN, LandClass::Meadow, flat);
    let mut mesh = terrain(&pkg);
    let n = N as usize;
    for k in 45..=55 {
        for i in 25..=35 {
            mesh.positions[k * n + i][1] -= 0.4;
        }
    }
    let all = bits_with(
        &pkg,
        &mesh,
        &SolidFootprints::default(),
        None,
        &Params::FULL,
    );
    check_on_ground(&pkg, &mesh, &all);
    let lowered = all
        .iter()
        .filter(|b| {
            b.kind != Kind::Gravel
                && (260.0..340.0).contains(&b.pos.x)
                && (460.0..540.0).contains(&b.pos.z)
        })
        .count();
    assert!(lowered > 50, "{lowered}");
    assert!(
        all.iter()
            .filter(|b| b.kind == Kind::Tuft
                && (260.0..340.0).contains(&b.pos.x)
                && (460.0..540.0).contains(&b.pos.z)
                && (b.pos.z - 500.0).abs() > 5.8)
            .all(|b| b.pos.y < 299.9)
    );
}

#[test]
fn nothing_on_the_road_but_edge_gravel_and_its_own_bits_and_nothing_in_buildings() {
    let pkg = package(BLACK_FOREST, LandClass::Meadow, flat);
    // A house 2 m beside the road.
    let house = vec![
        [500.0, 505.0],
        [520.0, 505.0],
        [520.0, 520.0],
        [500.0, 520.0],
    ];
    let solid = SolidFootprints::new([(house.clone(), 310.0)]);
    let all = bits_with(&pkg, &terrain(&pkg), &solid, None, &Params::FULL);
    let mut edge = 0;
    for b in &all {
        let (x, z) = (f64::from(b.pos.x), f64::from(b.pos.z));
        let on = !pkg.road_heights(x, z).is_empty();
        if on && b.road {
            assert!(Kind::ON_ROAD.contains(&b.kind), "{b:?} on the road");
            continue;
        }
        assert!(!b.road || on, "{b:?}: a road bit off the road");
        if on {
            assert_eq!(b.kind, Kind::Gravel, "{b:?} on the road");
            // Within the edge band: 3 m from a centre line.
            let off = if z > 597.0 {
                (x - 300.0).abs()
            } else {
                (z - 500.0).abs()
            };
            assert!(off >= 3.0 - f64::from(EDGE_IN_M) - 1e-3, "{b:?} {off}");
            edge += 1;
        }
        assert!(solid.top_at(x, z).is_none(), "{b:?} in the house");
        // Within the band.
        let off = (z - 500.0).abs().min(if z > 590.0 {
            (x - 300.0).abs()
        } else {
            f64::MAX
        });
        assert!(off <= f64::from(3.0 + BAND_M + 0.1), "{b:?} {off}");
    }
    assert!(edge > 100, "{edge} edge gravel");
    // Beside the house the verge is empty, elsewhere not.
    let by_house = all
        .iter()
        .filter(|b| (500.0..520.0).contains(&b.pos.x) && b.pos.z > 504.0)
        .count();
    let by_meadow = all
        .iter()
        .filter(|b| (600.0..620.0).contains(&b.pos.x) && b.pos.z > 504.0)
        .count();
    assert!(
        by_house * 4 < by_meadow,
        "{by_house} by the house, {by_meadow} by the meadow"
    );
}

#[test]
fn no_bits_on_water_or_far_off_the_road() {
    let mut pkg = package(BLACK_FOREST, LandClass::Water, flat);
    assert!(
        bits(&pkg, None, &Params::FULL)
            .iter()
            .all(|b| b.kind == Kind::Gravel)
    );
    // A road in a tunnel (40 m under the ground): nothing on the hill.
    pkg = package(BLACK_FOREST, LandClass::Forest, flat);
    for r in &mut pkg.tiles.get_mut(&KEY).unwrap().roads {
        for p in &mut r.points {
            p[1] -= 40.0;
        }
    }
    let ground = TileGround::new(&pkg, KEY, None);
    let all = bits(&pkg, None, &Params::FULL);
    assert!(
        all.iter().all(|b| (b.pos.y
            - ground
                .height(f64::from(b.pos.x), f64::from(b.pos.z))
                .unwrap() as f32)
            .abs()
            > 10.0),
        "bits on the hill over a tunnel"
    );
}

#[test]
fn each_region_has_its_own_ground() {
    let tf = bits(
        &package(TENERIFE, LandClass::Default, |_, _| 600.0),
        None,
        &Params::FULL,
    );
    let bf = bits(
        &package(BLACK_FOREST, LandClass::Forest, |_, _| 700.0),
        None,
        &Params::FULL,
    );
    let li = bits(
        &package(LIECHTENSTEIN, LandClass::Meadow, |_, _| 500.0),
        None,
        &Params::FULL,
    );
    // Tenerife: volcanic rock, dust, sparse dry plants, no ferns or moss.
    assert_eq!(count(&tf, Kind::Fern) + count(&tf, Kind::Moss), 0);
    assert!(count(&tf, Kind::Rock) > count(&li, Kind::Rock) * 2);
    assert!(count(&tf, Kind::Dirt) > count(&li, Kind::Dirt) * 2);
    assert!(count(&tf, Kind::Tuft) * 2 < count(&li, Kind::Tuft));
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let mean = |v: &[Bit], k: Kind| {
        let s: Vec<f32> = v
            .iter()
            .filter(|b| b.kind == k)
            .map(|b| lum(b.colour))
            .collect();
        s.iter().sum::<f32>() / s.len().max(1) as f32
    };
    // Dark basalt against light limestone.
    assert!(mean(&tf, Kind::Pebble) * 2.0 < mean(&li, Kind::Pebble));
    // Black Forest woods: needles, cones, moss, ferns, twigs.
    for k in [
        Kind::Needles,
        Kind::Cone,
        Kind::Moss,
        Kind::Fern,
        Kind::Twig,
    ] {
        assert!(count(&bf, k) > 200, "{} {}", k.name(), count(&bf, k));
        assert!(count(&bf, k) > count(&li, k) * 2, "{}", k.name());
    }
    // Liechtenstein meadow: dense tufts, road-edge limestone gravel.
    assert!(count(&li, Kind::Tuft) > 2_500, "{}", count(&li, Kind::Tuft));
    assert!(count(&li, Kind::Gravel) > count(&bf, Kind::Gravel));
    // Alpine rocks higher up.
    let high = bits(
        &package(LIECHTENSTEIN, LandClass::Meadow, |_, _| 1600.0),
        None,
        &Params::FULL,
    );
    assert!(count(&high, Kind::Rock) > count(&li, Kind::Rock) * 2);
}

#[test]
fn autumn_brings_leaves_and_snow_covers_the_small_bits() {
    let at = BLACK_FOREST;
    // A beech wood (no forest type: give the region's share a lift).
    let mut pkg = package(at, LandClass::Forest, |_, _| 400.0);
    pkg.origin = at;
    let summer = day(at, 200.0);
    let autumn = day(at, 300.0);
    let winter = day(at, 30.0);
    let s = bits(&pkg, Some(&summer), &Params::FULL);
    let a = bits(&pkg, Some(&autumn), &Params::FULL);
    assert!(
        count(&a, Kind::Leaf) > count(&s, Kind::Leaf) * 2,
        "{} vs {}",
        count(&a, Kind::Leaf),
        count(&s, Kind::Leaf)
    );
    // Coloured leaves in autumn: more red than in summer's old litter.
    let red = |v: &[Bit]| {
        let l: Vec<f32> = v
            .iter()
            .filter(|b| b.kind == Kind::Leaf)
            .map(|b| b.colour[0] / b.colour[2].max(1e-3))
            .collect();
        l.iter().sum::<f32>() / l.len().max(1) as f32
    };
    assert!(red(&a) > red(&s), "{} {}", red(&a), red(&s));
    // Dry summer in Tenerife: more straw than in its wet winter.
    let tf = package(TENERIFE, LandClass::Default, |_, _| 300.0);
    let dry = bits(&tf, Some(&day(TENERIFE, 220.0)), &Params::FULL);
    let wet = bits(&tf, Some(&day(TENERIFE, 20.0)), &Params::FULL);
    assert!(
        count(&dry, Kind::Stalks) > count(&wet, Kind::Stalks),
        "{} {}",
        count(&dry, Kind::Stalks),
        count(&wet, Kind::Stalks)
    );
    // Winter on the Feldberg: the climatological snow covers the small
    // bits, the rocks stay.
    let feldberg = package(at, LandClass::Meadow, |_, _| 1400.0);
    let snowy = bits(&feldberg, Some(&winter), &Params::FULL);
    let green = bits(&feldberg, Some(&summer), &Params::FULL);
    assert!(winter.snow(1400.0) > 0.8, "{}", winter.snow(1400.0));
    assert!(count(&snowy, Kind::Tuft) * 10 < count(&green, Kind::Tuft).max(10));
    assert!(count(&snowy, Kind::Rock) * 2 > count(&green, Kind::Rock));
    // Lying snow of the weather: only rocks and shrubs.
    let lying = bits(
        &pkg,
        Some(&summer),
        &Params {
            density: 1.0,
            snow: true,
        },
    );
    assert!(!lying.is_empty());
    assert!(lying.iter().all(|b| !b.kind.small()));
}

#[test]
fn the_presets_draw_a_share_up_to_their_distance() {
    let pkg = package(LIECHTENSTEIN, LandClass::Meadow, flat);
    let n = |level| {
        let q = QualityPreset::of(level);
        bits(
            &pkg,
            None,
            &Params {
                density: q.ground_detail,
                snow: false,
            },
        )
        .len() as f32
    };
    let (low, medium, high) = (
        n(QualityLevel::Low),
        n(QualityLevel::Medium),
        n(QualityLevel::High),
    );
    assert!(
        low > 0.0 && low < medium && medium < high,
        "{low} {medium} {high}"
    );
    assert!((low / high - 0.25).abs() < 0.08, "{}", low / high);
    assert!((medium / high - 0.6).abs() < 0.08, "{}", medium / high);
    // The distances grow with the level; VR is modest.
    let d = |l| QualityPreset::of(l).ground_detail_m;
    assert!(
        d(QualityLevel::Low) < d(QualityLevel::Medium)
            && d(QualityLevel::Medium) < d(QualityLevel::High)
    );
    assert!((30.0..=80.0).contains(&d(QualityLevel::High)));
    assert!(VR_LEVEL.0 <= QualityPreset::of(QualityLevel::High).ground_detail);
    // Off: nothing.
    assert!(
        bits(
            &pkg,
            None,
            &Params {
                density: 0.0,
                snow: false
            }
        )
        .is_empty()
    );
    let off = GroundDetailSettings {
        mode: GroundDetailMode::Off,
    };
    assert_eq!(level(&QualityPreset::default(), &off), (0.0, 0.0));
    assert_eq!(
        level(&QualityPreset::default(), &GroundDetailSettings::default()),
        (1.0, 80.0)
    );
}

#[test]
fn the_setting_follows_the_preset_or_overrides_it() {
    // Spec 4.6.2 Q1: `display.ground_detail` auto (the default) follows
    // the preset, low ≈ the Low preset's, full ≈ the High preset's
    // whatever the preset, off draws nothing.
    assert_eq!(GroundDetailSettings::default().mode, GroundDetailMode::Auto);
    assert_eq!(GroundDetailSettings::default().mode.name(), "auto");
    let s = |mode| GroundDetailSettings { mode };
    for l in [QualityLevel::Low, QualityLevel::Medium, QualityLevel::High] {
        let q = QualityPreset::of(l);
        assert_eq!(
            level(&q, &s(GroundDetailMode::Auto)),
            (q.ground_detail, q.ground_detail_m),
            "{l:?}"
        );
        assert_eq!(level(&q, &s(GroundDetailMode::Off)), (0.0, 0.0));
        assert_eq!(level(&q, &s(GroundDetailMode::Low)), LOW_LEVEL);
        assert_eq!(level(&q, &s(GroundDetailMode::Full)), FULL_LEVEL);
    }
    let (low, high) = (
        QualityPreset::of(QualityLevel::Low),
        QualityPreset::of(QualityLevel::High),
    );
    assert_eq!(LOW_LEVEL, (low.ground_detail, low.ground_detail_m));
    assert_eq!(FULL_LEVEL, (high.ground_detail, high.ground_detail_m));
    // The words: the setting's choices, and the console's old `on`.
    for c in GroundDetailMode::CHOICES {
        assert_eq!(GroundDetailMode::parse(c).unwrap().name(), c);
    }
    assert_eq!(GroundDetailMode::parse("on"), Some(GroundDetailMode::Auto));
    assert_eq!(GroundDetailMode::parse("lots"), None);
    // VR (Q2, decided): 50 % up to 50 m, between Medium and Low.
    assert_eq!(VR_LEVEL, (0.5, 50.0));
    let medium = QualityPreset::of(QualityLevel::Medium);
    assert!(VR_LEVEL.0 > low.ground_detail && VR_LEVEL.0 < medium.ground_detail);
}

#[test]
fn the_spread_keeps_the_count_and_thins_out_to_the_band_end() {
    // The trapezoid: even to 7 m, thinning out linearly to 14 m.
    assert_eq!(spread(1.0, 7.0, 14.0, 0.0), 1.0);
    assert!((spread(1.0, 7.0, 14.0, 1.0) - 14.0).abs() < 1e-4);
    let total = 6.0 + 3.5;
    assert!((spread(1.0, 7.0, 14.0, 6.0 / total) - 7.0).abs() < 1e-4);
    // Half the tail's bits lie in its first 29 % (1 - 1/√2).
    let mid = spread(1.0, 7.0, 14.0, (6.0 + 3.5 / 2.0) / total);
    assert!((mid - (7.0 + 7.0 * (1.0 - std::f32::consts::FRAC_1_SQRT_2))).abs() < 1e-3);
    // No tail: even.
    assert_eq!(spread(-0.25, 0.5, 0.5, 0.5), 0.125);
    // Monotone.
    let mut last = f32::MIN;
    for i in 0..=100 {
        let d = spread(0.3, 7.0, 14.0, i as f32 / 100.0);
        assert!(d >= last);
        last = d;
    }
    // On a meadow road (x 600 – 900 m, only the road along x): the tufts
    // are as many as the mix says (the same total as the 7 m band had),
    // a third of them now beyond 7 m, none beyond 14 m.
    let pkg = package(LIECHTENSTEIN, LandClass::Meadow, flat);
    let all = bits(&pkg, None, &Params::FULL);
    let tufts: Vec<f32> = all
        .iter()
        .filter(|b| b.kind == Kind::Tuft && (600.0..900.0).contains(&b.pos.x))
        .map(|b| (b.pos.z - 500.0).abs() - 3.0)
        .collect();
    let place = Place {
        region: FloraRegion::AlpineRhine,
        class: LandClass::Meadow,
        alt_m: 300.0,
        broadleaf: region_broadleaf(FloraRegion::AlpineRhine, 300.0),
        surface: RoadSurface::Asphalt,
        sidewalk: false,
    };
    let expected = mix(&place, &SeasonFx::SUMMER).get(Kind::Tuft) * 300.0 * 2.0;
    let n = tufts.len() as f32;
    assert!(
        (n / expected - 1.0).abs() < 0.06,
        "{n} tufts, {expected} expected"
    );
    let far = tufts.iter().filter(|d| **d > NEAR_BAND_M).count() as f32 / n;
    let (b0, f, b1, _) = Kind::Tuft.band();
    let tail = (b1 - f) / 2.0 / (f - b0 + (b1 - f) / 2.0);
    assert!(
        (far - tail).abs() < 0.06,
        "{far} beyond 7 m, {tail} expected"
    );
    assert!(far > 0.25);
    assert!(tufts.iter().all(|d| *d <= BAND_M + 0.01));
    // Wider than the first band, thinner at its end than at its start.
    let between = |a: f32, b: f32| tufts.iter().filter(|d| (a..b).contains(*d)).count();
    assert!(between(2.0, 4.0) > between(11.0, 13.0) * 2);
    assert!(between(11.0, 13.0) > 0);
}

#[test]
fn very_few_bits_on_the_road_and_none_where_the_wheels_run() {
    let pkg = package(BLACK_FOREST, LandClass::Meadow, flat);
    let all = bits(&pkg, Some(&day(BLACK_FOREST, 300.0)), &Params::FULL);
    let road: Vec<&Bit> = all
        .iter()
        .filter(|b| b.road && (b.pos.z - 500.0).abs() < 3.0 && b.pos.x < 290.0)
        .collect();
    // The 6 m road along x (tertiary, two lanes; 290 m of it): a few dozen,
    // well under one per metre.
    assert!(road.len() > 8, "{} on the road", road.len());
    assert!(
        (road.len() as f32) < 0.25 * 290.0,
        "{} on the road",
        road.len()
    );
    // Never in a wheel track or a lane's middle (± 1.5 m), on the road's
    // surface.
    for b in &road {
        let u = b.pos.z - 500.0;
        assert!(
            ocw_world::road_wear::wheel_tracks(u, 3.0, [0, 0]) < TRACK_CLEAR,
            "{b:?} in a wheel track"
        );
        assert!(
            (u.abs() - 1.5).abs() > LANE_MIDDLE_CLEAR_M,
            "{b:?}: lane middle"
        );
        assert!(Kind::ON_ROAD.contains(&b.kind));
        let top = pkg.road_heights(f64::from(b.pos.x), 500.0)[0] as f32;
        assert!((b.pos.y - top - crate::ROAD_SURFACE_LIFT_M).abs() < 0.01);
        assert_eq!(b.normal, Vec3::Y);
    }
    // Most of them grit and small stones; small ones only.
    let stones = road
        .iter()
        .filter(|b| matches!(b.kind, Kind::Gravel | Kind::Pebble))
        .count();
    assert!(stones * 2 > road.len());
    assert!(
        road.iter()
            .filter(|b| b.kind == Kind::Pebble)
            .all(|b| b.size <= 0.055)
    );
    assert!(road_spot_free(-2.9, 3.0, [0, 0]) && !road_spot_free(-1.5, 3.0, [0, 0]));
    assert!(!road_spot_free(-2.3, 3.0, [0, 0]) && road_spot_free(0.0, 3.0, [0, 0]));
    // A worn road has more, like the road damage's loose stones.
    let place = Place {
        region: FloraRegion::BlackForest,
        class: LandClass::Meadow,
        alt_m: 300.0,
        broadleaf: 0.3,
        surface: RoadSurface::Asphalt,
        sidewalk: false,
    };
    let g = |grade| road_mix(&place, &SeasonFx::SUMMER, grade).get(Kind::Gravel);
    assert!(g(7) > g(4) && g(4) > g(1) && g(1) > 0.0);
    // Snow: nothing small on the road; water: nothing.
    let mut horrible = pkg.clone();
    for r in &mut horrible.tiles.get_mut(&KEY).unwrap().roads {
        r.condition.smoothness = ocw_world::road_condition::Smoothness::Horrible;
    }
    let worn = bits(&horrible, Some(&day(BLACK_FOREST, 300.0)), &Params::FULL)
        .iter()
        .filter(|b| b.road)
        .count();
    let fine = all.iter().filter(|b| b.road).count();
    assert!(worn > fine, "{worn} on a horrible road, {fine}");
    let lying = Params {
        density: 1.0,
        snow: true,
    };
    assert!(!bits(&pkg, None, &lying).iter().any(|b| b.road));
}

#[test]
fn wet_ground_detail_darkens_and_shines() {
    assert_eq!(wet_look(0.0), (1.0, 1.0));
    let (k, r) = wet_look(1.0);
    assert!((k - (1.0 - WET_DARKEN)).abs() < 1e-6 && (r - WET_ROUGHNESS).abs() < 1e-6);
    // Between the wet road's asphalt (0.55) and unpaved (0.72) darkening.
    assert!((0.55..=0.72).contains(&k), "{k}");
    // Monotone, and clamped.
    let (k5, r5) = wet_look(0.5);
    assert!(k5 < 1.0 && k5 > k && r5 < 1.0 && r5 > r);
    assert_eq!(wet_look(2.0), wet_look(1.0));
    assert_eq!(wet_look(-1.0), wet_look(0.0));
}

#[test]
fn autumn_lays_leaf_patches_and_leaves_on_the_road() {
    let at = BLACK_FOREST;
    let mut pkg = package(at, LandClass::Forest, |_, _| 400.0);
    pkg.origin = at;
    let summer = bits(&pkg, Some(&day(at, 200.0)), &Params::FULL);
    let autumn = bits(&pkg, Some(&day(at, 300.0)), &Params::FULL);
    let winter = bits(&pkg, Some(&day(at, 15.0)), &Params::FULL);
    let patches = |v: &[Bit]| count(v, Kind::LeafLitter);
    let road_leaves = |v: &[Bit]| v.iter().filter(|b| b.road && b.kind == Kind::Leaf).count();
    // Summer: none of either; autumn: both; winter: still some (brown).
    assert_eq!(patches(&summer), 0);
    assert_eq!(road_leaves(&summer), 0);
    assert!(patches(&autumn) > 50, "{} leaf patches", patches(&autumn));
    assert!(
        road_leaves(&autumn) > 30,
        "{} leaves on the road",
        road_leaves(&autumn)
    );
    assert!(patches(&winter) > 0 && road_leaves(&winter) > 0);
    // The patches are blended and leaf-coloured (warmer than the soil).
    let p = autumn.iter().find(|b| b.kind == Kind::LeafLitter).unwrap();
    assert!(p.kind.patch() && p.colour[0] > p.colour[2] * 2.0, "{p:?}");
    // A few more single leaves beside the road than before (6 → 7.5).
    let place = Place {
        region: FloraRegion::BlackForest,
        class: LandClass::Forest,
        alt_m: 400.0,
        broadleaf: 0.3,
        surface: RoadSurface::Asphalt,
        sidewalk: false,
    };
    let fx = SeasonFx::of(Some(&day(at, 300.0)), 400.0);
    assert!((mix(&place, &fx).get(Kind::Leaf) - 7.5 * 0.3 * fx.litter).abs() < 1e-5);
    // Determinism: the same again.
    assert_eq!(autumn, bits(&pkg, Some(&day(at, 300.0)), &Params::FULL));
}

#[test]
fn chunks_are_built_near_a_camera_and_dropped_far_away() {
    let draw = 80.0;
    // Not built: built within the distance and the margin.
    assert_eq!(ground_step(10.0, draw, false, false), GroundStep::Build);
    assert_eq!(
        ground_step(draw + BUILD_MARGIN_M, draw, false, false),
        GroundStep::Build
    );
    assert_eq!(
        ground_step(draw + BUILD_MARGIN_M + 1.0, draw, false, false),
        GroundStep::Keep
    );
    // Built: kept up to the drop distance, rebuilt for another preset.
    assert_eq!(
        ground_step(draw + DROP_MARGIN_M - 1.0, draw, true, true),
        GroundStep::Keep
    );
    assert_eq!(
        ground_step(draw + DROP_MARGIN_M + 1.0, draw, true, true),
        GroundStep::Drop
    );
    assert_eq!(ground_step(20.0, draw, true, false), GroundStep::Build);
    // Off: dropped.
    assert_eq!(ground_step(5.0, 0.0, true, true), GroundStep::Drop);
    // Drawn up to the distance, faded out over its last quarter.
    let r = detail_range(draw);
    assert_eq!(r.end_margin, 60.0..80.0);
    assert!(!r.use_aabb);
    const { assert!(BUILD_MARGIN_M < DROP_MARGIN_M) };
}

#[test]
fn the_models_stay_low_poly() {
    let pkg = package(BLACK_FOREST, LandClass::Forest, flat);
    let all = bits(&pkg, Some(&day(BLACK_FOREST, 300.0)), &Params::FULL);
    for k in Kind::ALL {
        let Some(b) = all.iter().find(|b| b.kind == k) else {
            continue;
        };
        let (mut s, mut p) = (MeshArrays::default(), MeshArrays::default());
        add_bit(&mut s, &mut p, b, Vec3::ZERO);
        let v = s.positions.len() + p.positions.len();
        let budget = match k {
            Kind::Fern => 140,
            Kind::Shrub => 70,
            Kind::Rock => 30,
            _ => 25,
        };
        assert!(v <= budget, "{}: {v} vertices", k.name());
        assert_eq!(s.uvs.len(), s.positions.len(), "{}", k.name());
        assert_eq!(p.colors.len(), p.positions.len(), "{}", k.name());
    }
    // A forest road's 25 m chunk: a few thousand vertices.
    let (solid, patches) = meshes(&all, Vec3::ZERO);
    let per_m = (solid.positions.len() + patches.positions.len()) as f32 / 1400.0;
    assert!(per_m < 400.0, "{per_m} vertices per metre of road");
}

#[test]
fn the_tile_build_keeps_the_chunks_near_the_roads() {
    let pkg = package(LIECHTENSTEIN, LandClass::Meadow, flat);
    let m = crate::package::build_tile_meshes(&pkg, KEY).unwrap();
    let d = &m.ground_detail;
    assert!(!d.chunks.is_empty());
    // Only chunks near the two roads (z = 500, x = 300).
    for c in &d.chunks {
        let [x, _, z] = c.centre;
        let near = (z - 500.0).abs() < 40.0 || ((x - 300.0).abs() < 40.0 && z > 560.0);
        assert!(near, "{c:?}");
    }
    assert!(d.bytes() < 400_000, "{}", d.bytes());
}

/// The memory per resident tile (spec §14 Phase 10 B2): a tile's chunks
/// do not keep the package version they were built with alive (a streamed
/// package's versions come and go); once it is gone they build from their
/// member's current package, and not at all once the tile has left it.
#[test]
fn detail_chunks_do_not_keep_their_package() {
    let pkg = Arc::new(package(LIECHTENSTEIN, LandClass::Meadow, flat));
    let mesh = terrain(&pkg);
    let detail = tile_detail(&pkg, KEY, None, &mesh, &SolidFootprints::default(), None);
    let chunk = detail.chunks[0].clone();
    let src = DetailSource {
        pkg: Arc::downgrade(&pkg),
        member: 0,
        tile: detail,
    };
    assert!(Arc::ptr_eq(&src.package(None).unwrap(), &pkg));
    let built = src.build(&pkg, &chunk, &Params::FULL);
    let current = Arc::new((*pkg).clone());
    let world = crate::package::PackageWorld::single(current.clone());
    drop(pkg);
    assert!(src.package(None).is_none(), "the old version was kept");
    let now = src.package(Some(&world)).unwrap();
    assert!(Arc::ptr_eq(&now, &current));
    // The same chunk from the current version.
    let again = src.build(&now, &chunk, &Params::FULL);
    assert_eq!((built.0.positions, built.2), (again.0.positions, again.2));
    let mut gone = (*current).clone();
    gone.tiles.remove(&KEY);
    let without = crate::package::PackageWorld::single(Arc::new(gone));
    assert!(src.package(Some(&without)).is_none());
}
