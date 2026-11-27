//! The frame's ride view from the client (spec §6.3; shared by the desktop
//! and browser apps since Phase 10 B6, so both HUDs show the same): the
//! riders interpolated between the 50 Hz prediction steps, the GPS
//! position, the event, the ghost, the free ride's paths and junction,
//! the notices in their order of importance, the riding aids drawn as
//! graphics (the proximity arrow, the "last moment" warning, the camera's
//! reaction) and the damage widget. The sky and the weather are added by
//! the shared time and weather glue (`ocw_ui`).

use ocw_render::RideView;

use crate::client::ClientCore;

/// What the app adds to the view's notices.
#[derive(Debug, Clone, Copy, Default)]
pub struct Notices<'a> {
    /// The link's line ("Connecting to the server …", "Disconnected: …"):
    /// it matters more than any ride notice.
    pub link: Option<&'a str>,
    /// A ride being loaded (the desktop's ride switch).
    pub loading: Option<&'a str>,
}

/// Fills `view` from `client` at game time `now` (`elapsed_s`: the ride
/// time the HUD shows; `overstep`: the fraction of the fixed step since
/// the last prediction step; `shake`: the camera's shake setting).
pub fn fill(
    view: &mut RideView,
    client: &ClientCore,
    now: f64,
    elapsed_s: f64,
    overstep: f64,
    notices: Notices<'_>,
    shake: bool,
) {
    view.elapsed_s = elapsed_s;
    view.game_s = client.server_time_s(now);
    // Interpolate between the last two 50 Hz prediction steps.
    view.riders = client.view(now, overstep);
    // The road vehicles near the rider (spec 18E T5).
    view.vehicles = client.vehicles_view(now);
    view.vehicle_cues = client.vehicle_cues(now);
    // The parked cars near the rider (18B; static, from the package).
    view.parked = view
        .local()
        .map(|l| {
            let p = client.track().sample(l.state.distance).position;
            client.parked_view([p[0], p[2]])
        })
        .unwrap_or_default();
    // Riders off the road ride the terrain's track (spec §14 Phase 13A).
    view.off_road = client.off_road_tracks(now);
    view.position = view
        .local()
        .and_then(|l| crate::gps::position(client.track(), client.geo_origin(), &l.state));
    view.event = client.event_view(now);
    view.ghost = client.ghost_state(now);
    if client.is_free_ride() {
        view.paths = client.paths();
        view.junction = client.junction_view();
        view.notice = client.notice();
        view.route_status = client.route_status();
    }
    // A dropped connection matters more than any ride notice; a ride
    // being loaded (ride switch) shows over the ride's own notices.
    if let Some(line) = notices.link {
        view.notice = Some(line.to_owned());
    } else if let Some(line) = notices.loading {
        view.notice = Some(line.to_owned());
    } else if !client.is_free_ride() {
        view.notice = None;
    }
    // Collisions (spec §14 Phase 12): the crash line first, then the
    // proximity cue and the crash feed (in events it is in the event panel).
    view.proximity = client.proximity();
    // The aids drawn as graphics (12D) and the camera's reaction (12C).
    view.proximity_arrow = client
        .proximity_arrow()
        .or_else(|| client.vehicle_behind_arrow(now));
    view.hit_warning = client.hit_warning();
    view.camera_fx = client.camera_fx(now, shake);
    // Damage (spec §14 Phase 12F): the widget; a stop's line as a notice.
    view.damage = client.damage_widget(now);
    if notices.link.is_none() {
        if let Some(line) = client.crash_notice(now) {
            view.notice = Some(line);
        } else if let Some(line) = client.damage_notice() {
            view.notice = Some(line);
        } else if view.notice.is_none() {
            // The proximity cue is an arrow now (aids_hud), not a line.
            view.notice = if view.event.is_none() {
                client.crash_feed(now).pop()
            } else {
                None
            };
        }
    }
}
