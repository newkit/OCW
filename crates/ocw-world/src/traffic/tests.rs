//! Tag fixtures for every rule of `TRFC` (spec 18E T2).

use super::*;

fn rules_in(
    class: RoadClass,
    country: Country,
    urban: bool,
    reversed: bool,
    tags: &[(&str, &str)],
) -> EdgeRules {
    EdgeRules::from_osm(
        &EdgeContext {
            class,
            reversed,
            country,
            urban,
            width_m: class.default_width_m(),
        },
        |k| tags.iter().find(|t| t.0 == k).map(|t| t.1),
    )
}

fn rules(class: RoadClass, tags: &[(&str, &str)]) -> EdgeRules {
    rules_in(class, Country::De, false, false, tags)
}

#[test]
fn oneway_yes_minus_one_roundabout_and_bicycle_exception() {
    let r = rules(RoadClass::Residential, &[("oneway", "yes")]);
    assert_eq!(r.oneway, Oneway::Forward);
    assert!(r.allows(Vehicle::Car, true));
    assert!(!r.allows(Vehicle::Car, false));
    assert_eq!(r.lanes, [1, 0]);
    let r = rules(RoadClass::Residential, &[("oneway", "-1")]);
    assert_eq!(r.oneway, Oneway::Backward);
    assert!(r.allows(Vehicle::Car, false) && !r.allows(Vehicle::Car, true));
    assert_eq!(r.lanes, [0, 1]);
    // An edge running against its way turns `oneway=yes` round.
    let r = rules_in(
        RoadClass::Residential,
        Country::De,
        false,
        true,
        &[("oneway", "yes")],
    );
    assert_eq!(r.oneway, Oneway::Backward);
    // `oneway:bicycle=no` frees riders only, not cars.
    let r = rules(
        RoadClass::Residential,
        &[("oneway", "yes"), ("oneway:bicycle", "no")],
    );
    assert_eq!(r.oneway, Oneway::Forward);
    // Roundabouts are one-way without a tag.
    let r = rules(RoadClass::Secondary, &[("junction", "roundabout")]);
    assert!(r.roundabout);
    assert_eq!(r.oneway, Oneway::Forward);
    let r = rules(RoadClass::Tertiary, &[("junction", "circular")]);
    assert!(r.roundabout && r.oneway == Oneway::Forward);
    // `oneway=no` and plain roads.
    assert_eq!(
        rules(RoadClass::Primary, &[("oneway", "no")]).oneway,
        Oneway::No
    );
    assert_eq!(rules(RoadClass::Primary, &[]).oneway, Oneway::No);
}

#[test]
fn lanes_per_direction() {
    let r = rules(RoadClass::Primary, &[("lanes", "4")]);
    assert_eq!(r.lanes, [2, 2]);
    let r = rules(
        RoadClass::Primary,
        &[
            ("lanes", "3"),
            ("lanes:forward", "2"),
            ("lanes:backward", "1"),
        ],
    );
    assert_eq!(r.lanes, [2, 1]);
    let r = rules(
        RoadClass::Primary,
        &[("lanes", "3"), ("lanes:forward", "2")],
    );
    assert_eq!(r.lanes, [2, 1]);
    // Reversed: the way's forward lanes are the edge's backward ones.
    let r = rules_in(
        RoadClass::Primary,
        Country::De,
        false,
        true,
        &[("lanes:forward", "2"), ("lanes:backward", "1")],
    );
    assert_eq!(r.lanes, [1, 2]);
    let r = rules(RoadClass::Trunk, &[("oneway", "yes"), ("lanes", "2")]);
    assert_eq!(r.lanes, [2, 0]);
    // A single lane for both directions, and narrow untagged roads.
    assert!(rules(RoadClass::Unclassified, &[("lanes", "1")]).single_track);
    assert!(rules(RoadClass::Service, &[]).single_track);
    assert!(!rules(RoadClass::Secondary, &[]).single_track);
    assert!(!rules(RoadClass::Service, &[("oneway", "yes")]).single_track);
}

#[test]
fn maxspeed_values() {
    assert_eq!(SpeedLimit::parse("50"), Some(SpeedLimit::Kmh(50)));
    assert_eq!(SpeedLimit::parse("50 km/h"), Some(SpeedLimit::Kmh(50)));
    assert_eq!(SpeedLimit::parse("30 mph"), Some(SpeedLimit::Kmh(48)));
    assert_eq!(SpeedLimit::parse("walk"), Some(SpeedLimit::Walk));
    assert_eq!(SpeedLimit::parse("50;30"), Some(SpeedLimit::Kmh(30)));
    assert!(matches!(
        SpeedLimit::parse("none"),
        Some(SpeedLimit::Unlimited { .. })
    ));
    assert_eq!(SpeedLimit::parse("signals"), None);
    assert_eq!(SpeedLimit::parse("DE:urban"), None);
    assert_eq!(SpeedLimit::parse("0"), None);
    // `maxspeed=walk`: walking pace, a mapped limit.
    let r = rules(RoadClass::Residential, &[("maxspeed", "walk")]);
    assert_eq!(r.maxspeed, [SpeedLimit::Walk; 2]);
    assert_eq!(r.speed_source, SpeedSource::Tagged);
    assert!((r.limit(true).kmh() - WALK_KMH).abs() < 1e-6);
    // A tagged number.
    let r = rules(RoadClass::Primary, &[("maxspeed", "70")]);
    assert_eq!(r.maxspeed, [SpeedLimit::Kmh(70); 2]);
    assert_eq!(r.speed_source, SpeedSource::Tagged);
    // Per direction, turned round on a reversed edge.
    let r = rules_in(
        RoadClass::Primary,
        Country::De,
        false,
        true,
        &[("maxspeed:forward", "80"), ("maxspeed:backward", "60")],
    );
    assert_eq!(r.limit(true), SpeedLimit::Kmh(60));
    assert_eq!(r.limit(false), SpeedLimit::Kmh(80));
    // `maxspeed=none` keeps the country's advised speed.
    let r = rules(RoadClass::Trunk, &[("maxspeed", "none")]);
    assert_eq!(r.limit(true), SpeedLimit::Unlimited { advisory_kmh: 130 });
    // Codes round trip.
    for s in [
        SpeedLimit::Kmh(30),
        SpeedLimit::Walk,
        SpeedLimit::Unlimited { advisory_kmh: 130 },
    ] {
        assert_eq!(SpeedLimit::from_code(s.code()), s);
    }
}

#[test]
fn typed_limits_and_zones() {
    let r = rules(RoadClass::Secondary, &[("maxspeed", "DE:rural")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(100));
    assert_eq!(r.speed_source, SpeedSource::Typed);
    // `maxspeed:type=DE:urban` makes the road urban even off a place.
    let r = rules(RoadClass::Secondary, &[("maxspeed:type", "DE:urban")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(50));
    assert!(r.urban);
    // ... and a tagged number wins over the type.
    let r = rules(
        RoadClass::Secondary,
        &[("maxspeed", "60"), ("source:maxspeed", "DE:urban")],
    );
    assert_eq!(r.limit(true), SpeedLimit::Kmh(60));
    assert_eq!(r.speed_source, SpeedSource::Tagged);
    let r = rules(RoadClass::Residential, &[("zone:maxspeed", "DE:30")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(30));
    assert_eq!(r.speed_source, SpeedSource::Typed);
    let r = rules(RoadClass::Residential, &[("maxspeed:type", "DE:zone30")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(30));
    let r = rules(RoadClass::Residential, &[("maxspeed", "DE:zone:20")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(20));
    let r = rules(RoadClass::Residential, &[("maxspeed", "DE:bicycle_road")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(30));
    let r = rules(RoadClass::Trunk, &[("maxspeed", "CH:trunk")]);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(100));
    assert_eq!(r.country, Country::Ch, "the type names the country");
    let r = rules(RoadClass::Trunk, &[("maxspeed:type", "DE:motorway")]);
    assert_eq!(r.limit(true), SpeedLimit::Unlimited { advisory_kmh: 130 });
    assert_eq!(parse_typed("sign"), None);
    assert_eq!(parse_typed("50"), None);
}

#[test]
fn implied_limits_de_ch_li_es_ad() {
    let lim = |country, urban, class, tags: &[(&str, &str)]| {
        let r = rules_in(class, country, urban, false, tags);
        assert_eq!(r.speed_source, SpeedSource::Implied);
        r.limit(true)
    };
    use Country::*;
    use RoadClass::{Primary, Residential, Secondary};
    use SpeedLimit::{Kmh, Unlimited, Walk};
    // Germany: 50 in town, 100 outside, motorways no limit (130 advised).
    assert_eq!(lim(De, true, Secondary, &[]), Kmh(50));
    assert_eq!(lim(De, false, Secondary, &[]), Kmh(100));
    assert_eq!(
        De.implied(Setting::Motorway, 2),
        Unlimited { advisory_kmh: 130 }
    );
    assert_eq!(
        lim(De, false, Residential, &[("highway", "living_street")]),
        Walk
    );
    // Switzerland and Liechtenstein: 50 / 80 / 120.
    for c in [Ch, Li] {
        assert_eq!(lim(c, true, Primary, &[]), Kmh(50));
        assert_eq!(lim(c, false, Primary, &[]), Kmh(80));
        assert_eq!(c.implied(Setting::Motorway, 2), Kmh(120));
        assert_eq!(
            lim(c, false, Residential, &[("highway", "living_street")]),
            Kmh(20)
        );
    }
    // Spain: 90 outside, 120 on motorways; in town 50 with two lanes per
    // direction, 30 with one (RD 970/2020).
    assert_eq!(lim(Es, false, Primary, &[]), Kmh(90));
    assert_eq!(Es.implied(Setting::Motorway, 2), Kmh(120));
    assert_eq!(lim(Es, true, Primary, &[("lanes", "4")]), Kmh(50));
    assert_eq!(lim(Es, true, Primary, &[]), Kmh(30));
    // Andorra: 50 in town, 90 outside.
    assert_eq!(lim(Ad, true, Secondary, &[]), Kmh(50));
    assert_eq!(lim(Ad, false, Secondary, &[]), Kmh(90));
    // Residential streets count as in town everywhere.
    assert_eq!(lim(De, false, Residential, &[]), Kmh(50));
    // Unknown country: generic limits, marked as such.
    let r = rules_in(Secondary, Other, false, false, &[]);
    assert_eq!(r.limit(true), Kmh(90));
    assert_eq!(r.speed_source, SpeedSource::Default);
}

#[test]
fn countries_from_codes_and_zones() {
    assert_eq!(Country::from_code("li"), Country::Li);
    assert_eq!(Country::from_code("ES-CN"), Country::Es);
    assert_eq!(Country::from_code("DE-BW"), Country::De);
    assert_eq!(Country::from_code("XX"), Country::Other);
    assert_eq!(Country::from_time_zone("Atlantic/Canary"), Country::Es);
    assert_eq!(Country::from_time_zone("Europe/Vaduz"), Country::Li);
    assert_eq!(Country::from_time_zone("Europe/Andorra"), Country::Ad);
    assert_eq!(Country::from_time_zone("Europe/Berlin"), Country::De);
    for c in Country::ALL {
        assert_eq!(Country::from_code(c.code()), c);
    }
}

#[test]
fn vehicle_access() {
    // hgv=no: no lorries, cars yes.
    let r = rules(RoadClass::Tertiary, &[("hgv", "no")]);
    assert!(!r.access.allows(Vehicle::Hgv));
    assert!(r.access.allows(Vehicle::Car) && r.access.allows(Vehicle::Bus));
    // motor_vehicle=no: nothing motorised …
    let r = rules(RoadClass::Residential, &[("motor_vehicle", "no")]);
    assert!(!r.has_traffic());
    // … except where buses are let through.
    let r = rules(
        RoadClass::Residential,
        &[("motor_vehicle", "no"), ("bus", "yes")],
    );
    assert_eq!(r.access, Access(Access::BUS));
    let r = rules(RoadClass::Primary, &[("access", "no"), ("psv", "yes")]);
    assert_eq!(r.access, Access(Access::BUS));
    // Cycleways, paths and pedestrian streets carry no traffic.
    assert!(!rules(RoadClass::Cycleway, &[]).has_traffic());
    assert!(!rules(RoadClass::Path, &[]).has_traffic());
    assert!(!rules(RoadClass::Residential, &[("highway", "pedestrian")]).has_traffic());
    // Tracks: tractors only, unless opened to cars.
    let r = rules(RoadClass::Track, &[("tracktype", "grade2")]);
    assert_eq!(r.access, Access(Access::TRACTOR));
    let r = rules(RoadClass::Track, &[("motor_vehicle", "yes")]);
    assert!(r.access.allows(Vehicle::Car));
    // Farm roads: "agricultural;forestry".
    let r = rules(
        RoadClass::Unclassified,
        &[("motor_vehicle", "agricultural;forestry")],
    );
    assert_eq!(r.access, Access(Access::TRACTOR));
    // Destination only.
    let r = rules(RoadClass::Residential, &[("motor_vehicle", "destination")]);
    assert!(r.access.allows(Vehicle::Car) && r.destination_only);
    // Weight and height.
    let r = rules(
        RoadClass::Tertiary,
        &[("maxweight", "7.5"), ("maxheight", "3.8 m")],
    );
    assert_eq!(r.maxweight_t, Some(7.5));
    assert_eq!(r.maxheight_m, Some(3.8));
    assert_eq!(
        rules(RoadClass::Tertiary, &[("maxweight", "3500 kg")]).maxweight_t,
        Some(3.5)
    );
    assert_eq!(
        rules(RoadClass::Tertiary, &[("maxheight", "default")]).maxheight_m,
        None
    );
}

#[test]
fn parking_both_sides_and_schemes() {
    // parking:both.
    let r = rules(
        RoadClass::Residential,
        &[
            ("parking:both", "lane"),
            ("parking:both:orientation", "parallel"),
        ],
    );
    assert!(
        r.parking
            .iter()
            .all(|p| p.place == ParkingPlace::Lane && p.has_cars())
    );
    // One side, the other forbidden; turned round on a reversed edge.
    let tags = [
        ("parking:left", "half_on_kerb"),
        ("parking:left:orientation", "diagonal"),
        ("parking:right", "no"),
    ];
    let r = rules(RoadClass::Residential, &tags);
    assert_eq!(
        r.parking[0],
        Parking {
            place: ParkingPlace::HalfOnKerb,
            orientation: ParkingOrientation::Diagonal
        }
    );
    assert_eq!(r.parking[1].place, ParkingPlace::No);
    assert_eq!(r.kerb_parking(true).place, ParkingPlace::No);
    let r = rules_in(RoadClass::Residential, Country::De, false, true, &tags);
    assert_eq!(r.parking[1].place, ParkingPlace::HalfOnKerb);
    assert_eq!(r.parking[0].place, ParkingPlace::No);
    // The older parking:lane scheme.
    let r = rules(
        RoadClass::Residential,
        &[
            ("parking:lane:both", "perpendicular"),
            ("parking:lane:both:perpendicular", "street_side"),
        ],
    );
    assert_eq!(
        r.parking[0],
        Parking {
            place: ParkingPlace::StreetSide,
            orientation: ParkingOrientation::Perpendicular
        }
    );
    let r = rules(
        RoadClass::Residential,
        &[("parking:lane:right", "no_stopping")],
    );
    assert_eq!(r.parking[1].place, ParkingPlace::No);
    assert_eq!(r.parking[0], Parking::UNKNOWN);
    for p in [
        Parking::UNKNOWN,
        Parking {
            place: ParkingPlace::OnKerb,
            orientation: ParkingOrientation::Perpendicular,
        },
    ] {
        assert_eq!(Parking::from_code(p.code()), p);
    }
}

#[test]
fn overtaking_and_driving_guess() {
    let r = rules(RoadClass::Secondary, &[("overtaking", "no")]);
    assert_eq!(r.no_overtaking, [true, true]);
    let r = rules(RoadClass::Secondary, &[("overtaking:forward", "no")]);
    assert_eq!(r.no_overtaking, [true, false]);
    let r = rules(RoadClass::Service, &[]);
    assert_eq!(r.target_kmh(RoadClass::Service, true), 30.0);
    let r = rules(RoadClass::Trunk, &[("maxspeed", "80")]);
    assert_eq!(r.target_kmh(RoadClass::Trunk, true), 80.0);
}

fn sample() -> TrafficData {
    let mut t = TrafficData {
        country: Country::Li,
        ..Default::default()
    };
    t.edges.insert(
        EdgeId(5),
        rules(
            RoadClass::Residential,
            &[
                ("oneway", "-1"),
                ("maxspeed", "30"),
                ("parking:both", "lane"),
                ("maxweight", "7.5"),
                ("maxheight", "3.2"),
                ("motor_vehicle", "destination"),
                ("overtaking", "no"),
            ],
        ),
    );
    let mut r = rules(RoadClass::Secondary, &[("junction", "roundabout")]);
    r.flow = 1.25;
    t.edges.insert(EdgeId(3), r);
    t.edges.insert(EdgeId(9), rules(RoadClass::Track, &[]));
    t.controls = vec![
        NodeControl {
            edge: EdgeId(5),
            along_m: 40.0,
            kind: ControlKind::CrossingSignals,
            applies: Applies::Both,
            at_junction: false,
            osm_node: 11,
        },
        NodeControl {
            edge: EdgeId(3),
            along_m: 120.0,
            kind: ControlKind::GiveWay,
            applies: Applies::Forward,
            at_junction: true,
            osm_node: 12,
        },
        NodeControl {
            edge: EdgeId(5),
            along_m: 2.0,
            kind: ControlKind::Stop,
            applies: Applies::Backward,
            at_junction: true,
            osm_node: 13,
        },
    ];
    t.sort();
    t.stops = vec![BusStop {
        osm_node: 21,
        pos: [10.0, 20.0],
        name: "Ada Platz".into(),
        edge: Some(EdgeId(5)),
        along_m: 33.0,
        side: Side::Right,
    }];
    t.bus_routes = vec![BusRoute {
        osm_relation: 31,
        reference: "12".into(),
        name: "Bus 12: Ada Platz – Bea Dorf".into(),
        legs: vec![(EdgeId(5), false), (EdgeId(3), true)],
        stops: vec![0],
    }];
    t.parking = vec![ParkingArea {
        osm_id: -41,
        pos: [50.0, 60.0],
        capacity: 120,
        estimated: true,
        public: true,
        kind: ParkingKind::MultiStorey,
        edge: None,
        along_m: 0.0,
    }];
    t
}

#[test]
fn round_trip_and_lookups() {
    let t = sample();
    let mut b = Vec::new();
    write_traffic(&t, &mut b);
    let back = read_traffic(&b).unwrap();
    assert_eq!(back, t);
    let on5: Vec<f32> = back
        .controls_on(EdgeId(5))
        .iter()
        .map(|c| c.along_m)
        .collect();
    assert_eq!(on5, vec![2.0, 40.0]);
    assert_eq!(back.controls_on(EdgeId(3)).len(), 1);
    assert!(back.controls_on(EdgeId(4)).is_empty());
    // A newer layout is "not supported" (skipped by the package reader).
    b[0] = 99;
    assert!(read_traffic(&b).unwrap_err().contains("not supported"));
}

#[test]
fn rules_without_data_follow_the_class() {
    let rr = RoadRules {
        data: None,
        country: Country::Es,
    };
    let edge = |class| Edge {
        id: EdgeId(1),
        from: crate::graph::VertexId(0),
        to: crate::graph::VertexId(1),
        points: vec![[0.0, 0.0], [100.0, 0.0]],
        class,
        name: None,
        width_m: 6.0,
        bridge: false,
        tunnel: false,
        dead_end_m: None,
        boundary: false,
        oneway: Oneway::No,
        osm_way: 1,
        profile: Vec::new(),
    };
    assert_eq!(
        rr.edge(&edge(RoadClass::Primary)).limit(true),
        SpeedLimit::Kmh(90)
    );
    assert!(!rr.edge(&edge(RoadClass::Cycleway)).has_traffic());
    assert!(rr.controls_on(EdgeId(1)).is_empty());
}
