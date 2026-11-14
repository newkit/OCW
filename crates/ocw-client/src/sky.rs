//! The sky of the ride on the client (spec §14 Phase 9): the time zone of
//! the ridden place, the HUD clock, and the [`SkyView`] for the renderer,
//! from the server's world clock as [`ClientCore`] predicts it. Shared by
//! the desktop's time machine (`apps/desktop/src/timemachine.rs`) and the
//! browser app.

use ocw_render::sky::SkyView;
use ocw_sky::{ClockMode, Sky, Zone};

use crate::client::ClientCore;

/// The zone of a ride: the package's zone if it has a valid one, else the
/// zone of its geographic origin, else the default (Vaduz).
pub fn place_zone(package: Option<&str>, origin: Option<(f64, f64)>) -> (Zone, &'static str) {
    if let Some(z) = package.and_then(|id| Zone::get(id).ok()) {
        return (z, "package");
    }
    if let Some((lat, lon)) = origin
        && let Ok(z) = Zone::get(&ocw_sky::zone::zone_for(lat, lon))
    {
        return (z, "origin");
    }
    (
        Zone::get(ocw_sky::zone::DEFAULT_ZONE).unwrap_or_else(|_| Zone::utc()),
        "default",
    )
}

/// The HUD clock: the glyph and the local time, then the mode when the
/// time is not running normally ("x600", "paused", "fixed", "+2 h").
pub fn hud_label(sky: &Sky, clock: Option<&ocw_protocol::ClockInfo>) -> String {
    let base = sky.hud_clock();
    let Some(c) = clock else { return base };
    let extra = if c.paused {
        "paused".to_owned()
    } else if c.speed != 1.0 {
        ocw_sky::clock::speed_label(c.speed, false)
    } else {
        match c.mode {
            ClockMode::Real => return base,
            ClockMode::Fixed => "fixed".into(),
            ClockMode::Offset => {
                let h = c.offset_ms as f64 / 3_600_000.0;
                if h.fract().abs() < 1e-6 {
                    format!("{h:+.0} h")
                } else {
                    format!("{h:+.1} h")
                }
            }
        }
    };
    format!("{base} {extra}")
}

/// The sky at the local rider at game time `now_s`: in `zone`, at the
/// rider's GPS `position` (else the ride's origin, else the default place);
/// `None` until the server's clock is known.
pub fn sky_view(
    client: &mut ClientCore,
    now_s: f64,
    zone: &Zone,
    position: Option<(f64, f64, f64)>,
    night_floor: f32,
) -> Option<SkyView> {
    let ms = client.world_ms(now_s)?;
    let latlon = position
        .map(|p| (p.0, p.1))
        .or(client.geo_origin())
        .unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
    let sky = Sky::at(ms, zone, latlon, night_floor);
    let hud = hud_label(&sky, client.clock_info());
    Some(SkyView { sky, hud })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::RiderProfile;
    use ocw_server::local::SteppedServer;
    use ocw_world::Route;

    #[test]
    fn test_routes_are_in_the_default_zone() {
        let (zone, source) = place_zone(None, None);
        assert_eq!(source, "default");
        assert_eq!(zone.id(), ocw_sky::zone::DEFAULT_ZONE);
        let (zone, source) = place_zone(Some("Europe/Zurich"), None);
        assert_eq!((zone.id(), source), ("Europe/Zurich", "package"));
    }

    #[test]
    fn the_sky_appears_once_the_servers_clock_is_known() {
        let mut client = ClientCore::new(Route::test_loop(), "Sky", RiderProfile::default());
        let (zone, _) = place_zone(None, None);
        assert!(sky_view(&mut client, 0.0, &zone, None, 0.1).is_none());
        let mut server = SteppedServer::new(Route::test_loop(), 0, 0.0);
        server.send(&client.hello());
        for bytes in server.drain() {
            client.on_server_message(&bytes, 0.0);
        }
        let view = sky_view(&mut client, 0.0, &zone, None, 0.1).expect("clock known");
        assert!(!view.hud.is_empty());
    }
}
