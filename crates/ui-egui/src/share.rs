//! The Share panel: turn the API on or off, hand out keys, and the porch light (public link).

use egui::{RichText, Ui};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::porch_light;
use crate::theme::Tokens;
use crate::{Dialog, PorchlyApp};

enum Action {
    Start,
    Stop,
    Revoke(i64),
    NewKey,
    Copy(String, &'static str),
    OpenDocs,
    ConnectAi,
    DismissFreshKey,
    Light(porch_light::Action),
}

pub(crate) fn show(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens) {
    let mut actions = Vec::new();
    egui::Panel::right("share")
        .resizable(true)
        .default_size(360.0)
        .size_range(300.0..=640.0)
        .frame(
            egui::Frame::NONE
                .fill(t.surface)
                .inner_margin(egui::Margin::same(18))
                .stroke(egui::Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Share").heading().strong());
                ui.label(
                    RichText::new(
                        "Apps and websites use your tables through Porchly's API. Each one needs a key from you.",
                    )
                    .color(t.text_dim),
                );
                section(ui, t, "Your API");
                api_section(app, ui, t, &mut actions);
                section(ui, t, "Keys");
                keys_section(app, ui, t, &mut actions);
                section(ui, t, "On the internet");
                let mut light_actions = Vec::new();
                porch_light::section(app, ui, t, &mut light_actions);
                actions.extend(light_actions.into_iter().map(Action::Light));
                ui.add_space(10.0);
                developers(app, ui, t, &mut actions);
            });
        });
    apply(app, ui.ctx(), actions);
}

fn section(ui: &mut Ui, t: &Tokens, title: &str) {
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(RichText::new(title).size(16.0).strong().color(t.text));
    ui.add_space(2.0);
}

fn api_section(app: &PorchlyApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    if app.server.running {
        let reach = if app.light.is_on() {
            "On. With the porch light on, apps anywhere can reach it."
        } else {
            "On. Only this computer can reach it."
        };
        icons::status_dot(ui, true, reach);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&app.server.url).monospace());
            if icons::icon_button(ui, Icon::Copy, "Copy the address").clicked() {
                actions.push(Action::Copy(app.server.url.clone(), "Address copied."));
            }
        });
        ui.add_space(4.0);
        let hint = if app.light.is_on() {
            "Also turns off the porch light"
        } else {
            "Apps can't reach your tables while it's off"
        };
        if icons::button(ui, None, "Turn off", false).on_hover_text(hint).clicked() {
            actions.push(Action::Stop);
        }
    } else {
        icons::status_dot(ui, false, "Off. Apps can't reach your tables.");
        if let Some(e) = &app.server.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(4.0);
        if icons::button(ui, None, "Turn on", true).clicked() {
            actions.push(Action::Start);
        }
    }
}

fn keys_section(app: &PorchlyApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    if let Some((name, secret)) = &app.fresh_key {
        egui::Frame::NONE.fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.label(RichText::new(format!("Here is the key for \u{201c}{name}\u{201d}")).strong());
            ui.label("Copy it now and give it to the app. Porchly can't show it again.");
            ui.add_space(4.0);
            let mut job = egui::text::LayoutJob::single_section(
                secret.clone(),
                egui::TextFormat { font_id: egui::FontId::monospace(13.5), color: t.text, ..Default::default() },
            );
            job.wrap = egui::text::TextWrapping {
                break_anywhere: true,
                max_width: ui.available_width(),
                ..Default::default()
            };
            ui.label(job);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if icons::button(ui, Some(Icon::Copy), "Copy key", true).clicked() {
                    actions.push(Action::Copy(secret.clone(), "Key copied."));
                }
                if icons::button(ui, None, "Done", false).clicked() {
                    actions.push(Action::DismissFreshKey);
                }
            });
        });
        ui.add_space(8.0);
    }
    let active: Vec<_> = app.keys.iter().filter(|k| k.revoked_at.is_none()).collect();
    if active.is_empty() {
        ui.label(RichText::new("No keys yet, so no app can use your tables.").color(t.text_dim));
    }
    for key in active {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
            icons::paint(ui.painter(), rect, Icon::Key, t.accent);
            ui.vertical(|ui| {
                ui.label(RichText::new(&key.name).strong());
                let what = if key.can_write { "Can read and make changes" } else { "Can only read" };
                ui.label(RichText::new(format!("{what} · {}…", key.prefix)).small().color(t.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Turn off").on_hover_text("This key stops working right away").clicked() {
                    actions.push(Action::Revoke(key.id));
                }
            });
        });
    }
    let revoked = app.keys.iter().filter(|k| k.revoked_at.is_some()).count();
    if revoked > 0 {
        ui.label(RichText::new(format!("{revoked} key(s) turned off.")).small().color(t.text_dim));
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if icons::button(ui, Some(Icon::Plus), "New key", false).clicked() {
            actions.push(Action::NewKey);
        }
        // AI assistants use a key like any other app.
        if icons::button(ui, None, "Connect an AI assistant\u{2026}", false)
            .on_hover_text("Let Claude, Cursor or another assistant look things up in your tables, and change them if its key allows")
            .clicked()
        {
            actions.push(Action::ConnectAi);
        }
    });
}

fn developers(app: &PorchlyApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    egui::CollapsingHeader::new(RichText::new("For developers").strong()).default_open(false).show(ui, |ui| {
        let base = if app.server.url.is_empty() { "http://127.0.0.1:11911".to_owned() } else { app.server.url.clone() };
        let table = app.selected.clone().unwrap_or_else(|| "your_table".to_owned());
        let example = format!("curl -H \"Authorization: Bearer YOUR_KEY\" \\\n  {base}/api/{table}");
        ui.label(RichText::new(&example).monospace().color(t.text));
        if ui.small_button("Copy example").clicked() {
            actions.push(Action::Copy(example.clone(), "Example copied."));
        }
        ui.label(
            RichText::new(format!("A full description of your API is at {base}/api/openapi.json"))
                .small()
                .color(t.text_dim),
        );
        if icons::button(ui, Some(Icon::Share), "Open API docs", false)
            .on_hover_text("A guide to your API with ready-to-run examples, opened in your web browser")
            .clicked()
        {
            actions.push(Action::OpenDocs);
        }
        if let Some(log) = &app.log_path {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Log file: {log}")).small().color(t.text_dim));
        }
    });
}

fn apply(app: &mut PorchlyApp, ctx: &egui::Context, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Start => app.start_api(),
            Action::Stop => {
                app.run("api.stop", json!({}));
                app.refresh_status();
                app.refresh_light();
            }
            Action::Revoke(id) => {
                if app.run("api.key.revoke", json!({"id": id})).is_some() {
                    app.info("Key turned off. Apps using it can't reach your tables any more.");
                }
                app.reload_keys();
            }
            Action::NewKey => app.open_dialog(Dialog::new_key()),
            Action::Copy(text, done) => {
                ctx.copy_text(text);
                app.info(done);
            }
            Action::DismissFreshKey => app.fresh_key = None,
            Action::OpenDocs => app.open_docs(ctx),
            Action::ConnectAi => app.open_connect_ai(),
            Action::Light(a) => porch_light::apply(app, ctx, a),
        }
    }
}
