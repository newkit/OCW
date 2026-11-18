//! Where a Strava (S … S) recording is written (spec §2.8.1 *Where an S
//! recording is written*, v4.79 / v4.80), Bevy-free and wasm-safe: every
//! recording gets its own subfolder of the **activities folder**
//! (`~/Activities/OCW`, setting `activities.folder`), named by its ISO
//! date and the place it started at, holding its FIT file and its photos:
//!
//! ```text
//! ~/Activities/OCW/2026-11-18 Puerto de la Cruz/
//!     OCW 2026-11-18 10-05 Puerto de la Cruz.fit
//!     OCW 2026-11-18 10-05 Puerto de la Cruz photo 1.jpg
//! ```
//!
//! Built as the spec proposed: the place is the nearest town or village
//! of the package's places (`ocw_world::places`, PLCE) within
//! [`PLACE_RADIUS_M`] of where the recording **started** (rank up to
//! [`PLACE_MAX_RANK`], a village), else the route's name, else the
//! world's name; the date is the world's local date (the place's time
//! zone); a second recording at the same place on the same day gets ` 2`,
//! ` 3`, … ([`unique`]); the time goes into the file names; only
//! characters every system accepts ([`crate::strava::safe_file_part`]).

use ocw_world::places::Places;

/// The activities folder below the home folder (every platform).
pub const DEFAULT_SUBFOLDER: [&str; 2] = ["Activities", "OCW"];
/// How far the nearest town or village may be from the start (m).
pub const PLACE_RADIUS_M: f64 = 5_000.0;
/// The least important place that names a folder (a village's rank).
pub const PLACE_MAX_RANK: u8 = 2;
/// The longest place in a folder or file name (characters).
pub const PLACE_MAX_CHARS: usize = 60;

/// The nearest town or village to `at` (package metres x, z) within
/// [`PLACE_RADIUS_M`], of rank at most [`PLACE_MAX_RANK`].
pub fn nearest_place(places: &Places, at: [f64; 2]) -> Option<&str> {
    places
        .places
        .iter()
        .filter(|p| p.rank <= PLACE_MAX_RANK && !p.name.trim().is_empty())
        .map(|p| {
            let dx = f64::from(p.pos[0]) - at[0];
            let dz = f64::from(p.pos[1]) - at[1];
            (p, (dx * dx + dz * dz).sqrt())
        })
        .filter(|(_, d)| *d <= PLACE_RADIUS_M)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p.name.as_str())
}

/// The place a recording is named after: the nearest town or village to
/// where it started, else the route's name, else the world's name, else
/// "Ride"; safe for every file system.
pub fn place_name(
    places: Option<&Places>,
    at: Option<[f64; 2]>,
    route: &str,
    world: &str,
) -> String {
    let near = places.zip(at).and_then(|(p, at)| nearest_place(p, at));
    [near.unwrap_or(""), route, world]
        .into_iter()
        .map(|s| crate::strava::safe_file_part(s, PLACE_MAX_CHARS))
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "Ride".into())
}

/// The local date and time of `unix_s` at `offset_s`: (`YYYY-MM-DD`,
/// `HH-MM`).
pub fn local_date_time(unix_s: i64, offset_s: i32) -> (String, String) {
    let (y, mo, d, h, mi, _) = ocw_core::timestamp::civil(unix_s + i64::from(offset_s));
    (format!("{y:04}-{mo:02}-{d:02}"), format!("{h:02}-{mi:02}"))
}

/// The folder's name before [`unique`]: `2026-11-18 Puerto de la Cruz`.
pub fn folder_base(unix_s: i64, offset_s: i32, place: &str) -> String {
    format!("{} {place}", local_date_time(unix_s, offset_s).0)
}

/// `base`, or `base 2`, `base 3`, … – the first that `taken` says is free.
pub fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !taken(n))
        .expect("a free name")
}

/// The files' common start: `OCW 2026-11-18 10-05 Puerto de la Cruz`.
pub fn file_stem(unix_s: i64, offset_s: i32, place: &str) -> String {
    let (date, time) = local_date_time(unix_s, offset_s);
    format!("OCW {date} {time} {place}")
}

/// Where one recording goes: its subfolder and its files' names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The subfolder's name (`2026-11-18 Puerto de la Cruz`, ` 2` …).
    pub folder: String,
    /// The files' common start ([`file_stem`]).
    pub stem: String,
    /// Photos written so far.
    pub photos: usize,
}

impl Target {
    /// The target of a recording that starts at `unix_s` at `place`;
    /// `taken` says which folder names are used already.
    pub fn new(unix_s: i64, offset_s: i32, place: &str, taken: impl Fn(&str) -> bool) -> Self {
        Self {
            folder: unique(&folder_base(unix_s, offset_s, place), taken),
            stem: file_stem(unix_s, offset_s, place),
            photos: 0,
        }
    }

    /// The FIT file's name.
    pub fn fit_name(&self) -> String {
        format!("{}.fit", self.stem)
    }

    /// The name of photo `n` (from 1).
    pub fn photo_name(&self, n: usize) -> String {
        format!("{} photo {n}.jpg", self.stem)
    }

    /// The next photo's name (counts it).
    pub fn next_photo(&mut self) -> String {
        self.photos += 1;
        self.photo_name(self.photos)
    }

    /// The folder as the HUD names it, below the activities folder's
    /// label: `Activities/OCW/2026-11-18 Puerto de la Cruz`.
    pub fn label(&self, root: &str) -> String {
        if root.is_empty() {
            self.folder.clone()
        } else {
            format!("{}/{}", root.trim_end_matches('/'), self.folder)
        }
    }

    /// A file's path below the activities folder (`/`-separated, the
    /// browser's folder writer and its tests).
    pub fn rel(&self, file: &str) -> String {
        format!("{}/{file}", self.folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::places::{OsmElement, Place, PlaceKind, place_rank};

    fn place(name: &str, kind: PlaceKind, x: f32, z: f32) -> Place {
        Place {
            kind,
            rank: place_rank(kind, None, None),
            pos: [x, z],
            name: name.into(),
            names: Vec::new(),
            population: None,
            capital: None,
            area: false,
            element: OsmElement::Node,
            osm_id: 1,
        }
    }

    fn places() -> Places {
        Places::new(vec![
            place("Puerto de la Cruz", PlaceKind::Town, 0.0, 0.0),
            place("La Vera", PlaceKind::Village, 2_000.0, 0.0),
            // Hamlets and localities never name a folder.
            place("El Rincón", PlaceKind::Hamlet, 2_900.0, 0.0),
            place("Mirador", PlaceKind::Locality, 3_000.0, 10.0),
            place("Far Town", PlaceKind::Town, 20_000.0, 0.0),
        ])
    }

    /// The place: the nearest town or village to the start, else the
    /// route, else the world (made-up places where not on the island).
    #[test]
    fn the_place_comes_from_the_places_then_the_route_then_the_world() {
        let p = places();
        let name = |at| place_name(Some(&p), Some(at), "Puerto – Teide", "Tenerife");
        assert_eq!(name([100.0, 50.0]), "Puerto de la Cruz");
        assert_eq!(name([1_800.0, 0.0]), "La Vera");
        // Nearest is a hamlet: the nearest village wins.
        assert_eq!(name([2_950.0, 0.0]), "La Vera");
        // No town or village within 5 km: the route.
        assert_eq!(name([11_000.0, 0.0]), "Puerto – Teide");
        assert_eq!(name([19_000.0, 0.0]), "Far Town");
        // No places (an older package), no position: the route, the world.
        assert_eq!(
            place_name(None, Some([0.0, 0.0]), "Loop", "Test world"),
            "Loop"
        );
        assert_eq!(place_name(Some(&p), None, "", "Test world"), "Test world");
        assert_eq!(place_name(None, None, " ", ""), "Ride");
    }

    /// Characters every system accepts, in the place too.
    #[test]
    fn names_are_safe() {
        let p = Places::new(vec![place("Ada/Town: *?", PlaceKind::Town, 0.0, 0.0)]);
        let n = place_name(Some(&p), Some([0.0, 0.0]), "", "");
        assert_eq!(n, "Ada-Town- --");
        let t = Target::new(1_794_992_700, 0, &n, |_| false);
        for s in [&t.folder, &t.stem] {
            assert!(
                !s.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']),
                "{s}"
            );
        }
        let long = place_name(None, None, &"Bea Climber's Route ".repeat(10), "");
        assert!(long.chars().count() <= PLACE_MAX_CHARS, "{long}");
    }

    /// The ISO date in the world's local date, the time in the files'
    /// names, ` 2` for a second recording there that day.
    #[test]
    fn the_folder_and_its_files() {
        // 2026-11-18 23:30 UTC: still the 18th in Tenerife (UTC+0), the
        // 19th at UTC+1.
        let t = 1_795_044_600;
        let a = Target::new(t, 0, "Puerto de la Cruz", |_| false);
        assert_eq!(a.folder, "2026-11-18 Puerto de la Cruz");
        assert_eq!(a.stem, "OCW 2026-11-18 23-30 Puerto de la Cruz");
        let b = Target::new(t, 3600, "Vaduz", |_| false);
        assert_eq!(b.folder, "2026-11-19 Vaduz");
        assert_eq!(b.fit_name(), "OCW 2026-11-19 00-30 Vaduz.fit");
        // A second and third recording at the same place on the same day.
        let used = [
            "2026-11-18 Puerto de la Cruz",
            "2026-11-18 Puerto de la Cruz 2",
        ];
        let c = Target::new(t, 0, "Puerto de la Cruz", |n| used.contains(&n));
        assert_eq!(c.folder, "2026-11-18 Puerto de la Cruz 3");
        let mut c = c;
        assert_eq!(
            c.next_photo(),
            "OCW 2026-11-18 23-30 Puerto de la Cruz photo 1.jpg"
        );
        assert_eq!(
            c.next_photo(),
            "OCW 2026-11-18 23-30 Puerto de la Cruz photo 2.jpg"
        );
        assert_eq!(
            a.label("Activities/OCW"),
            "Activities/OCW/2026-11-18 Puerto de la Cruz"
        );
        assert_eq!(
            a.rel(&a.fit_name()),
            "2026-11-18 Puerto de la Cruz/OCW 2026-11-18 23-30 Puerto de la Cruz.fit"
        );
    }
}
