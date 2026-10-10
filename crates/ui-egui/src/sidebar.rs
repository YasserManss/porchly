//! The list of tables on the left.

use egui::{RichText, Sense, Ui};

use crate::icons::{self, Icon};
use crate::names::pretty;
use crate::theme::Tokens;
use crate::{Dialog, PorchlyApp};

pub(crate) fn show(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("tables")
        .resizable(true)
        .default_size(220.0)
        .size_range(160.0..=420.0)
        .frame(
            egui::Frame::NONE
                .fill(t.chrome)
                .inner_margin(egui::Margin::symmetric(12, 16))
                .stroke(egui::Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            ui.label(RichText::new("TABLES").size(12.0).color(t.text_dim).strong());
            ui.add_space(4.0);
            let mut clicked = None;
            // Room kept below the tables for the buttons and the Functions section.
            let functions_height = match (app.functions_available, app.functions_on) {
                (false, _) => 0.0,
                (true, false) => 70.0,
                (true, true) => 120.0 + 34.0 * app.functions.len().min(6) as f32,
            };
            let tables_height = (ui.available_height() - 88.0 - functions_height).max(68.0);
            egui::ScrollArea::vertical()
                .id_salt("tables_list")
                .auto_shrink([false, true])
                .max_height(tables_height)
                .show(ui, |ui| {
                    for table in &app.tables {
                        let selected =
                            app.open_function.is_none() && app.selected.as_deref() == Some(table.name.as_str());
                        let id = egui::Id::new(("table_row", &table.name));
                        if table_row(ui, t, id, &pretty(&table.name), selected).clicked() {
                            clicked = Some(table.name.clone());
                        }
                    }
                });
            if let Some(name) = clicked {
                app.select_table(&name);
            }
            ui.add_space(8.0);
            if icons::button(ui, Some(Icon::Plus), "New table", false).clicked() {
                app.open_dialog(Dialog::new_table());
            }
            ui.add_space(4.0);
            if icons::button(ui, Some(Icon::Import), "Import CSV\u{2026}", false)
                .on_hover_text("Bring in a spreadsheet saved as CSV. You can also drop the file on this window.")
                .clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv", "tsv", "txt"]).pick_file()
            {
                app.begin_import(&path);
            }
            if app.functions_available {
                functions_section(app, ui, t);
            }
        });
}

fn functions_section(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens) {
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(6.0);
    if !app.functions_on {
        // Off until wanted: one quiet line that explains itself when clicked.
        let response =
            list_row(ui, t, egui::Id::new("functions_off_row"), Icon::Code, "Functions", false, true, Some("Off"))
                .on_hover_text("Add small programs to your API. Click to learn more.");
        if response.clicked() {
            app.open_dialog(Dialog::functions_opt_in());
        }
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("FUNCTIONS").size(12.0).color(t.text_dim).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new(RichText::new("Turn off").small().color(t.text_dim)).frame(false))
                .on_hover_text("Stop running functions (their code is kept)")
                .clicked()
            {
                app.open_dialog(Dialog::TurnOffFunctions { remove_runtime: false });
            }
        });
    });
    ui.add_space(4.0);
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("functions_list").auto_shrink([false, true]).max_height(34.0 * 6.0).show(
        ui,
        |ui| {
            if app.functions.is_empty() {
                ui.label(RichText::new("No functions yet.").color(t.text_dim));
            }
            for f in &app.functions {
                let selected = app.open_function.as_ref().is_some_and(|o| o.name == f.name);
                let badge = (f.access == "public").then_some("no key");
                let id = egui::Id::new(("function_row", &f.name));
                if list_row(ui, t, id, Icon::Code, &pretty(&f.name), selected, false, badge).clicked() {
                    clicked = Some(f.name.clone());
                }
            }
        },
    );
    if let Some(name) = clicked {
        app.open_function(&name);
    }
    ui.add_space(8.0);
    if icons::button(ui, Some(Icon::Plus), "New function", false).clicked() {
        app.open_dialog(Dialog::new_function());
    }
}

fn table_row(ui: &mut Ui, t: &Tokens, id: egui::Id, label: &str, selected: bool) -> egui::Response {
    list_row(ui, t, id, Icon::Table, label, selected, false, None)
}

/// A row in the sidebar: icon, label, and an optional small note on the right. `dim` draws
/// it greyed out (a feature that's off). `id` must be stable (from the row's name): ids made
/// from the position can change between frames, and then clicks get lost.
#[allow(clippy::too_many_arguments)]
fn list_row(
    ui: &mut Ui,
    t: &Tokens,
    id: egui::Id,
    icon: Icon,
    label: &str,
    selected: bool,
    dim: bool,
    note: Option<&str>,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 34.0), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    // Screen readers (and tests) hear the note too: "Stripe hook, no key", "Functions, Off".
    let spoken = match note {
        Some(n) => format!("{label}, {n}"),
        None => label.to_owned(),
    };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &spoken));
    let fill = if selected {
        Some(t.accent_soft)
    } else if response.hovered() {
        Some(t.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 7.0, fill);
    }
    let icon_rect =
        egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), egui::Vec2::splat(16.0));
    let icon_color = if selected {
        t.accent
    } else if dim {
        t.off
    } else {
        t.text_dim
    };
    icons::paint(ui.painter(), icon_rect, icon, icon_color);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_color = if dim { t.off } else { t.text };
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    if let Some(note) = note {
        let small = egui::TextStyle::Small.resolve(ui.style());
        let color = if dim { t.off } else { t.danger };
        ui.painter().text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            note,
            small,
            color,
        );
    }
    response
}
