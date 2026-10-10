//! The porch light in the app: the "On the internet" part of the Share panel, and turning
//! the light on in the background (the first time downloads cloudflared).

use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{Dialog, PorchlyApp};

/// The porch light as the app shows it (from `tunnel.status`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Light {
    /// "off", "downloading", "connecting", "on" or "failed".
    pub state: String,
    pub url: Option<String>,
    pub slow: bool,
    pub error: Option<String>,
    pub allow_changes: bool,
    pub program_ready: bool,
    pub download: Option<(u64, u64)>,
}

impl Light {
    fn from(v: &Value) -> Self {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        Light {
            state: text("state").unwrap_or_else(|| "off".into()),
            url: text("url"),
            slow: flag("slow"),
            error: text("error"),
            allow_changes: flag("allow_changes"),
            program_ready: flag("program_ready"),
            download: v.get("download").and_then(|d| Some((d.get("done")?.as_u64()?, d.get("total")?.as_u64()?))),
        }
    }

    pub fn is_on(&self) -> bool {
        self.state == "on"
    }

    /// Downloading or connecting: the status is re-read often.
    pub fn busy(&self) -> bool {
        self.state == "downloading" || self.state == "connecting"
    }
}

/// Turning the light on, in the background.
pub(crate) struct Starting {
    pub result: Receiver<Result<Value, String>>,
}

impl PorchlyApp {
    pub(crate) fn refresh_light(&mut self) {
        if let Ok(v) = self.engine.execute("tunnel.status", &Value::Null) {
            self.light = Light::from(&v);
        }
    }

    /// Turns the porch light on in the background (it may download cloudflared first).
    pub fn start_porch_light(&mut self, allow_changes: bool) {
        if self.light_starting.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let engine = self.engine.clone();
        let spawned = std::thread::Builder::new().name("porchly-porch-light-start".into()).spawn(move || {
            let _ = tx.send(
                engine.execute("tunnel.start", &json!({"allow_changes": allow_changes})).map_err(|e| e.to_string()),
            );
        });
        match spawned {
            Ok(_) => {
                self.light_starting = Some(Starting { result: rx });
                self.light.state = if self.light.program_ready { "connecting".into() } else { "downloading".into() };
                self.light.error = None;
            }
            Err(e) => self.error(format!("Couldn't turn on the porch light: {e}")),
        }
    }

    pub(crate) fn stop_porch_light(&mut self) {
        self.run("tunnel.stop", json!({}));
        self.refresh_light();
    }

    /// Checks on the light while it downloads or connects.
    pub(crate) fn poll_light(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.light_starting {
            match job.result.try_recv() {
                Err(TryRecvError::Empty) => {}
                Ok(Ok(_)) | Err(TryRecvError::Disconnected) => self.light_starting = None,
                Ok(Err(e)) => {
                    self.light_starting = None;
                    self.error(e);
                }
            }
        }
        if self.light.busy() || self.light_starting.is_some() {
            let was_on = self.light.is_on();
            self.refresh_light();
            if self.light_starting.is_some() && self.light.state == "off" {
                // Still downloading on the background thread: keep showing that.
                self.light.state = "downloading".into();
            }
            if !was_on && self.light.is_on() {
                self.info("The porch light is on. Your API has a public link.");
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

pub(crate) enum Action {
    TurnOn,
    TurnOff,
    Retry,
    AllowChanges(bool),
    Copy(String),
}

/// The "On the internet" section of the Share panel.
pub(crate) fn section(app: &PorchlyApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let light = &app.light;
    match light.state.as_str() {
        "on" => {
            lamp_status(ui, t, true, "On. Apps anywhere can reach your API with a key.");
            if let Some(url) = &light.url {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(url).monospace()).wrap());
                    if icons::icon_button(ui, Icon::Copy, "Copy the public link").clicked() {
                        actions.push(Action::Copy(url.clone()));
                    }
                });
            }
            changes_checkbox(light, ui, t, actions);
            ui.label(
                RichText::new("The link changes every time you turn the light on. It's free, from Cloudflare.")
                    .small()
                    .color(t.text_dim),
            );
            ui.add_space(4.0);
            if icons::button(ui, None, "Turn off", false).clicked() {
                actions.push(Action::TurnOff);
            }
        }
        "downloading" => {
            ui.label("Getting the porch light ready (a one-time download from Cloudflare)\u{2026}");
            let (done, total) = light.download.unwrap_or((0, 0));
            let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.scope(|ui| {
                ui.visuals_mut().extreme_bg_color = t.hover;
                ui.add(egui::ProgressBar::new(fraction).desired_width(ui.available_width()).fill(t.accent));
            });
        }
        "connecting" => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Connecting to Cloudflare\u{2026}");
            });
            if light.slow {
                ui.label(
                    RichText::new("This is taking longer than usual. Some networks (offices, hotels, schools) block this kind of link.")
                        .small()
                        .color(t.text_dim),
                );
            }
            if icons::button(ui, None, "Cancel", false).clicked() {
                actions.push(Action::TurnOff);
            }
        }
        "failed" => {
            lamp_status(ui, t, false, "The porch light couldn't stay on.");
            if let Some(e) = &light.error {
                ui.label(RichText::new(e).color(t.danger));
            }
            ui.horizontal(|ui| {
                if icons::button(ui, None, "Try again", true).clicked() {
                    actions.push(Action::Retry);
                }
                if icons::button(ui, None, "Turn off", false).clicked() {
                    actions.push(Action::TurnOff);
                }
            });
        }
        _ => {
            lamp_status(ui, t, false, "Off. Only this computer can reach your API.");
            ui.label(
                RichText::new(
                    "Turn on the porch light to give your API a link that works from anywhere. Apps still need a key.",
                )
                .color(t.text_dim),
            );
            ui.add_space(4.0);
            if icons::button(ui, Some(Icon::Lamp), "Turn on the porch light\u{2026}", true).clicked() {
                actions.push(Action::TurnOn);
            }
        }
    }
}

fn changes_checkbox(light: &Light, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let mut allow = light.allow_changes;
    if ui.checkbox(&mut allow, "Allow changes through the link").changed() {
        actions.push(Action::AllowChanges(allow));
    }
    let note = if allow {
        "Keys that may make changes can change your tables from anywhere."
    } else {
        "Apps can only read through the link, even with a key that may make changes."
    };
    ui.label(RichText::new(note).small().color(t.text_dim));
}

/// The porch lantern, lit or not, with a sentence.
fn lamp_status(ui: &mut Ui, t: &Tokens, on: bool, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
        if on {
            ui.painter().circle_filled(rect.center(), 10.0, t.accent_soft);
        }
        icons::paint(ui.painter(), rect, Icon::Lamp, if on { t.accent } else { t.off });
        ui.label(RichText::new(text).color(if on { t.text } else { t.text_dim }));
    });
}

pub(crate) fn apply(app: &mut PorchlyApp, ctx: &egui::Context, action: Action) {
    match action {
        Action::TurnOn => app.open_dialog(Dialog::PorchLight {
            allow_changes: app.light.allow_changes,
            public_functions: app.functions.iter().any(|f| f.access == "public"),
        }),
        Action::Retry => {
            app.stop_porch_light();
            app.start_porch_light(app.light.allow_changes);
        }
        Action::TurnOff => {
            app.stop_porch_light();
            app.info("The porch light is off. The public link no longer works.");
        }
        Action::AllowChanges(allow) => {
            if let Some(v) = app.run("tunnel.allow_changes", json!({"allow": allow})) {
                app.light = Light::from(&v);
            }
        }
        Action::Copy(url) => {
            ctx.copy_text(url);
            app.info("Public link copied.");
        }
    }
}
