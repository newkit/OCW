//! The Settings and Workshop pages of the in-game menu (spec §3.2.1, M2)
//! as a Bevy-free model: each page is a list of rows; a row shows a label
//! and a value and says which console command line ← / → (change the
//! value) and Confirm run, or which page Confirm opens. Menu, console and
//! scripts therefore share one code path, and each row's command is
//! testable without Bevy.
//!
//! Integration point for the menu shell (M1): a menu page for Settings →
//! Controls / Trainer / Strava / Display / Riders and for the Workshop
//! renders [`rows`] for its [`Page`], maps the abstract actions Left /
//! Right / Confirm onto [`Row::left`] / [`Row::right`] / [`Row::confirm`]
//! and queues the command lines with `ConsoleState::queue`; `menu::model`
//! does this ([`crate::menu::model::Page::Sub`]).

use super::registry::{Kind, Settings, def};
use super::riders::Riders;

/// A page of the Settings or Workshop menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Controls,
    Trainer,
    Strava,
    Display,
    Riders,
    /// Editing one rider (index into the profiles).
    Rider(usize),
    Workshop,
}

impl Page {
    /// The pages with a tab (Settings sub pages and the Workshop).
    pub const TABS: [Page; 6] = [
        Page::Workshop,
        Page::Riders,
        Page::Controls,
        Page::Trainer,
        Page::Display,
        Page::Strava,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Controls => "Controls",
            Self::Trainer => "Trainer",
            Self::Strava => "Strava",
            Self::Display => "Display",
            Self::Riders => "Riders",
            Self::Rider(_) => "Rider",
            Self::Workshop => "Workshop",
        }
    }

    /// One page up (the rider editor goes back to the list).
    pub fn parent(self) -> Option<Self> {
        match self {
            Self::Rider(_) => Some(Self::Riders),
            _ => None,
        }
    }

    /// Short explanation shown above the rows.
    pub fn notes(self) -> &'static [&'static str] {
        match self {
            Self::Workshop => &[
                "Choose who rides: their weight goes into the physics and the trainer (from the next start). Bike and character: Phase 4.",
            ],
            Self::Riders => &["Rider profiles: weight, height, FTP. Choose one in the Workshop."],
            Self::Rider(_) => &["Changes apply from the next start (Restart)."],
            Self::Trainer => &["Devices, SIM difficulty and the virtual trainer."],
            Self::Strava => &["Uploads rides to Strava (needs API credentials, see README)."],
            Self::Controls | Self::Display => &[],
        }
    }
}

/// What Confirm does on a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activate {
    /// Run a console command line.
    Command(String),
    /// Open a page.
    Open(Page),
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    /// The current value, if the row has one.
    pub value: Option<String>,
    /// Command line for ← (one step down) and → (one step up).
    pub left: Option<String>,
    pub right: Option<String>,
    pub confirm: Option<Activate>,
}

impl Row {
    fn action(label: &str, line: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::Command(line.into())),
        }
    }
}

/// A row for a registered setting: ← / → step it (`set KEY VALUE`);
/// Confirm toggles a switch or moves a choice on.
pub fn setting_row(settings: &Settings, key: &str, label: &str) -> Row {
    let d = def(key).expect("a registered setting");
    let v = settings.get(key).expect("registered");
    let set = |dir| format!("set {key} {}", d.step(&v, dir));
    let confirm = match d.kind {
        Kind::Bool | Kind::Choice(_) => Some(Activate::Command(set(1))),
        Kind::Int { .. } => None,
    };
    Row {
        label: label.into(),
        value: Some(v.to_string()),
        left: Some(set(-1)),
        right: Some(set(1)),
        confirm,
    }
}

/// A rider edit command line: `riders.edit "NAME" ARGS`.
fn edit(name: &str, args: &str) -> String {
    format!("riders.edit \"{name}\" {args}")
}

/// A number field of the rider editor: ← / → by `step` within the range;
/// unset, they start from `start`.
fn number_row(
    label: &str,
    value: Option<f64>,
    unit: &str,
    (step, start, min, max): (f64, f64, f64, f64),
    line: impl Fn(f64) -> String,
) -> Row {
    let base = value.unwrap_or(start);
    let to = |dir: f64| {
        let v = if value.is_none() {
            base
        } else {
            (base + dir * step).clamp(min, max)
        };
        line((v * 10.0).round() / 10.0)
    };
    Row {
        label: label.into(),
        value: Some(value.map_or("-".into(), |v| {
            format!("{} {unit}", (v * 10.0).round() / 10.0)
        })),
        left: Some(to(-1.0)),
        right: Some(to(1.0)),
        confirm: None,
    }
}

/// The rows of `page`.
pub fn rows(page: Page, settings: &Settings, riders: &Riders) -> Vec<Row> {
    match page {
        Page::Controls => vec![setting_row(
            settings,
            "controls.autodrive",
            "Autodrive at the start",
        )],
        Page::Trainer => vec![
            setting_row(settings, "trainer.difficulty", "Difficulty (%)"),
            setting_row(
                settings,
                "trainer.virtual_power",
                "Virtual trainer power (W)",
            ),
            Row::action("Scan for devices", "devices.scan"),
            Row::action("Show devices", "devices.list"),
        ],
        Page::Strava => vec![
            Row::action("Status", "strava.status"),
            Row::action("Link account", "strava.connect"),
            Row::action("Sync now", "strava.sync"),
            Row::action("Retry failed uploads", "strava.sync --retry"),
            Row::action("Unlink", "strava.disconnect"),
        ],
        Page::Display => vec![
            setting_row(settings, "display.hud", "HUD"),
            setting_row(settings, "display.units", "Units"),
            setting_row(settings, "display.camera", "Camera"),
            setting_row(settings, "display.fps", "FPS counter"),
            setting_row(settings, "display.fullscreen", "Fullscreen (next start)"),
        ],
        Page::Riders => {
            let mut out: Vec<Row> = riders
                .riders
                .iter()
                .enumerate()
                .map(|(i, r)| Row {
                    label: r.name.clone(),
                    value: Some(r.summary()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Open(Page::Rider(i))),
                })
                .collect();
            out.push(Row::action(
                "Add a rider",
                format!("riders.add \"{}\"", riders.new_name()),
            ));
            out
        }
        Page::Rider(i) => {
            let Some(r) = riders.riders.get(i) else {
                return Vec::new();
            };
            let n = &r.name;
            let (hmin, hmax) = super::riders::HEIGHT_RANGE_CM;
            let (fmin, fmax) = super::riders::FTP_RANGE_W;
            vec![
                Row {
                    label: "Name".into(),
                    value: Some(n.clone()),
                    left: None,
                    right: None,
                    confirm: None,
                },
                number_row(
                    "Weight",
                    Some(r.weight_kg),
                    "kg",
                    (
                        1.0,
                        75.0,
                        ocw_core::MIN_RIDER_WEIGHT_KG,
                        ocw_core::MAX_RIDER_WEIGHT_KG,
                    ),
                    |v| edit(n, &format!("--weight {v}")),
                ),
                number_row("Height", r.height_cm, "cm", (1.0, 175.0, hmin, hmax), |v| {
                    edit(n, &format!("--height {v}"))
                }),
                number_row(
                    "FTP",
                    r.ftp_w.map(f64::from),
                    "W",
                    (5.0, 200.0, f64::from(fmin), f64::from(fmax)),
                    |v| edit(n, &format!("--ftp {v}")),
                ),
                Row::action("Ride as this rider", format!("riders.select \"{n}\"")),
                Row::action("Delete this rider", format!("riders.remove \"{n}\"")),
            ]
        }
        Page::Workshop => {
            let mut out: Vec<Row> = riders
                .riders
                .iter()
                .map(|r| Row {
                    label: r.name.clone(),
                    value: Some(if riders.is_active(&r.name) {
                        format!("{} - riding", r.summary())
                    } else {
                        r.summary()
                    }),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(format!("riders.select \"{}\"", r.name))),
                })
                .collect();
            if out.is_empty() {
                let d = super::riders::Rider::default();
                out.push(Row {
                    label: d.name.clone(),
                    value: Some(format!("{} - riding (default)", d.summary())),
                    left: None,
                    right: None,
                    confirm: None,
                });
            }
            out.push(Row {
                label: "Edit riders".into(),
                value: None,
                left: None,
                right: None,
                confirm: Some(Activate::Open(Page::Riders)),
            });
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::riders::Rider;
    use super::*;

    fn riders() -> Riders {
        let mut r = Riders::default();
        r.add(Rider {
            name: "Alex".into(),
            weight_kg: 78.0,
            height_cm: None,
            ftp_w: Some(250),
        })
        .unwrap();
        r.add(Rider {
            name: "Anna".into(),
            weight_kg: 61.5,
            ..Rider::default()
        })
        .unwrap();
        r
    }

    fn row<'a>(rows: &'a [Row], label: &str) -> &'a Row {
        rows.iter().find(|r| r.label == label).expect(label)
    }

    fn cmd(line: &str) -> Option<Activate> {
        Some(Activate::Command(line.into()))
    }

    #[test]
    fn setting_rows_run_set_commands() {
        let (s, _) = Settings::parse("[display]\ncamera = \"side\"\n").unwrap();
        let display = rows(Page::Display, &s, &Riders::default());
        let hud = row(&display, "HUD");
        assert_eq!(hud.value.as_deref(), Some("on"));
        assert_eq!(hud.confirm, cmd("set display.hud off"));
        assert_eq!(hud.left.as_deref(), Some("set display.hud off"));
        let cam = row(&display, "Camera");
        assert_eq!(cam.value.as_deref(), Some("side"));
        assert_eq!(cam.right.as_deref(), Some("set display.camera top"));
        assert_eq!(cam.left.as_deref(), Some("set display.camera chase"));
        assert_eq!(
            row(&display, "Units").confirm,
            cmd("set display.units imperial")
        );
        let trainer = rows(Page::Trainer, &s, &Riders::default());
        let d = row(&trainer, "Difficulty (%)");
        assert_eq!(d.value.as_deref(), Some("50"));
        assert_eq!(d.left.as_deref(), Some("set trainer.difficulty 45"));
        assert_eq!(d.right.as_deref(), Some("set trainer.difficulty 55"));
        assert_eq!(d.confirm, None);
        assert_eq!(
            row(&trainer, "Scan for devices").confirm,
            cmd("devices.scan")
        );
        let controls = rows(Page::Controls, &s, &Riders::default());
        assert_eq!(controls[0].confirm, cmd("set controls.autodrive off"));
        let strava = rows(Page::Strava, &s, &Riders::default());
        assert_eq!(
            row(&strava, "Retry failed uploads").confirm,
            cmd("strava.sync --retry")
        );
    }

    #[test]
    fn workshop_selects_the_rider() {
        let s = Settings::default();
        let r = riders();
        let w = rows(Page::Workshop, &s, &r);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].value.as_deref(), Some("78 kg, FTP 250 W - riding"));
        assert_eq!(w[1].value.as_deref(), Some("61.5 kg"));
        assert_eq!(w[1].confirm, cmd("riders.select \"Anna\""));
        assert_eq!(w[2].confirm, Some(Activate::Open(Page::Riders)));
        // Without profiles: the default rider, and the way to the editor.
        let w = rows(Page::Workshop, &s, &Riders::default());
        assert_eq!(w[0].value.as_deref(), Some("75 kg - riding (default)"));
        assert_eq!(w[0].confirm, None);
        assert_eq!(w[1].confirm, Some(Activate::Open(Page::Riders)));
    }

    #[test]
    fn rider_pages_edit_the_profile() {
        let s = Settings::default();
        let r = riders();
        let list = rows(Page::Riders, &s, &r);
        assert_eq!(list[1].confirm, Some(Activate::Open(Page::Rider(1))));
        assert_eq!(list.last().unwrap().confirm, cmd("riders.add \"Rider\""));
        let alex = rows(Page::Rider(0), &s, &r);
        let weight = row(&alex, "Weight");
        assert_eq!(weight.value.as_deref(), Some("78 kg"));
        assert_eq!(
            weight.left.as_deref(),
            Some("riders.edit \"Alex\" --weight 77")
        );
        assert_eq!(
            weight.right.as_deref(),
            Some("riders.edit \"Alex\" --weight 79")
        );
        // No height yet: ← / → start at 175 cm.
        let height = row(&alex, "Height");
        assert_eq!(height.value.as_deref(), Some("-"));
        assert_eq!(
            height.right.as_deref(),
            Some("riders.edit \"Alex\" --height 175")
        );
        assert_eq!(
            row(&alex, "FTP").left.as_deref(),
            Some("riders.edit \"Alex\" --ftp 245")
        );
        assert_eq!(
            row(&alex, "Delete this rider").confirm,
            cmd("riders.remove \"Alex\"")
        );
        // The weight stays in range at the ends.
        let mut light = Riders::default();
        light
            .add(Rider {
                name: "Kid".into(),
                weight_kg: 30.0,
                ..Rider::default()
            })
            .unwrap();
        assert_eq!(
            row(&rows(Page::Rider(0), &s, &light), "Weight")
                .left
                .as_deref(),
            Some("riders.edit \"Kid\" --weight 30")
        );
        assert!(rows(Page::Rider(9), &s, &r).is_empty());
    }

    #[test]
    fn page_parents() {
        assert_eq!(Page::Rider(2).parent(), Some(Page::Riders));
        assert_eq!(Page::Workshop.parent(), None);
    }
}
