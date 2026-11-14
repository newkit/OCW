//! Navi → Riders in this ride (spec §14 Phase 13E: "a host can see who is
//! in god mode in the rider list"; 12F: the damage dot next to riders in
//! the rider list): the session's riders as menu rows – the name, where
//! they are, a unicorn before riders flying in unicorn mode, the damage
//! dot (amber ▲ / red ■) after hurt ones. The rows come from the pure
//! `ocw_client::rider_list`; this is the Bevy glue the menu reads (both
//! apps, since Phase 10 B6).

use bevy::prelude::*;
use ocw_client::rider_list::{self, RiderListInfo};
use ocw_render::RideView;

/// The list as the menu shows it, refreshed every frame.
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct RiderList(pub RiderListInfo);

pub struct RidersPagePlugin;

impl Plugin for RidersPagePlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<RiderList>()
            .add_systems(Update, refresh.in_set(crate::ViewExtrasSet));
    }
}

fn refresh(world: &mut World) {
    let rows = {
        let Some(view) = world.get_resource::<RideView>() else {
            return;
        };
        let client = crate::client(world);
        let entries = rider_list::entries(view, |id| client.and_then(|c| c.damage_state().dot(id)));
        rider_list::rows(&entries, view.free_ride)
    };
    let mut info = world.resource_mut::<RiderList>();
    if info.rows != rows {
        info.rows = rows;
    }
}
