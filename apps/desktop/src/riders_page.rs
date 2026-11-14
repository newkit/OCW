//! Navi → Riders in this ride (spec §14 Phase 13E: "a host can see who is
//! in god mode in the rider list"; 12F: the damage dot next to riders in
//! the rider list): the session's riders as menu rows – the name, where
//! they are, a unicorn before riders flying in unicorn mode, the damage
//! dot (amber ▲ / red ■) after hurt ones. The rows come from the pure
//! `ocw_client::rider_list`; this is the Bevy glue the menu reads.

use bevy::prelude::*;
use ocw_client::rider_list;
use ocw_render::RideView;
use ocw_render::menu::RowMark;

use crate::connection::ClientState;
use crate::settings::pages::Row;

/// The list as the menu shows it, refreshed every frame.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct RiderListInfo {
    pub rows: Vec<rider_list::Row>,
}

impl RiderListInfo {
    /// The menu rows (information only: Confirm does nothing).
    pub fn menu_rows(&self) -> Vec<Row> {
        self.rows
            .iter()
            .map(|r| Row {
                label: r.label.clone(),
                value: Some(r.detail.clone()),
                left: None,
                right: None,
                confirm: None,
            })
            .collect()
    }

    /// The markers of row `i` (`None` past the rows).
    pub fn mark(&self, i: usize) -> Option<RowMark> {
        let r = self.rows.get(i)?;
        Some(RowMark {
            unicorn: r.flying,
            dot: r.dot.map(|s| (s.icon().to_owned(), s.rgb())),
        })
    }

    /// The page's status line: "5 riders, 1 flying, 1 damaged".
    pub fn summary(&self) -> String {
        rider_list::summary(&self.rows)
    }
}

pub struct RidersPagePlugin;

impl Plugin for RidersPagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderListInfo>()
            .add_systems(Update, refresh);
    }
}

fn refresh(
    view: Option<Res<RideView>>,
    client: Option<Res<ClientState>>,
    mut info: ResMut<RiderListInfo>,
) {
    let Some(view) = view else {
        return;
    };
    let entries = rider_list::entries(&view, |id| {
        client.as_ref().and_then(|c| c.0.damage_state().dot(id))
    });
    let rows = rider_list::rows(&entries, view.free_ride);
    if info.rows != rows {
        info.rows = rows;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::RiderId;
    use ocw_sim::damage::Status;

    #[test]
    fn rows_and_marks_for_the_menu() {
        let info = RiderListInfo {
            rows: vec![
                rider_list::Row {
                    id: RiderId(1),
                    label: "Nils (you)".into(),
                    detail: "km 1.2".into(),
                    flying: false,
                    dot: None,
                },
                rider_list::Row {
                    id: RiderId(2),
                    label: "Kim".into(),
                    detail: "flying, 40 m ahead".into(),
                    flying: true,
                    dot: Some(Status::Critical),
                },
            ],
        };
        let rows = info.menu_rows();
        assert_eq!(rows[1].label, "Kim");
        assert_eq!(rows[1].value.as_deref(), Some("flying, 40 m ahead"));
        assert!(rows.iter().all(|r| r.confirm.is_none()));
        assert_eq!(info.mark(0), Some(RowMark::default()));
        let m = info.mark(1).unwrap();
        assert!(m.unicorn);
        assert_eq!(m.dot, Some(("■".into(), Status::Critical.rgb())));
        assert_eq!(info.mark(2), None);
        assert_eq!(info.summary(), "2 riders, 1 flying, 1 damaged");
    }
}
