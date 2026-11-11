//! Interim egui panel for the Settings and Workshop pages (`settings.panel
//! [page]`), until the menu shell (M1, Bevy UI) renders [`pages::rows`]
//! itself. It only draws rows and queues their command lines, so it has no
//! logic of its own; mouse only (the ride keeps its keys meanwhile).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use ocw_render::console::ConsoleState;

use super::pages::{self, Activate, Page};
use super::{RiderProfiles, UserSettings};

/// The open page of the panel (`None`: closed).
#[derive(Resource, Default)]
pub struct SettingsPanel {
    pub page: Option<Page>,
    /// The rename field of the rider editor.
    rename: String,
}

/// Draws the panel (needs the egui plugin, added by the console).
pub struct SettingsPanelPlugin;

impl Plugin for SettingsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SettingsPanel>()
            .add_systems(EguiPrimaryContextPass, panel_ui);
    }
}

fn panel_ui(
    mut contexts: EguiContexts,
    mut panel: ResMut<SettingsPanel>,
    settings: Res<UserSettings>,
    riders: Res<RiderProfiles>,
    mut console: ResMut<ConsoleState>,
) -> Result {
    let Some(page) = panel.page else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let height = ctx.content_rect().height();
    // Ten-foot text: about 3 % of the window height (spec §3.2.1).
    let size = (height * 0.03).clamp(14.0, 40.0);
    let text = |s: &str| egui::RichText::new(s).size(size);
    let rows = pages::rows(page, &settings.settings, &riders.riders);
    let mut next_page = Some(page);
    egui::Window::new("settings")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(12, 16, 24, 235))
                .inner_margin(size)
                .corner_radius(size * 0.4),
        )
        .show(ctx, |ui| {
            ui.style_mut().spacing.item_spacing = egui::vec2(size * 0.5, size * 0.4);
            ui.horizontal(|ui| {
                for tab in Page::TABS {
                    let on = tab == page || page.parent() == Some(tab);
                    if ui.selectable_label(on, text(tab.title())).clicked() {
                        next_page = Some(tab);
                    }
                }
            });
            ui.separator();
            let title = match page {
                Page::Rider(i) => riders
                    .riders
                    .riders
                    .get(i)
                    .map_or("Rider".to_string(), |r| r.name.clone()),
                p => p.title().to_string(),
            };
            ui.label(text(&title).strong().color(egui::Color32::WHITE));
            for note in page.notes() {
                ui.label(text(note).size(size * 0.7).color(egui::Color32::LIGHT_GRAY));
            }
            ui.add_space(size * 0.3);
            egui::Grid::new("settings-rows")
                .num_columns(3)
                .spacing([size, size * 0.4])
                .show(ui, |ui| {
                    for row in &rows {
                        ui.label(text(&row.label).color(egui::Color32::WHITE));
                        ui.label(
                            text(row.value.as_deref().unwrap_or(""))
                                .color(egui::Color32::from_rgb(140, 200, 255)),
                        );
                        ui.horizontal(|ui| {
                            if let Some(l) = &row.left
                                && ui.button(text(" < ")).clicked()
                            {
                                console.queue(l.clone());
                            }
                            if let Some(r) = &row.right
                                && ui.button(text(" > ")).clicked()
                            {
                                console.queue(r.clone());
                            }
                            match &row.confirm {
                                Some(Activate::Command(c)) => {
                                    if ui.button(text("OK")).clicked() {
                                        console.queue(c.clone());
                                        if c.starts_with("riders.remove") {
                                            next_page = Some(Page::Riders);
                                        }
                                    }
                                }
                                Some(Activate::Open(p)) if ui.button(text("Open")).clicked() => {
                                    next_page = Some(*p);
                                }
                                Some(Activate::Open(_)) | None => {}
                            }
                        });
                        ui.end_row();
                    }
                });
            if let Page::Rider(i) = page
                && let Some(r) = riders.riders.riders.get(i)
            {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut panel.rename)
                            .font(egui::FontId::proportional(size))
                            .hint_text("new name")
                            .desired_width(size * 10.0),
                    );
                    if ui.button(text("Rename")).clicked() && !panel.rename.trim().is_empty() {
                        console.queue(format!(
                            "riders.edit \"{}\" --rename \"{}\"",
                            r.name,
                            panel.rename.trim()
                        ));
                        panel.rename.clear();
                    }
                });
            }
            ui.add_space(size * 0.3);
            ui.horizontal(|ui| {
                if let Some(parent) = page.parent()
                    && ui.button(text("Back")).clicked()
                {
                    next_page = Some(parent);
                }
                if ui.button(text("Close")).clicked() {
                    next_page = None;
                }
            });
        });
    panel.page = next_page;
    Ok(())
}
