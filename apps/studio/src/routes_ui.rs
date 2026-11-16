//! The Routes workspace's panel (spec §14 Phase 16, 16D): new, import and
//! the package's routes; the via-points; the planner (built-in profiles,
//! BRouter with its server and the privacy note); the edits; the
//! elevation profile with the gradient colours and the imported track's
//! elevation as a grey line; the figures, problems and findings; export,
//! save and *Test ride*. It only shows the route and sends commands.

use bevy_egui::egui;
use ocw_edit::Severity;
use ocw_edit::route::ProblemKind;
use ocw_world::formats::TrackFormat;
use ocw_world::plan::Profile;
use ocw_world::plan::brouter::STOCK_PROFILES;

use crate::edit::{Pending, Studio, StudioCommand};
use crate::routes::{PlannerKind, RouteCommand, RouteView, grade_rgb};
use crate::ui::Tool;
use crate::world::OpenWorld;

/// The Routes panel's fields.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteForm {
    pub import: String,
    pub export: String,
    pub format: TrackFormat,
    /// The BRouter server field (applied on Enter / leaving it).
    pub brouter_url: Option<String>,
}

impl Default for RouteForm {
    fn default() -> Self {
        Self {
            import: String::new(),
            export: String::new(),
            format: TrackFormat::Gpx,
            brouter_url: None,
        }
    }
}

fn send(pending: &mut Pending, c: RouteCommand) {
    pending.send(StudioCommand::Route(c));
}

fn rgb(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// The colour of a problem section (map and list).
pub fn problem_rgb(k: ProblemKind) -> [u8; 3] {
    match k {
        ProblemKind::OffGraph => [255, 170, 40],
        ProblemKind::Unmatched | ProblemKind::NotMatched => [255, 60, 60],
        ProblemKind::Gap => [200, 120, 255],
        ProblemKind::MissingPackage => [120, 180, 255],
    }
}

pub fn routes_panel(
    ui: &mut egui::Ui,
    studio: &Studio,
    world: Option<&OpenWorld>,
    form: &mut RouteForm,
    tool: &mut Tool,
    pending: &mut Pending,
) {
    ui.heading("Routes");
    if world.is_none() {
        ui.label("No world open. File > Open world... (routes are planned on its roads)");
        return;
    }
    ui.horizontal(|ui| {
        if ui.button("New route").clicked() {
            send(pending, RouteCommand::New(None));
        }
        ui.label("Import");
        let r = ui.add(
            egui::TextEdit::singleline(&mut form.import)
                .hint_text("~/ride.gpx | .tcx | .geojson | .fit | .ocwroute")
                .desired_width(170.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Open").clicked() || enter) && !form.import.trim().is_empty() {
            send(
                pending,
                RouteCommand::Import(form.import.trim().to_string()),
            );
        }
    });
    if let Some(pkg) = studio.package.as_ref()
        && !pkg.pkg.routes.is_empty()
    {
        egui::CollapsingHeader::new(format!("The package's routes ({})", pkg.pkg.routes.len()))
            .default_open(studio.route().is_none())
            .show(ui, |ui| {
                for (i, r) in pkg.pkg.routes.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("Open").clicked() {
                            send(pending, RouteCommand::PackageRoute(i));
                        }
                        ui.label(&r.name);
                    });
                }
            });
    }
    let Some(doc) = studio.route() else {
        ui.add_space(6.0);
        ui.label("New route: then click the map (or the 3D view) to add via-points; drag the route on the map to move it onto other roads.");
        return;
    };
    let c = doc.content();
    let view = &studio.routes.view;
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Name");
        let mut name = c.name.clone();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(200.0));
        if r.changed() {
            send(pending, RouteCommand::Rename(name));
        }
        if r.lost_focus() {
            send(pending, RouteCommand::Seal);
        }
    });
    let mut desc = c.description.clone();
    let r = ui.add(
        egui::TextEdit::multiline(&mut desc)
            .hint_text("Description")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    if r.changed() {
        send(pending, RouteCommand::Describe(desc));
    }
    if r.lost_focus() {
        send(pending, RouteCommand::Seal);
    }
    // The figures.
    if let Some(p) = &view.profile {
        ui.label(
            egui::RichText::new(format!(
                "{}  ↑ {:.0} m  ↓ {:.0} m  steepest {:+.1} % / {:+.1} %{}",
                crate::map::format_length(p.length_m),
                p.gain_m,
                p.loss_m,
                p.max_grade * 100.0,
                p.min_grade * 100.0,
                if c.geometry.closed { "  (loop)" } else { "" }
            ))
            .strong(),
        );
        if !p.surfaces.is_empty() && p.length_m > 0.0 {
            let parts: Vec<String> = p
                .surfaces
                .iter()
                .map(|(s, m)| {
                    format!(
                        "{} {:.0} %",
                        crate::ui::surface_name(*s),
                        100.0 * m / p.length_m
                    )
                })
                .collect();
            ui.label(parts.join(", "));
        }
        profile_widget(ui, view, 150.0);
    } else if let Some(e) = &view.error {
        ui.colored_label(egui::Color32::from_rgb(255, 90, 80), e);
    }
    for credit in &c.geometry.credits {
        ui.label(egui::RichText::new(credit).weak());
    }
    // Planner.
    ui.add_space(6.0);
    egui::CollapsingHeader::new(egui::RichText::new("Planner").strong())
        .default_open(true)
        .show(ui, |ui| planner_panel(ui, studio, form, pending));
    // Via-points and edits.
    ui.add_space(6.0);
    ui.label(egui::RichText::new(format!("Via-points ({})", c.geometry.vias.len())).strong());
    match tool {
        Tool::RouteMoveVia(i) => {
            ui.colored_label(
                egui::Color32::from_rgb(120, 200, 255),
                format!("Click where via-point {} goes (Esc: cancel)", *i + 1),
            );
        }
        Tool::RouteInsertVia(i) => {
            ui.colored_label(
                egui::Color32::from_rgb(120, 200, 255),
                format!(
                    "Click where the new via-point {} goes (Esc: cancel)",
                    *i + 1
                ),
            );
        }
        _ => {
            ui.label("A click on the map adds a via-point at the end; drag the route or a via-point to move it.");
        }
    }
    let selected = doc.selection.single().copied();
    egui::ScrollArea::vertical()
        .id_salt("route-vias")
        .max_height(160.0)
        .show(ui, |ui| {
            for (i, v) in c.geometry.vias.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(selected == Some(i), format!("{:>2}", i + 1))
                        .clicked()
                    {
                        send(pending, RouteCommand::SelectVia(Some(i)));
                    }
                    ui.label(format!("{:.0}, {:.0}", v.at[0], v.at[1]));
                    if ui.small_button("Move").clicked() {
                        *tool = Tool::RouteMoveVia(i);
                    }
                    if ui
                        .small_button("Insert")
                        .on_hover_text("A new via-point before this one")
                        .clicked()
                    {
                        *tool = Tool::RouteInsertVia(i);
                    }
                    if ui.small_button("Remove").clicked() {
                        send(pending, RouteCommand::RemoveVia(i));
                    }
                });
            }
        });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Reverse").clicked() {
            send(pending, RouteCommand::Reverse);
        }
        let closed = c.geometry.closed;
        if ui
            .button(if closed {
                "Open the loop"
            } else {
                "Make a loop"
            })
            .clicked()
        {
            send(pending, RouteCommand::Loop(!closed));
        }
        if ui
            .button("Out and back")
            .on_hover_text("Ride back the same way, with a U-turn at the end")
            .clicked()
        {
            send(pending, RouteCommand::OutAndBack);
        }
        if let Some(i) = selected {
            if ui.button(format!("Start at {}", i + 1)).clicked() {
                send(pending, RouteCommand::CutStart(i));
            }
            if ui.button(format!("End at {}", i + 1)).clicked() {
                send(pending, RouteCommand::CutEnd(i));
            }
            if ui
                .button(format!("Split at {}", i + 1))
                .on_hover_text("The second part is saved as a route file of its own")
                .clicked()
            {
                send(pending, RouteCommand::Split(i));
            }
        }
    });
    // Problems of the import.
    if !c.problems.is_empty() {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!(
                "Problem sections of the {} ({})",
                c.track.as_ref().map_or("track", |t| t.format.as_str()),
                c.problems.len()
            ))
            .strong(),
        );
        for p in &c.problems {
            ui.colored_label(rgb(problem_rgb(p.kind)), p.message());
        }
    }
    let findings: Vec<_> = doc
        .findings()
        .iter()
        .filter(|f| {
            f.target
                .as_deref()
                .is_none_or(|t| !t.starts_with("problem:"))
        })
        .collect();
    if !findings.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Findings").strong());
        for f in findings {
            let colour = match f.severity {
                Severity::Error => egui::Color32::from_rgb(255, 90, 80),
                Severity::Warning => egui::Color32::from_rgb(255, 190, 80),
                Severity::Info => egui::Color32::from_gray(190),
            };
            ui.colored_label(colour, &f.message);
        }
    }
    // Export, save, test ride.
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Export").strong());
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("route-format")
            .selected_text(form.format.name())
            .show_ui(ui, |ui| {
                for f in TrackFormat::ALL {
                    if ui.selectable_label(form.format == f, f.name()).clicked() {
                        form.format = f;
                    }
                }
            });
        if form.export.is_empty() {
            form.export = format!("~/{}.{}", slug(&c.name), form.format.extension());
        }
        ui.add(egui::TextEdit::singleline(&mut form.export).desired_width(160.0));
        if ui.button("Export").clicked() {
            send(
                pending,
                RouteCommand::Export(form.export.trim().to_string(), Some(form.format)),
            );
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("Save route")
            .on_hover_text("As a graph route next to the package (routes/NAME.ocwroute)")
            .clicked()
        {
            send(pending, RouteCommand::Save(None));
        }
        let riding = studio.routes.test_ride.is_some();
        if ui
            .add_enabled(!riding, egui::Button::new("Test ride"))
            .on_hover_text("Saves the route and rides it in ocw-desktop")
            .clicked()
        {
            send(pending, RouteCommand::TestRide);
        }
        if riding {
            ui.label("riding …");
        }
    });
    if let Some(f) = &studio.routes.file {
        ui.label(egui::RichText::new(crate::edit::shown_path(f)).weak());
    }
    ui.label(
        egui::RichText::new("Exported tracks follow OSM roads: © OpenStreetMap contributors")
            .weak(),
    );
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "route".into() } else { s }
}

fn planner_panel(ui: &mut egui::Ui, studio: &Studio, form: &mut RouteForm, pending: &mut Pending) {
    let s = &studio.routes.settings;
    let doc = studio.route();
    ui.horizontal(|ui| {
        ui.label("Profile");
        let now = doc.map(|d| d.content().profile.clone()).unwrap_or_default();
        egui::ComboBox::from_id_salt("route-profile")
            .selected_text(Profile::parse(&now).map_or("-", Profile::name))
            .show_ui(ui, |ui| {
                for p in Profile::ALL {
                    if ui.selectable_label(now == p.id(), p.name()).clicked() {
                        send(pending, RouteCommand::Profile(p.id().into()));
                    }
                }
            });
        ui.label(egui::RichText::new("(built-in planner, on the package's roads)").weak());
    });
    ui.horizontal(|ui| {
        ui.label("Planner");
        if ui
            .radio(s.planner == PlannerKind::Builtin, "Built-in")
            .clicked()
        {
            send(pending, RouteCommand::Planner(PlannerKind::Builtin));
        }
        if ui
            .radio(s.planner == PlannerKind::Brouter, "BRouter")
            .clicked()
        {
            send(pending, RouteCommand::Planner(PlannerKind::Brouter));
        }
    });
    if s.planner != PlannerKind::Brouter {
        return;
    }
    ui.horizontal(|ui| {
        ui.label("Server");
        let text = form
            .brouter_url
            .get_or_insert_with(|| s.brouter_url.clone());
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .hint_text("https://… (none is set by default)")
                .desired_width(200.0),
        );
        if r.lost_focus() {
            send(pending, RouteCommand::BRouterUrl(text.clone()));
            form.brouter_url = None;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Profile");
        egui::ComboBox::from_id_salt("brouter-profile")
            .selected_text(&s.brouter_profile)
            .show_ui(ui, |ui| {
                for p in STOCK_PROFILES {
                    if ui.selectable_label(s.brouter_profile == p, p).clicked() {
                        send(pending, RouteCommand::BRouterProfile(p.into()));
                    }
                }
            });
    });
    if s.brouter_url.trim().is_empty() {
        ui.label(
            "Enter a BRouter server (your own, a friend's, or a public one within its terms).",
        );
        return;
    }
    let mut consent = studio.routes.brouter_consent;
    if ui
        .checkbox(
            &mut consent,
            format!(
                "Send the via-points to {} to plan (they leave this computer)",
                s.brouter_url.trim()
            ),
        )
        .changed()
    {
        send(pending, RouteCommand::BRouterConsent(consent));
    }
    ui.horizontal(|ui| {
        let busy = studio.routes.job.is_some();
        let ok = consent && !busy && doc.is_some_and(|d| d.content().geometry.vias.len() >= 2);
        if ui
            .add_enabled(ok, egui::Button::new("Plan with BRouter"))
            .on_hover_text(
                "Plans through the via-points; the answer is matched onto the package's roads",
            )
            .clicked()
        {
            send(pending, RouteCommand::PlanExternal);
        }
        if let Some(job) = &studio.routes.job {
            ui.spinner();
            ui.label(format!("asking BRouter at {} …", job.server));
        }
    });
}

/// The elevation profile: the road's heights coloured by gradient, the
/// imported track's elevation grey, axes with the height range and the
/// distance.
pub fn profile_widget(ui: &mut egui::Ui, view: &RouteView, height: f32) {
    let Some(p) = &view.profile else {
        return;
    };
    if p.samples.len() < 2 {
        return;
    }
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(28));
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in &p.samples {
        lo = lo.min(s[1]);
        hi = hi.max(s[1]);
    }
    for s in &view.track_ele {
        lo = lo.min(s[1]);
        hi = hi.max(s[1]);
    }
    if hi - lo < 20.0 {
        let mid = (hi + lo) * 0.5;
        lo = mid - 10.0;
        hi = mid + 10.0;
    }
    let pad = (hi - lo) * 0.08;
    let (lo, hi) = (lo - pad, hi + pad);
    let len = p.length_m.max(1.0);
    let inner = rect.shrink2(egui::vec2(34.0, 14.0));
    let inner = egui::Rect::from_min_max(egui::pos2(inner.left() + 8.0, inner.top()), inner.max);
    let at = |d: f64, h: f64| {
        egui::pos2(
            inner.left() + (d / len) as f32 * inner.width(),
            inner.bottom() - ((h - lo) / (hi - lo)) as f32 * inner.height(),
        )
    };
    // The imported track's elevation, scaled to the route's length.
    if view.track_ele.len() >= 2 {
        let tl = view.track_ele.last().map_or(1.0, |t| t[0]).max(1.0);
        let pts: Vec<egui::Pos2> = view
            .track_ele
            .iter()
            .map(|t| at(t[0] / tl * len, t[1]))
            .collect();
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new(1.0, egui::Color32::from_gray(150)),
        ));
    }
    // The road profile, filled by gradient over 50 m.
    let mut k = 0;
    while k + 1 < p.samples.len() {
        let mut j = k + 1;
        while j + 1 < p.samples.len() && p.samples[j][0] - p.samples[k][0] < 50.0 {
            j += 1;
        }
        let (a, b) = (p.samples[k], p.samples[j]);
        let g = (b[1] - a[1]) / (b[0] - a[0]).max(0.01);
        let c = rgb(grade_rgb(g));
        let pts: Vec<egui::Pos2> = p.samples[k..=j].iter().map(|s| at(s[0], s[1])).collect();
        // Thin vertical strips (convex) for the fill.
        for w in pts.windows(2) {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    w[0],
                    w[1],
                    egui::pos2(w[1].x, inner.bottom()),
                    egui::pos2(w[0].x, inner.bottom()),
                ],
                c.gamma_multiply(0.45),
                egui::Stroke::NONE,
            ));
        }
        painter.add(egui::Shape::line(pts, egui::Stroke::new(2.0, c)));
        k = j;
    }
    let text = |pos: egui::Pos2, align: egui::Align2, s: String| {
        painter.text(
            pos,
            align,
            s,
            egui::FontId::proportional(11.0),
            egui::Color32::from_gray(210),
        );
    };
    text(
        egui::pos2(rect.left() + 4.0, inner.top()),
        egui::Align2::LEFT_TOP,
        format!("{:.0} m", hi),
    );
    text(
        egui::pos2(rect.left() + 4.0, inner.bottom()),
        egui::Align2::LEFT_BOTTOM,
        format!("{:.0} m", lo),
    );
    text(
        egui::pos2(inner.right(), rect.bottom() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        crate::map::format_length(len),
    );
    text(
        egui::pos2(inner.left(), rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        "0".into(),
    );
    if let Some(pos) = response.hover_pos()
        && inner.contains(pos)
    {
        let d = f64::from((pos.x - inner.left()) / inner.width()) * len;
        let i = p
            .samples
            .partition_point(|s| s[0] < d)
            .min(p.samples.len() - 1);
        let s = p.samples[i];
        let q = at(s[0], s[1]);
        painter.line_segment(
            [
                egui::pos2(q.x, inner.top()),
                egui::pos2(q.x, inner.bottom()),
            ],
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        );
        text(
            egui::pos2(q.x + 4.0, inner.top()),
            egui::Align2::LEFT_TOP,
            format!("{}  {:.0} m", crate::map::format_length(s[0]), s[1]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names_are_slugs() {
        assert_eq!(slug("Ada's Loop 2"), "ada-s-loop-2");
        assert_eq!(slug("!!"), "route");
    }

    #[test]
    fn problems_have_distinct_colours() {
        let kinds = [
            ProblemKind::OffGraph,
            ProblemKind::Unmatched,
            ProblemKind::Gap,
            ProblemKind::MissingPackage,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(problem_rgb(*a), problem_rgb(*b));
            }
        }
    }
}
