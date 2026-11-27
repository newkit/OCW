//! Aerodromes, runways and helipads of a package (spec 14 Phase 18, 18C
//! *Data from the package*): what the simulated aircraft take off from,
//! land on and circle – OSM `aeroway=aerodrome` (with `aerodrome:type`,
//! the ICAO and IATA codes), `aeroway=runway` (heading, length, width),
//! `aeroway=helipad | heliport` (hospitals: rescue helicopters), gliding
//! sites (`aerodrome:type=gliding`, `sport=gliding`), plus the airports
//! just outside the package that matter for its sky ([`NEARBY`]: TFN and
//! TFS for Tenerife, Zurich, Altenrhein and Friedrichshafen for
//! Liechtenstein, Karlsruhe / Baden-Baden, Strasbourg, Stuttgart and
//! Basel-Mulhouse for the Black Forest, La Seu d'Urgell, Barcelona and
//! Toulouse for Andorra). Pure data, wasm-safe.
//!
//! **Section `AERO`** (optional, additive; older readers skip it; a
//! package without it has no aerodromes – the sky then has only
//! overflights):
//!
//! ```text
//! layout version: u8 (1)
//! aerodrome count: u32, per aerodrome:
//!   OSM id: i64 (ways and relations −id; 0 from the NEARBY table)
//!   name, ICAO, IATA: str str str   kind: u8
//!   lat, lon: f64 f64   x, z: f32 f32 (package metres)
//!   elevation: f32 (m, NaN unknown)   outside the package: u8
//! runway count: u32, per runway:
//!   aerodrome: u16 (index, 0xFFFF none)   ref: str
//!   ends: [f32; 4] (x0, z0, x1, z1)   heading: f32 (° true, end 0 → 1)
//!   length, width: f32 f32 (m)   surface: str
//! helipad count: u32, per helipad:
//!   OSM id: i64   name: str   x, z: f32 f32   elevation: f32 (NaN)
//!   flags: u8 (1 hospital, 2 heliport)
//! str = u16 length + UTF-8
//! ```

use crate::geo::LocalProjection;
use crate::package::Bytes;
use crate::traffic::get_str;

/// Section tag.
pub const AERO_TAG: &[u8; 4] = b"AERO";
const AERO_VERSION: u8 = 1;

/// What kind of aerodrome (for the flights it gets: airliners only at the
/// big ones, circuits at airfields, gliders at gliding sites).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AerodromeKind {
    /// Scheduled international traffic (`aerodrome:type=international`,
    /// or an IATA code with a long runway).
    International = 0,
    /// Regional airports (`regional`, `public` with an IATA code).
    Regional = 1,
    /// Small public airfields (`public`, `airfield`, untyped).
    Airfield = 2,
    /// Private strips (`private`).
    Private = 3,
    /// Military (`military`).
    Military = 4,
    /// Gliding sites (`gliding`, `sport=gliding`).
    Gliding = 5,
    /// Ultralight and model airfields (`ultralight`).
    Ultralight = 6,
    /// A heliport mapped as an aerodrome.
    Heliport = 7,
}

impl AerodromeKind {
    fn from_code(c: u8) -> Self {
        match c {
            0 => Self::International,
            1 => Self::Regional,
            3 => Self::Private,
            4 => Self::Military,
            5 => Self::Gliding,
            6 => Self::Ultralight,
            7 => Self::Heliport,
            _ => Self::Airfield,
        }
    }

    /// From the OSM tags of an `aeroway=aerodrome` (or `heliport`).
    pub fn from_osm<'a>(tag: impl Fn(&str) -> Option<&'a str>) -> Self {
        if tag("aeroway") == Some("heliport") {
            return Self::Heliport;
        }
        if tag("sport").is_some_and(|s| s.split(';').any(|s| s.trim() == "gliding")) {
            return Self::Gliding;
        }
        let ty = tag("aerodrome:type").or_else(|| tag("aerodrome"));
        match ty.map(str::trim) {
            Some("international") => Self::International,
            Some("regional") => Self::Regional,
            Some("gliding") => Self::Gliding,
            Some("military" | "military/public") => Self::Military,
            Some("private") => Self::Private,
            Some("ultralight" | "model") => Self::Ultralight,
            Some("public") if tag("iata").is_some() => Self::Regional,
            _ if tag("military").is_some() => Self::Military,
            _ => Self::Airfield,
        }
    }
}

/// An aerodrome inside the package, or a nearby one outside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Aerodrome {
    pub osm_id: i64,
    pub name: String,
    /// ICAO code ("GCXO"), empty if none.
    pub icao: String,
    /// IATA code ("TFN"), empty if none.
    pub iata: String,
    pub kind: AerodromeKind,
    pub lat_lon: (f64, f64),
    /// Package metres (x east, z south).
    pub pos: [f32; 2],
    pub ele_m: Option<f32>,
    /// From [`NEARBY`]: outside the package's tiles.
    pub outside: bool,
}

/// A runway.
#[derive(Debug, Clone, PartialEq)]
pub struct Runway {
    /// Index into [`Aero::aerodromes`].
    pub aerodrome: Option<u16>,
    /// "12/30".
    pub reference: String,
    /// The two ends (package metres).
    pub ends: [[f32; 2]; 2],
    /// True heading from end 0 to end 1 (°, 0 = north, clockwise).
    pub heading_deg: f32,
    pub length_m: f32,
    pub width_m: f32,
    pub surface: String,
}

impl Runway {
    /// The runway from its two ends (lat, lon) in `proj`.
    pub fn from_ends(
        reference: &str,
        a: (f64, f64),
        b: (f64, f64),
        width_m: f32,
        proj: &LocalProjection,
    ) -> Self {
        let (pa, pb) = (proj.project(a.0, a.1), proj.project(b.0, b.1));
        let (dx, dz) = (pb[0] - pa[0], pb[1] - pa[1]);
        // x east, z south: north is −z.
        let heading = dx.atan2(-dz).to_degrees().rem_euclid(360.0);
        Self {
            aerodrome: None,
            reference: reference.to_string(),
            ends: [[pa[0] as f32, pa[1] as f32], [pb[0] as f32, pb[1] as f32]],
            heading_deg: heading as f32,
            length_m: dx.hypot(dz) as f32,
            width_m,
            surface: String::new(),
        }
    }

    /// The centre of the runway.
    pub fn centre(&self) -> [f32; 2] {
        [
            (self.ends[0][0] + self.ends[1][0]) / 2.0,
            (self.ends[0][1] + self.ends[1][1]) / 2.0,
        ]
    }
}

/// A helipad or heliport.
#[derive(Debug, Clone, PartialEq)]
pub struct Helipad {
    pub osm_id: i64,
    pub name: String,
    pub pos: [f32; 2],
    pub ele_m: Option<f32>,
    /// At a hospital (rescue helicopters).
    pub hospital: bool,
    /// A heliport (`aeroway=heliport`: a base, e.g. Balzers), not a pad.
    pub heliport: bool,
}

/// The aerodromes of a package (section `AERO`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Aero {
    pub aerodromes: Vec<Aerodrome>,
    pub runways: Vec<Runway>,
    pub helipads: Vec<Helipad>,
}

impl Aero {
    pub fn is_empty(&self) -> bool {
        self.aerodromes.is_empty() && self.runways.is_empty() && self.helipads.is_empty()
    }

    /// The runways of aerodrome `index`.
    pub fn runways_of(&self, index: usize) -> impl Iterator<Item = &Runway> {
        self.runways
            .iter()
            .filter(move |r| r.aerodrome.map(usize::from) == Some(index))
    }

    /// The aerodrome with this ICAO code.
    pub fn by_icao(&self, icao: &str) -> Option<&Aerodrome> {
        self.aerodromes.iter().find(|a| a.icao == icao)
    }

    /// Adds the airports of [`NEARBY`] within `radius_km` of `centre`
    /// (lat, lon) that are not in the package already (by ICAO), with
    /// their runways; returns how many.
    pub fn add_nearby(
        &mut self,
        centre: (f64, f64),
        radius_km: f64,
        proj: &LocalProjection,
    ) -> usize {
        let mut added = 0;
        for n in NEARBY {
            if self.by_icao(n.icao).is_some() || distance_km(centre, (n.lat, n.lon)) > radius_km {
                continue;
            }
            let p = proj.project(n.lat, n.lon);
            let index = self.aerodromes.len() as u16;
            self.aerodromes.push(Aerodrome {
                osm_id: 0,
                name: n.name.to_string(),
                icao: n.icao.to_string(),
                iata: n.iata.to_string(),
                kind: n.kind,
                lat_lon: (n.lat, n.lon),
                pos: [p[0] as f32, p[1] as f32],
                ele_m: Some(n.ele_m),
                outside: true,
            });
            for &(reference, heading, length) in n.runways {
                let h = f64::from(heading).to_radians();
                let half = f64::from(length) / 2.0;
                // x east, z south.
                let (dx, dz) = (h.sin() * half, -h.cos() * half);
                self.runways.push(Runway {
                    aerodrome: Some(index),
                    reference: reference.to_string(),
                    ends: [
                        [(p[0] - dx) as f32, (p[1] - dz) as f32],
                        [(p[0] + dx) as f32, (p[1] + dz) as f32],
                    ],
                    heading_deg: heading,
                    length_m: length,
                    width_m: 45.0,
                    surface: "asphalt".into(),
                });
            }
            added += 1;
        }
        added
    }
}

/// Great-circle distance (km).
pub fn distance_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la, lb) = (a.0.to_radians(), b.0.to_radians());
    let dl = (b.1 - a.1).to_radians();
    let h = ((lb - la) / 2.0).sin().powi(2) + la.cos() * lb.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

/// An airport of the hand-made table of airports near our worlds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearbyAirport {
    pub icao: &'static str,
    pub iata: &'static str,
    pub name: &'static str,
    pub kind: AerodromeKind,
    pub lat: f64,
    pub lon: f64,
    pub ele_m: f32,
    /// (designator, true heading of the lower-numbered end's direction in
    /// °, length in m) – approximate, from the published aerodrome charts.
    pub runways: &'static [(&'static str, f32, f32)],
}

/// Airports near our worlds (spec 18C): approximate positions, headings
/// and lengths, good for the sky (approaches seen from afar), not for
/// navigation.
pub const NEARBY: &[NearbyAirport] = &[
    // Tenerife.
    NearbyAirport {
        icao: "GCXO",
        iata: "TFN",
        name: "Tenerife North",
        kind: AerodromeKind::International,
        lat: 28.4827,
        lon: -16.3415,
        ele_m: 632.0,
        runways: &[("12/30", 118.0, 3400.0)],
    },
    NearbyAirport {
        icao: "GCTS",
        iata: "TFS",
        name: "Tenerife South",
        kind: AerodromeKind::International,
        lat: 28.0445,
        lon: -16.5725,
        ele_m: 64.0,
        runways: &[("07/25", 73.0, 3200.0)],
    },
    // Liechtenstein.
    NearbyAirport {
        icao: "LSZH",
        iata: "ZRH",
        name: "Zurich",
        kind: AerodromeKind::International,
        lat: 47.4647,
        lon: 8.5492,
        ele_m: 432.0,
        runways: &[
            ("10/28", 96.0, 2500.0),
            ("14/32", 137.0, 3300.0),
            ("16/34", 155.0, 3700.0),
        ],
    },
    NearbyAirport {
        icao: "LSZR",
        iata: "ACH",
        name: "St. Gallen-Altenrhein",
        kind: AerodromeKind::Regional,
        lat: 47.4850,
        lon: 9.5608,
        ele_m: 398.0,
        runways: &[("10/28", 93.0, 1580.0)],
    },
    NearbyAirport {
        icao: "EDNY",
        iata: "FDH",
        name: "Friedrichshafen",
        kind: AerodromeKind::Regional,
        lat: 47.6713,
        lon: 9.5115,
        ele_m: 417.0,
        runways: &[("06/24", 58.0, 2356.0)],
    },
    // The Black Forest.
    NearbyAirport {
        icao: "EDSB",
        iata: "FKB",
        name: "Karlsruhe / Baden-Baden",
        kind: AerodromeKind::Regional,
        lat: 48.7794,
        lon: 8.0805,
        ele_m: 125.0,
        runways: &[("03/21", 32.0, 3000.0)],
    },
    NearbyAirport {
        icao: "LFST",
        iata: "SXB",
        name: "Strasbourg",
        kind: AerodromeKind::International,
        lat: 48.5383,
        lon: 7.6282,
        ele_m: 153.0,
        runways: &[("05/23", 49.0, 2400.0)],
    },
    NearbyAirport {
        icao: "EDDS",
        iata: "STR",
        name: "Stuttgart",
        kind: AerodromeKind::International,
        lat: 48.6899,
        lon: 9.2220,
        ele_m: 389.0,
        runways: &[("07/25", 72.0, 3345.0)],
    },
    NearbyAirport {
        icao: "LFSB",
        iata: "BSL",
        name: "Basel-Mulhouse",
        kind: AerodromeKind::International,
        lat: 47.5896,
        lon: 7.5299,
        ele_m: 270.0,
        runways: &[("15/33", 155.0, 3900.0), ("08/26", 80.0, 1820.0)],
    },
    // Andorra.
    NearbyAirport {
        icao: "LESU",
        iata: "LEU",
        name: "Andorra-La Seu d'Urgell",
        kind: AerodromeKind::Regional,
        lat: 42.3386,
        lon: 1.4094,
        ele_m: 1047.0,
        runways: &[("03/21", 30.0, 1300.0)],
    },
    NearbyAirport {
        icao: "LEBL",
        iata: "BCN",
        name: "Barcelona",
        kind: AerodromeKind::International,
        lat: 41.2971,
        lon: 2.0785,
        ele_m: 4.0,
        runways: &[("06L/24R", 65.0, 3352.0), ("02/20", 19.0, 2528.0)],
    },
    NearbyAirport {
        icao: "LFBO",
        iata: "TLS",
        name: "Toulouse",
        kind: AerodromeKind::International,
        lat: 43.6291,
        lon: 1.3638,
        ele_m: 152.0,
        runways: &[("14L/32R", 143.0, 3500.0)],
    },
];

/// How far from the package's centre [`NEARBY`] airports are added (km):
/// Barcelona and Toulouse from Andorra (~150 km) still shape its sky.
pub const NEARBY_RADIUS_KM: f64 = 170.0;

// --- Encoding -------------------------------------------------------------

fn put_str(b: &mut Vec<u8>, s: &str) {
    let bytes = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
    b.extend((bytes.len() as u16).to_le_bytes());
    b.extend(bytes);
}

fn put_ele(b: &mut Vec<u8>, e: Option<f32>) {
    b.extend(e.unwrap_or(f32::NAN).to_le_bytes());
}

fn get_ele(r: &mut Bytes) -> Result<Option<f32>, String> {
    let v = r.f32()?;
    Ok(v.is_finite().then_some(v))
}

/// Writes the `AERO` payload.
pub fn write_aero(a: &Aero, b: &mut Vec<u8>) {
    b.push(AERO_VERSION);
    b.extend((a.aerodromes.len() as u32).to_le_bytes());
    for d in &a.aerodromes {
        b.extend(d.osm_id.to_le_bytes());
        put_str(b, &d.name);
        put_str(b, &d.icao);
        put_str(b, &d.iata);
        b.push(d.kind as u8);
        b.extend(d.lat_lon.0.to_le_bytes());
        b.extend(d.lat_lon.1.to_le_bytes());
        b.extend(d.pos[0].to_le_bytes());
        b.extend(d.pos[1].to_le_bytes());
        put_ele(b, d.ele_m);
        b.push(u8::from(d.outside));
    }
    b.extend((a.runways.len() as u32).to_le_bytes());
    for r in &a.runways {
        b.extend(r.aerodrome.unwrap_or(u16::MAX).to_le_bytes());
        put_str(b, &r.reference);
        for e in r.ends {
            b.extend(e[0].to_le_bytes());
            b.extend(e[1].to_le_bytes());
        }
        b.extend(r.heading_deg.to_le_bytes());
        b.extend(r.length_m.to_le_bytes());
        b.extend(r.width_m.to_le_bytes());
        put_str(b, &r.surface);
    }
    b.extend((a.helipads.len() as u32).to_le_bytes());
    for h in &a.helipads {
        b.extend(h.osm_id.to_le_bytes());
        put_str(b, &h.name);
        b.extend(h.pos[0].to_le_bytes());
        b.extend(h.pos[1].to_le_bytes());
        put_ele(b, h.ele_m);
        b.push(u8::from(h.hospital) | (u8::from(h.heliport) << 1));
    }
}

/// Reads an `AERO` payload; a newer layout gives an error containing "not
/// supported".
pub fn read_aero(payload: &[u8]) -> Result<Aero, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != AERO_VERSION {
        return Err(format!("aero layout {version} not supported"));
    }
    let n = r.u32()? as usize;
    let mut aerodromes = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        aerodromes.push(Aerodrome {
            osm_id: r.i64()?,
            name: get_str(&mut r)?,
            icao: get_str(&mut r)?,
            iata: get_str(&mut r)?,
            kind: AerodromeKind::from_code(r.u8()?),
            lat_lon: (r.f64()?, r.f64()?),
            pos: [r.f32()?, r.f32()?],
            ele_m: get_ele(&mut r)?,
            outside: r.u8()? != 0,
        });
    }
    let n = r.u32()? as usize;
    let mut runways = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let a = u16::from_le_bytes(r.arr::<2>()?);
        let reference = get_str(&mut r)?;
        let ends = [[r.f32()?, r.f32()?], [r.f32()?, r.f32()?]];
        runways.push(Runway {
            aerodrome: (a != u16::MAX).then_some(a),
            reference,
            ends,
            heading_deg: r.f32()?,
            length_m: r.f32()?,
            width_m: r.f32()?,
            surface: get_str(&mut r)?,
        });
    }
    let n = r.u32()? as usize;
    let mut helipads = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let osm_id = r.i64()?;
        let name = get_str(&mut r)?;
        let pos = [r.f32()?, r.f32()?];
        let ele_m = get_ele(&mut r)?;
        let f = r.u8()?;
        helipads.push(Helipad {
            osm_id,
            name,
            pos,
            ele_m,
            hospital: f & 1 != 0,
            heliport: f & 2 != 0,
        });
    }
    r.finish("aero")?;
    Ok(Aero {
        aerodromes,
        runways,
        helipads,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_from_tags() {
        let k = |tags: &[(&str, &str)]| {
            AerodromeKind::from_osm(|key| tags.iter().find(|t| t.0 == key).map(|t| t.1))
        };
        assert_eq!(
            k(&[
                ("aeroway", "aerodrome"),
                ("aerodrome:type", "international")
            ]),
            AerodromeKind::International
        );
        assert_eq!(
            k(&[("aeroway", "aerodrome"), ("aerodrome:type", "gliding")]),
            AerodromeKind::Gliding
        );
        assert_eq!(
            k(&[("aeroway", "aerodrome"), ("sport", "gliding")]),
            AerodromeKind::Gliding
        );
        assert_eq!(k(&[("aeroway", "heliport")]), AerodromeKind::Heliport);
        assert_eq!(
            k(&[
                ("aeroway", "aerodrome"),
                ("aerodrome:type", "public"),
                ("iata", "TFN")
            ]),
            AerodromeKind::Regional
        );
        assert_eq!(k(&[("aeroway", "aerodrome")]), AerodromeKind::Airfield);
    }

    #[test]
    fn runway_heading_and_length_from_its_ends() {
        let proj = LocalProjection::new(28.48, -16.34);
        // Due east, ~1 km.
        let r = Runway::from_ends("09/27", (28.48, -16.35), (28.48, -16.34), 45.0, &proj);
        assert!((r.heading_deg - 90.0).abs() < 0.5, "{}", r.heading_deg);
        assert!((r.length_m - 978.0).abs() < 15.0, "{}", r.length_m);
        // Due north.
        let r = Runway::from_ends("18/36", (28.47, -16.34), (28.48, -16.34), 45.0, &proj);
        assert!(
            r.heading_deg < 0.5 || r.heading_deg > 359.5,
            "{}",
            r.heading_deg
        );
    }

    #[test]
    fn nearby_airports_by_distance_and_not_twice() {
        let proj = LocalProjection::new(28.40, -16.55);
        let mut a = Aero::default();
        // Tenerife's centre: both island airports, nothing of Europe.
        assert_eq!(a.add_nearby((28.27, -16.6), NEARBY_RADIUS_KM, &proj), 2);
        assert!(
            a.by_icao("GCXO")
                .is_some_and(|d| d.outside && d.iata == "TFN")
        );
        assert_eq!(a.runways_of(0).count(), 1);
        let r = a.runways_of(0).next().unwrap();
        assert!((r.length_m - 3400.0).abs() < 1.0);
        // Again: already there.
        assert_eq!(a.add_nearby((28.27, -16.6), NEARBY_RADIUS_KM, &proj), 0);
        // Liechtenstein: Zurich, Altenrhein, Friedrichshafen (and Stuttgart?
        // 210 km – no).
        let mut li = Aero::default();
        li.add_nearby(
            (47.14, 9.55),
            NEARBY_RADIUS_KM,
            &LocalProjection::new(47.14, 9.55),
        );
        let codes: Vec<&str> = li.aerodromes.iter().map(|a| a.icao.as_str()).collect();
        for c in ["LSZH", "LSZR", "EDNY"] {
            assert!(codes.contains(&c), "{codes:?}");
        }
        assert!(!codes.contains(&"GCXO"));
        // Andorra: La Seu, Barcelona, Toulouse.
        let mut ad = Aero::default();
        ad.add_nearby(
            (42.54, 1.57),
            NEARBY_RADIUS_KM,
            &LocalProjection::new(42.54, 1.57),
        );
        let codes: Vec<&str> = ad.aerodromes.iter().map(|a| a.icao.as_str()).collect();
        for c in ["LESU", "LEBL", "LFBO"] {
            assert!(codes.contains(&c), "{codes:?}");
        }
    }

    #[test]
    fn round_trip_and_newer_layouts() {
        let proj = LocalProjection::new(47.06, 9.50);
        let mut a = Aero::default();
        a.aerodromes.push(Aerodrome {
            osm_id: -42,
            name: "Flugplatz Ada".into(),
            icao: "LSXA".into(),
            iata: String::new(),
            kind: AerodromeKind::Gliding,
            lat_lon: (47.06, 9.50),
            pos: [10.0, -20.0],
            ele_m: None,
            outside: false,
        });
        let mut r = Runway::from_ends("08/26", (47.06, 9.49), (47.06, 9.51), 20.0, &proj);
        r.aerodrome = Some(0);
        r.surface = "grass".into();
        a.runways.push(r);
        a.helipads.push(Helipad {
            osm_id: 7,
            name: "Spital Bea".into(),
            pos: [100.0, 200.0],
            ele_m: Some(455.0),
            hospital: true,
            heliport: false,
        });
        a.add_nearby((47.14, 9.55), NEARBY_RADIUS_KM, &proj);
        let mut b = Vec::new();
        write_aero(&a, &mut b);
        assert_eq!(read_aero(&b).unwrap(), a);
        b[0] = 99;
        assert!(read_aero(&b).unwrap_err().contains("not supported"));
    }
}
