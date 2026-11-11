//! Which OSM ways are rideable by bike, and their road class (spec §4.4).

use ocw_world::graph::RoadClass;

use crate::osm::Way;

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
        "path" | "footway" | "bridleway" | "pedestrian" if bike_allowed => RoadClass::Path,
        _ => return None,
    };
    Some(class)
}

/// Carriageway width: the `width` tag if it is a plain number of metres,
/// otherwise the class default.
pub fn width_m(way: &Way, class: RoadClass) -> f64 {
    way.tag("width")
        .and_then(|w| w.trim_end_matches(" m").trim().parse::<f64>().ok())
        .filter(|w| (1.0..=30.0).contains(w))
        .unwrap_or_else(|| class.default_width_m())
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
    fn widths() {
        let w = way(&[("highway", "residential"), ("width", "4.5")]);
        assert_eq!(width_m(&w, RoadClass::Residential), 4.5);
        let w = way(&[("highway", "residential"), ("width", "3 m")]);
        assert_eq!(width_m(&w, RoadClass::Residential), 3.0);
        let w = way(&[("highway", "residential"), ("width", "narrow")]);
        assert_eq!(width_m(&w, RoadClass::Residential), 5.0);
        assert_eq!(width_m(&way(&[]), RoadClass::Primary), 7.5);
    }
}
