//! Which OSM ways are rideable by bike, and their road class (spec §4.4).

use ocw_world::graph::{Oneway, RoadClass};

use crate::osm::{OsmData, Way};

/// Road class of a rideable way, or `None` if bikes cannot or should not
/// use it (motorways, private roads, driveways, footpaths without bicycle
/// access, areas, construction).
pub fn rideable(way: &Way) -> Option<RoadClass> {
    let bike = way.tag("bicycle");
    let bike_allowed = matches!(bike, Some("yes" | "designated" | "permissive"));
    if matches!(bike, Some("no" | "dismount" | "use_sidepath")) {
        return None;
    }
    if way.tag("area") == Some("yes") {
        return None;
    }
    if matches!(way.tag("access"), Some("no" | "private")) && !bike_allowed {
        return None;
    }
    let class = match way.tag("highway")? {
        "trunk" | "trunk_link" => RoadClass::Trunk,
        "primary" | "primary_link" => RoadClass::Primary,
        "secondary" | "secondary_link" => RoadClass::Secondary,
        "tertiary" | "tertiary_link" => RoadClass::Tertiary,
        "unclassified" => RoadClass::Unclassified,
        "residential" | "living_street" => RoadClass::Residential,
        "service" => {
            // Driveways and parking aisles are left out entirely (spec §4.4).
            if matches!(
                way.tag("service"),
                Some("driveway" | "parking_aisle" | "drive-through")
            ) {
                return None;
            }
            RoadClass::Service
        }
        "track" => RoadClass::Track,
        "cycleway" => RoadClass::Cycleway,
        // Pedestrian streets open to bikes are paved town streets as wide
        // as residential ones (Freiburg's old town), not gravel paths.
        "pedestrian" if bike_allowed => RoadClass::Residential,
        "path" | "footway" | "bridleway" if bike_allowed => RoadClass::Path,
        _ => return None,
    };
    Some(class)
}

/// One-way restriction for bicycles: `oneway=yes|true|1|-1`, roundabouts
/// (implied one-way); `oneway:bicycle=no` (contraflow cycling) lifts it.
pub fn oneway(way: &Way) -> Oneway {
    if matches!(way.tag("oneway:bicycle"), Some("no" | "false" | "0")) {
        return Oneway::No;
    }
    match way.tag("oneway") {
        Some("yes" | "true" | "1") => Oneway::Forward,
        Some("-1" | "reverse") => Oneway::Backward,
        Some(_) => Oneway::No,
        None if matches!(way.tag("junction"), Some("roundabout" | "circular")) => Oneway::Forward,
        None => Oneway::No,
    }
}

/// Carriageway width: the `width` tag if it is a plain number of metres,
/// otherwise the class default.
pub fn width_m(way: &Way, class: RoadClass) -> f64 {
    way.tag("width")
        .and_then(|w| w.trim_end_matches(" m").trim().parse::<f64>().ok())
        .filter(|w| (1.0..=30.0).contains(w))
        .unwrap_or_else(|| class.default_width_m())
}

/// The look of a rideable way (spec §4.4, Phase 8 T2): OSM `surface`,
/// `smoothness`, `tracktype`, else the category.
pub fn surface(way: &Way, class: RoadClass) -> ocw_world::RoadSurface {
    ocw_world::RoadSurface::from_osm(class, |k| way.tag(k))
}

/// The look of every rideable way of `osm` whose surface differs from its
/// category's default, by OSM way id (for
/// [`crate::package::apply_road_surfaces`]).
pub fn way_surfaces(osm: &OsmData) -> std::collections::HashMap<i64, ocw_world::RoadSurface> {
    osm.ways
        .iter()
        .filter_map(|w| {
            let class = rideable(w)?;
            let s = surface(w, class);
            (s != ocw_world::RoadSurface::of_class(class)).then_some((w.id, s))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn way(tags: &[(&str, &str)]) -> Way {
        Way {
            id: 1,
            nodes: vec![1, 2],
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn rideable_roads_and_classes() {
        assert_eq!(
            rideable(&way(&[("highway", "primary")])),
            Some(RoadClass::Primary)
        );
        assert_eq!(
            rideable(&way(&[("highway", "secondary_link")])),
            Some(RoadClass::Secondary)
        );
        assert_eq!(
            rideable(&way(&[("highway", "living_street")])),
            Some(RoadClass::Residential)
        );
        assert_eq!(
            rideable(&way(&[("highway", "cycleway")])),
            Some(RoadClass::Cycleway)
        );
        assert_eq!(
            rideable(&way(&[("highway", "track")])),
            Some(RoadClass::Track)
        );
        assert_eq!(
            rideable(&way(&[("highway", "service")])),
            Some(RoadClass::Service)
        );
    }

    #[test]
    fn excluded_ways() {
        assert_eq!(rideable(&way(&[("highway", "motorway")])), None);
        assert_eq!(
            rideable(&way(&[("highway", "service"), ("service", "driveway")])),
            None
        );
        assert_eq!(
            rideable(&way(&[
                ("highway", "service"),
                ("service", "parking_aisle")
            ])),
            None
        );
        assert_eq!(
            rideable(&way(&[("highway", "residential"), ("access", "private")])),
            None
        );
        assert_eq!(
            rideable(&way(&[("highway", "primary"), ("bicycle", "no")])),
            None
        );
        assert_eq!(rideable(&way(&[("highway", "footway")])), None);
        assert_eq!(
            rideable(&way(&[("highway", "pedestrian"), ("area", "yes")])),
            None
        );
        assert_eq!(rideable(&way(&[("highway", "construction")])), None);
        assert_eq!(rideable(&way(&[("building", "yes")])), None);
    }

    /// Regression (Black Forest, Freiburg): the old town's pedestrian
    /// streets (bicycle=yes) were narrow gravel paths ("unpaved").
    #[test]
    fn pedestrian_streets_open_to_bikes_are_paved_town_streets() {
        assert_eq!(
            rideable(&way(&[("highway", "pedestrian"), ("bicycle", "yes")])),
            Some(RoadClass::Residential)
        );
        assert_eq!(
            ocw_world::Surface::of_class(RoadClass::Residential),
            ocw_world::Surface::Asphalt
        );
        assert_eq!(rideable(&way(&[("highway", "pedestrian")])), None);
    }

    #[test]
    fn bicycle_access_opens_paths_and_private_roads() {
        assert_eq!(
            rideable(&way(&[("highway", "footway"), ("bicycle", "yes")])),
            Some(RoadClass::Path)
        );
        assert_eq!(
            rideable(&way(&[("highway", "path"), ("bicycle", "designated")])),
            Some(RoadClass::Path)
        );
        assert_eq!(
            rideable(&way(&[
                ("highway", "track"),
                ("access", "no"),
                ("bicycle", "yes")
            ])),
            Some(RoadClass::Track)
        );
    }

    #[test]
    fn oneway_for_bicycles() {
        assert_eq!(oneway(&way(&[("highway", "primary")])), Oneway::No);
        assert_eq!(
            oneway(&way(&[("highway", "primary"), ("oneway", "yes")])),
            Oneway::Forward
        );
        assert_eq!(
            oneway(&way(&[("highway", "primary"), ("oneway", "-1")])),
            Oneway::Backward
        );
        assert_eq!(
            oneway(&way(&[("highway", "primary"), ("oneway", "no")])),
            Oneway::No
        );
        assert_eq!(
            oneway(&way(&[("highway", "tertiary"), ("junction", "roundabout")])),
            Oneway::Forward
        );
        // Contraflow cycling allowed.
        assert_eq!(
            oneway(&way(&[
                ("highway", "residential"),
                ("oneway", "yes"),
                ("oneway:bicycle", "no")
            ])),
            Oneway::No
        );
    }

    #[test]
    fn widths() {
        let w = way(&[("highway", "residential"), ("width", "4.5")]);
        assert_eq!(width_m(&w, RoadClass::Residential), 4.5);
        let w = way(&[("highway", "residential"), ("width", "3 m")]);
        assert_eq!(width_m(&w, RoadClass::Residential), 3.0);
        let w = way(&[("highway", "residential"), ("width", "narrow")]);
        assert_eq!(width_m(&w, RoadClass::Residential), 5.0);
        assert_eq!(width_m(&way(&[]), RoadClass::Primary), 7.5);
    }

    #[test]
    fn road_surfaces_of_ways() {
        use ocw_world::RoadSurface;
        let w = way(&[("highway", "residential"), ("surface", "sett")]);
        assert_eq!(surface(&w, RoadClass::Residential), RoadSurface::Paving);
        let w = way(&[("highway", "track"), ("tracktype", "grade1")]);
        assert_eq!(surface(&w, RoadClass::Track), RoadSurface::WornAsphalt);
        let mut osm = OsmData::default();
        let mut add = |id: i64, tags: &[(&str, &str)]| {
            let mut w = way(tags);
            w.id = id;
            osm.ways.push(w);
        };
        add(1, &[("highway", "residential"), ("surface", "paving_stones")]);
        add(2, &[("highway", "residential"), ("surface", "asphalt")]);
        add(3, &[("highway", "track"), ("surface", "dirt")]);
        add(4, &[("highway", "motorway"), ("surface", "dirt")]);
        add(5, &[("highway", "track")]);
        let m = way_surfaces(&osm);
        // Only rideable ways that differ from their category's default.
        assert_eq!(m.len(), 2, "{m:?}");
        assert_eq!(m[&1], RoadSurface::Paving);
        assert_eq!(m[&3], RoadSurface::Dirt);
    }
}
