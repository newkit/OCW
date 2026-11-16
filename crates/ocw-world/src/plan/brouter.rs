//! The BRouter client (spec §14 Phase 16, 16E, 16H Q3: the first external
//! planner): the request URL of BRouter's HTTP API and the reading of its
//! GeoJSON answer, without the transport. `GET <server>/brouter?lonlats=
//! LON,LAT|LON,LAT…&profile=P&alternativeidx=0&format=geojson` answers a
//! `FeatureCollection` whose first feature is a `LineString` of
//! `[lon, lat, elevation]` with `track-length`, `filtered ascend` and
//! `total-time` as strings in its properties; errors come back as plain
//! text (for example "from-position not mapped in existing datafile").
//!
//! The server is a setting with **no default** (`route.planner.brouter_url`,
//! 16H Q3): without one, nothing is sent. The transport is the app's
//! ([`HttpGet`]: a native HTTP client, later the browser's `fetch`), so
//! this stays pure and is tested on answers in BRouter's format only –
//! tests never call a public server (§11.4).

use super::{PlanError, PlannedTrack, RoutePlanner};
use crate::gpx::GpxPoint;

/// The User-Agent of every request (16E: "a clear User-Agent").
pub const USER_AGENT: &str = concat!(
    "OpenCyclingWorld-Studio/",
    env!("CARGO_PKG_VERSION"),
    " (route planning; https://github.com/newkit/OCW)"
);

/// The profiles BRouter's stock install offers (the server may have more).
pub const STOCK_PROFILES: [&str; 6] = [
    "trekking", "fastbike", "safety", "shortest", "mtb", "gravel",
];

/// Where and how to ask BRouter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BRouterServer {
    /// The server's base URL (`https://host/brouter` or `http://host:17777`);
    /// empty: none set.
    pub url: String,
    /// The default profile ("trekking").
    pub profile: String,
}

/// An HTTP answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpAnswer {
    pub status: u16,
    pub body: String,
}

/// The app's HTTP GET (blocking; the studio calls it off the UI thread).
pub trait HttpGet {
    fn get(&self, url: &str, user_agent: &str) -> Result<HttpAnswer, String>;
}

/// The request URL for `via` (lat, lon) and `profile`.
pub fn request_url(server: &str, via: &[(f64, f64)], profile: &str) -> Result<String, PlanError> {
    let base = server.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(PlanError::NoServer(
            "no BRouter server is set (route.planner.brouter_url): enter one first".into(),
        ));
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(PlanError::NoServer(format!(
            "the BRouter server must be an http:// or https:// URL, not `{base}`"
        )));
    }
    if via.len() < 2 {
        return Err(PlanError::TooFewVias);
    }
    let profile = profile.trim();
    if profile.is_empty()
        || !profile
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PlanError::NoServer(format!(
            "`{profile}` is not a BRouter profile name"
        )));
    }
    let endpoint = if base.ends_with("/brouter") {
        base.to_string()
    } else {
        format!("{base}/brouter")
    };
    let lonlats: Vec<String> = via
        .iter()
        .map(|(lat, lon)| format!("{lon:.6},{lat:.6}"))
        .collect();
    Ok(format!(
        "{endpoint}?lonlats={}&profile={profile}&alternativeidx=0&format=geojson",
        lonlats.join("%7C")
    ))
}

fn number(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Reads BRouter's answer (an HTTP status and its body).
pub fn parse_answer(answer: &HttpAnswer) -> Result<PlannedTrack, PlanError> {
    let body = answer.body.trim();
    if answer.status != 200 || !body.starts_with('{') {
        let text: String = body.chars().take(300).collect();
        return Err(PlanError::Server(if text.is_empty() {
            format!("BRouter answered HTTP {}", answer.status)
        } else {
            format!("BRouter: {text}")
        }));
    }
    parse_geojson(body)
}

/// Reads BRouter's GeoJSON route.
pub fn parse_geojson(body: &str) -> Result<PlannedTrack, PlanError> {
    let bad = |m: &str| PlanError::Parse(format!("BRouter's answer: {m}"));
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| bad(&format!("not JSON ({e})")))?;
    let feature = v
        .get("features")
        .and_then(|f| f.get(0))
        .or_else(|| (v.get("type")?.as_str()? == "Feature").then_some(&v))
        .ok_or_else(|| bad("no feature"))?;
    let geometry = feature.get("geometry").ok_or_else(|| bad("no geometry"))?;
    if geometry.get("type").and_then(|t| t.as_str()) != Some("LineString") {
        return Err(bad("the geometry is not a LineString"));
    }
    let coords = geometry
        .get("coordinates")
        .and_then(|c| c.as_array())
        .ok_or_else(|| bad("no coordinates"))?;
    let mut points = Vec::with_capacity(coords.len());
    for c in coords {
        let c = c
            .as_array()
            .ok_or_else(|| bad("a coordinate is not a list"))?;
        let (Some(lon), Some(lat)) = (number(c.first()), number(c.get(1))) else {
            return Err(bad("a coordinate without longitude and latitude"));
        };
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err(bad("a coordinate out of range"));
        }
        points.push(GpxPoint {
            lat,
            lon,
            ele: number(c.get(2)),
        });
    }
    if points.len() < 2 {
        return Err(bad("fewer than two points"));
    }
    let props = feature.get("properties");
    let prop = |k: &str| number(props.and_then(|p| p.get(k)));
    Ok(PlannedTrack {
        points,
        length_m: prop("track-length"),
        ascent_m: prop("filtered ascend"),
        time_s: prop("total-time"),
        credit: "Routing: BRouter, © OpenStreetMap contributors".into(),
    })
}

/// BRouter as a [`RoutePlanner`] over the app's transport.
pub struct BRouterClient<H: HttpGet> {
    pub server: BRouterServer,
    pub http: H,
}

impl<H: HttpGet> RoutePlanner for BRouterClient<H> {
    fn id(&self) -> &str {
        "brouter"
    }
    fn credit(&self) -> String {
        "Routing: BRouter, © OpenStreetMap contributors".into()
    }
    fn is_external(&self) -> bool {
        true
    }
    fn plan(&self, via: &[(f64, f64)], profile: &str) -> Result<PlannedTrack, PlanError> {
        let profile = if profile.trim().is_empty() {
            self.server.profile.as_str()
        } else {
            profile
        };
        let url = request_url(&self.server.url, via, profile)?;
        let answer = self
            .http
            .get(&url, USER_AGENT)
            .map_err(PlanError::Network)?;
        parse_answer(&answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const ROUTE: &str = include_str!("../../tests/fixtures/brouter-grid.geojson");

    #[test]
    fn the_request_names_the_points_lon_first_and_asks_for_geojson() {
        let url = request_url(
            "https://planner.example.org/brouter/",
            &[(47.14, 9.52), (47.15, 9.53)],
            "trekking",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://planner.example.org/brouter?lonlats=9.520000,47.140000%7C9.530000,47.150000&profile=trekking&alternativeidx=0&format=geojson"
        );
        // A bare server gets the endpoint added.
        assert!(
            request_url(
                "http://localhost:17777",
                &[(1.0, 2.0), (1.0, 2.1)],
                "fastbike"
            )
            .unwrap()
            .starts_with("http://localhost:17777/brouter?lonlats=2.000000,1.000000%7C")
        );
    }

    #[test]
    fn without_a_server_nothing_is_sent() {
        for (server, profile) in [
            ("  ", "trekking"),
            ("ftp://x", "trekking"),
            ("https://x", "a&b=c"),
        ] {
            assert!(matches!(
                request_url(server, &[(1.0, 2.0), (1.0, 2.1)], profile),
                Err(PlanError::NoServer(_))
            ));
        }
        assert_eq!(
            request_url("https://x", &[(1.0, 2.0)], "trekking"),
            Err(PlanError::TooFewVias)
        );
        struct Never;
        impl HttpGet for Never {
            fn get(&self, _: &str, _: &str) -> Result<HttpAnswer, String> {
                panic!("nothing may be sent without a server");
            }
        }
        let c = BRouterClient {
            server: BRouterServer {
                url: String::new(),
                profile: "trekking".into(),
            },
            http: Never,
        };
        assert!(c.plan(&[(1.0, 2.0), (1.0, 2.1)], "").is_err());
    }

    #[test]
    fn a_route_answer_is_read() {
        let t = parse_geojson(ROUTE).unwrap();
        assert!(t.points.len() > 20);
        assert_eq!(t.length_m, Some(1240.0));
        assert_eq!(t.ascent_m, Some(12.0));
        assert_eq!(t.time_s, Some(250.0));
        assert!(t.points.iter().all(|p| p.ele.is_some()));
        assert!(t.credit.contains("BRouter"));
    }

    #[test]
    fn errors_are_said_plainly() {
        let e = parse_answer(&HttpAnswer {
            status: 400,
            body: include_str!("../../tests/fixtures/brouter-error.txt").into(),
        })
        .unwrap_err();
        assert!(e.to_string().contains("from-position not mapped"), "{e}");
        assert!(matches!(
            parse_answer(&HttpAnswer {
                status: 500,
                body: String::new()
            }),
            Err(PlanError::Server(m)) if m.contains("500")
        ));
        for bad in [
            "{",
            r#"{"type":"FeatureCollection","features":[]}"#,
            r#"{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]}}"#,
            r#"{"type":"Feature","geometry":{"type":"LineString","coordinates":[[1,2]]}}"#,
            r#"{"type":"Feature","geometry":{"type":"LineString","coordinates":[[1,95],[1,2]]}}"#,
        ] {
            assert!(
                matches!(parse_geojson(bad), Err(PlanError::Parse(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_client_sends_one_request_with_the_user_agent() {
        struct Recorded(RefCell<Vec<(String, String)>>);
        impl HttpGet for &Recorded {
            fn get(&self, url: &str, ua: &str) -> Result<HttpAnswer, String> {
                self.0.borrow_mut().push((url.into(), ua.into()));
                Ok(HttpAnswer {
                    status: 200,
                    body: ROUTE.into(),
                })
            }
        }
        let rec = Recorded(RefCell::new(Vec::new()));
        let c = BRouterClient {
            server: BRouterServer {
                url: "http://localhost:17777".into(),
                profile: "trekking".into(),
            },
            http: &rec,
        };
        let t = c.plan(&[(47.14, 9.52), (47.141, 9.53)], "").unwrap();
        assert!(t.points.len() > 20);
        let calls = rec.0.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.contains("profile=trekking"));
        assert!(calls[0].1.starts_with("OpenCyclingWorld-Studio/"));
        assert!(c.is_external());
    }
}
