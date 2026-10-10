//! Functions in the app: the screen for one function (its address, who can run it, its code,
//! a place to try it, and its log), and turning the feature on in the background.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::icons::{self, Icon};
use crate::model::FnItem;
use crate::names::pretty;
use crate::theme::Tokens;
use crate::{Dialog, PorchlyApp};

/// Methods offered when trying a function.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
/// Log lines shown under a function.
const LOG_LINES: usize = 200;

/// A function open on screen.
pub(crate) struct FnEditor {
    pub name: String,
    pub file: String,
    pub code: String,
    pub saved_code: String,
    pub public: bool,
    /// Variables as `NAME=value` lines.
    pub vars: String,
    pub saved_vars: String,
    pub try_method: String,
    pub try_path: String,
    pub try_body: String,
    pub try_result: Option<Result<(u16, String), String>>,
    pub trying: Option<Receiver<Result<Value, String>>>,
    pub log: Vec<String>,
}

impl FnEditor {
    pub fn dirty(&self) -> bool {
        self.code != self.saved_code
    }
}

/// Where turning Functions on has got to.
pub(crate) struct Enabling {
    pub result: Receiver<Result<Value, String>>,
}

impl PorchlyApp {
    /// Re-reads whether functions are on and which exist.
    pub(crate) fn reload_functions(&mut self) {
        let status = self.engine.execute("functions.status", &Value::Null).ok();
        let flag = |k: &str| status.as_ref().and_then(|s| s.get(k)).and_then(Value::as_bool).unwrap_or(false);
        self.functions_available = flag("available");
        self.functions_on = flag("enabled");
        self.functions = if self.functions_on {
            self.run("function.list", json!({}))
                .and_then(|v| serde_json::from_value::<Vec<FnItem>>(v).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if self.open_function.as_ref().is_some_and(|f| !self.functions.iter().any(|x| x.name == f.name)) {
            self.open_function = None;
        }
    }

    /// Shows a function instead of a table.
    pub fn open_function(&mut self, name: &str) {
        let Some(v) = self.run("function.get", json!({"name": name})) else { return };
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let vars: String = v
            .get("env")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| format!("{k}={}\n", v.as_str().unwrap_or_default())).collect())
            .unwrap_or_default();
        let code = text("code");
        self.open_function = Some(FnEditor {
            name: name.to_owned(),
            file: text("file"),
            saved_code: code.clone(),
            code,
            public: v.get("access").and_then(Value::as_str) == Some("public"),
            saved_vars: vars.clone(),
            vars,
            try_method: "GET".into(),
            try_path: String::new(),
            try_body: String::new(),
            try_result: None,
            trying: None,
            log: Vec::new(),
        });
        self.reload_function_log();
    }

    pub(crate) fn reload_function_log(&mut self) {
        let Some(name) = self.open_function.as_ref().map(|f| f.name.clone()) else { return };
        let lines = self
            .engine
            .execute("function.logs", &json!({"name": name, "lines": LOG_LINES}))
            .ok()
            .and_then(|v| v.get("lines").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        if let Some(f) = &mut self.open_function {
            f.log = lines.iter().filter_map(Value::as_str).map(str::to_owned).collect();
        }
    }

    /// Saves the open function's code. Returns whether it worked.
    pub fn save_function(&mut self) -> bool {
        let Some((name, code)) = self.open_function.as_ref().map(|f| (f.name.clone(), f.code.clone())) else {
            return false;
        };
        if self.run("function.save", json!({"name": name, "code": code})).is_none() {
            return false;
        }
        if let Some(f) = &mut self.open_function {
            f.saved_code = code;
        }
        true
    }

    fn save_function_settings(&mut self) {
        let Some(f) = &self.open_function else { return };
        let env: serde_json::Map<String, Value> = f
            .vars
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_owned(), json!(v.trim())))
            .filter(|(k, _)| !k.is_empty())
            .collect();
        let params = json!({"name": f.name, "access": if f.public { "public" } else { "key" }, "env": env});
        if self.run("function.settings", params).is_some() {
            if let Some(f) = &mut self.open_function {
                f.saved_vars = f.vars.clone();
            }
            self.reload_functions();
        }
    }

    /// Saves, then calls the open function through the API in the background.
    pub fn try_function(&mut self) {
        if self.open_function.as_ref().is_some_and(FnEditor::dirty) && !self.save_function() {
            return;
        }
        let Some(f) = &mut self.open_function else { return };
        let mut params = json!({"name": f.name, "method": f.try_method, "path": f.try_path});
        if !f.try_body.trim().is_empty()
            && let Some(p) = params.as_object_mut()
        {
            p.insert("body".into(), json!(f.try_body));
        }
        let (tx, rx) = mpsc::channel();
        let engine = self.engine.clone();
        let spawned = std::thread::Builder::new().name("porchly-try-function".into()).spawn(move || {
            let _ = tx.send(engine.execute("function.call", &params).map_err(|e| e.to_string()));
        });
        match spawned {
            Ok(_) => {
                f.trying = Some(rx);
                f.try_result = None;
            }
            Err(e) => f.try_result = Some(Err(format!("Couldn't start: {e}"))),
        }
    }

    /// Turns Functions on in the background (downloading Bun the first time).
    pub fn start_enabling_functions(&mut self) {
        if self.enabling.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let engine = self.engine.clone();
        let spawned = std::thread::Builder::new().name("porchly-functions-download".into()).spawn(move || {
            let _ = tx.send(engine.execute("functions.enable", &json!({})).map_err(|e| e.to_string()));
        });
        match spawned {
            Ok(_) => self.enabling = Some(Enabling { result: rx }),
            Err(e) => self.dialog_error = Some(format!("Couldn't start the download: {e}")),
        }
    }

    /// Checks on background work: turning Functions on, trying a function.
    pub(crate) fn poll_function_jobs(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.enabling {
            match job.result.try_recv() {
                Err(TryRecvError::Empty) => {
                    let progress = self
                        .engine
                        .execute("functions.status", &Value::Null)
                        .ok()
                        .and_then(|s| s.get("download").cloned())
                        .and_then(|d| Some((d.get("done")?.as_u64()?, d.get("total")?.as_u64()?)));
                    if let Some(Dialog::FunctionsOptIn { progress: p, .. }) = &mut self.dialog {
                        *p = Some(progress.unwrap_or((0, 0)));
                    }
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Ok(result) => {
                    self.enabling = None;
                    match result {
                        Ok(_) => {
                            if matches!(self.dialog, Some(Dialog::FunctionsOptIn { .. })) {
                                self.dialog = None;
                            }
                            self.dialog_error = None;
                            self.reload_functions();
                            self.info("Functions are on. Make your first one with \u{201c}New function\u{201d}.");
                        }
                        Err(e) => {
                            if let Some(Dialog::FunctionsOptIn { progress, .. }) = &mut self.dialog {
                                *progress = None;
                            }
                            self.dialog_error = Some(e);
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => self.enabling = None,
            }
        }
        let mut finished = false;
        if let Some(f) = &mut self.open_function
            && let Some(rx) = &f.trying
        {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Ok(result) => {
                    f.try_result = Some(result.map(|v| {
                        let status = v.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
                        let body = v.get("body").and_then(Value::as_str).unwrap_or_default();
                        // Pretty-print JSON answers; show anything else as it came.
                        let body = serde_json::from_str::<Value>(body)
                            .ok()
                            .and_then(|j| serde_json::to_string_pretty(&j).ok())
                            .unwrap_or_else(|| body.to_owned());
                        (status, body)
                    }));
                    f.trying = None;
                    finished = true;
                }
                Err(TryRecvError::Disconnected) => f.trying = None,
            }
        }
        if finished {
            self.reload_function_log();
        }
    }
}

/// The function screen (in the middle of the window).
pub(crate) fn show(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens) {
    let address = if app.server.running {
        app.open_function.as_ref().map(|f| format!("{}/api/fn/{}", app.server.url, f.name))
    } else {
        None
    };
    let Some(f) = &mut app.open_function else { return };
    let mut action = None;

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
        icons::paint(ui.painter(), rect, Icon::Code, t.accent);
        ui.label(RichText::new(pretty(&f.name)).size(22.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, Some(Icon::Trash), "Delete\u{2026}", false).clicked() {
                action = Some(Action::Delete);
            }
            if icons::button(ui, Some(Icon::Edit), "Rename\u{2026}", false).clicked() {
                action = Some(Action::Rename);
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| match &address {
        Some(a) => {
            ui.label(RichText::new("Address").color(t.text_dim));
            ui.label(RichText::new(a).monospace());
            if icons::icon_button(ui, Icon::Copy, "Copy the address").clicked() {
                ui.ctx().copy_text(a.clone());
                action = Some(Action::Copied);
            }
        }
        None => {
            ui.label(RichText::new("Turn on the API (Share) to reach this function.").color(t.text_dim));
        }
    });
    ui.add_space(10.0);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Who can run it.
        ui.label(RichText::new("Who can run it").strong());
        ui.horizontal(|ui| {
            let before = f.public;
            ui.radio_value(&mut f.public, false, "Apps with a key");
            ui.radio_value(&mut f.public, true, "Anyone, no key needed");
            if f.public != before {
                action = Some(Action::SaveSettings);
            }
        });
        if f.public {
            egui::Frame::NONE.fill(t.danger_soft).corner_radius(6).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Anyone who knows the address can run this function. With the porch light on, that's anyone on the internet. Use this for webhooks from services like Stripe or GitHub, and check who is calling inside your code.",
                    )
                    .color(t.danger),
                );
            });
        }
        ui.add_space(12.0);

        // The code.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Code").strong());
            ui.label(RichText::new(format!("functions/{}", f.file)).small().color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = f.dirty();
                if ui.add_enabled_ui(dirty, |ui| icons::button(ui, None, "Save", dirty)).inner.clicked() {
                    action = Some(Action::Save);
                }
                if dirty {
                    ui.label(RichText::new("Not saved yet").small().color(t.accent));
                }
            });
        });
        let save_key = ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S)));
        if save_key && f.dirty() {
            action = Some(Action::Save);
        }
        egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("code").max_height(360.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut f.code)
                        .code_editor()
                        .desired_rows(16)
                        .desired_width(f32::INFINITY)
                        .lock_focus(true),
                );
            });
        });
        ui.label(
            RichText::new("Tip: the code file is a normal file, so you can also edit it in any editor. Saving is enough; nothing needs restarting.")
                .small()
                .color(t.text_dim),
        );
        ui.add_space(14.0);

        // Try it, and the log.
        ui.columns(2, |cols| {
            let [left, right] = cols else { return };
            left.label(RichText::new("Try it").strong());
            left.horizontal(|ui| {
                egui::ComboBox::from_id_salt("try_method").selected_text(f.try_method.as_str()).width(80.0).show_ui(
                    ui,
                    |ui| {
                        for m in METHODS {
                            ui.selectable_value(&mut f.try_method, m.to_owned(), m);
                        }
                    },
                );
                ui.add(egui::TextEdit::singleline(&mut f.try_path).hint_text("/ (optional path)").desired_width(160.0));
                let busy = f.trying.is_some();
                let label = if busy { "Running\u{2026}" } else { "Run" };
                if ui.add_enabled_ui(!busy, |ui| icons::button(ui, None, label, true)).inner.clicked() {
                    action = Some(Action::Try);
                }
            });
            if f.try_method != "GET" {
                left.add(
                    egui::TextEdit::multiline(&mut f.try_body)
                        .hint_text("Body to send, for example {\"name\": \"Ann\"}")
                        .code_editor()
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            }
            match &f.try_result {
                Some(Ok((status, body))) => {
                    let color = if *status < 400 { t.ok } else { t.danger };
                    left.label(RichText::new(format!("Answered {status}")).color(color).strong());
                    egui::ScrollArea::vertical().id_salt("try_body").max_height(200.0).show(left, |ui| {
                        // Read-only, but selectable, so the answer can be copied.
                        ui.add(egui::TextEdit::multiline(&mut body.as_str()).code_editor().desired_width(f32::INFINITY));
                    });
                }
                Some(Err(e)) => {
                    left.label(RichText::new(e).color(t.danger));
                }
                None => {
                    left.label(
                        RichText::new("Runs the saved function through your API, as an app would.").small().color(t.text_dim),
                    );
                }
            }

            right.horizontal(|ui| {
                ui.label(RichText::new("Log").strong());
                ui.label(RichText::new("what the function printed").small().color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, None, "Clear", false).clicked() {
                        action = Some(Action::ClearLog);
                    }
                    if icons::button(ui, None, "Refresh", false).clicked() {
                        action = Some(Action::RefreshLog);
                    }
                });
            });
            egui::Frame::NONE.fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(
                right,
                |ui| {
                    egui::ScrollArea::vertical().id_salt("log").max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if f.log.is_empty() {
                            ui.label(RichText::new("Nothing yet. console.log() lines show up here.").color(t.text_dim));
                        }
                        for line in &f.log {
                            ui.label(RichText::new(line).monospace().size(12.0));
                        }
                    });
                },
            );
        });
        ui.add_space(14.0);

        // Variables.
        ui.label(RichText::new("Variables").strong());
        ui.label(
            RichText::new("For keys of other services. One per line, like MAIL_KEY=abc123. The code reads them with process.env.MAIL_KEY.")
                .small()
                .color(t.text_dim),
        );
        ui.add(egui::TextEdit::multiline(&mut f.vars).code_editor().desired_rows(3).desired_width(f32::INFINITY));
        let vars_dirty = f.vars != f.saved_vars;
        if ui.add_enabled_ui(vars_dirty, |ui| icons::button(ui, None, "Save variables", vars_dirty)).inner.clicked() {
            action = Some(Action::SaveSettings);
        }
    });

    let name = f.name.clone();
    match action {
        Some(Action::Save) => {
            if app.save_function() {
                app.info("Saved. The next call runs the new code.");
            }
        }
        Some(Action::SaveSettings) => app.save_function_settings(),
        Some(Action::Try) => app.try_function(),
        Some(Action::RefreshLog) => app.reload_function_log(),
        Some(Action::ClearLog) => {
            app.run("function.logs", json!({"name": name, "lines": 0, "clear": true}));
            app.reload_function_log();
        }
        Some(Action::Copied) => app.info("Copied the address."),
        Some(Action::Rename) => app.open_dialog(Dialog::RenameFunction { name: name.clone(), label: pretty(&name) }),
        Some(Action::Delete) => app.open_dialog(Dialog::DeleteFunction { name }),
        None => {}
    }
}

enum Action {
    Save,
    SaveSettings,
    Try,
    RefreshLog,
    ClearLog,
    Copied,
    Rename,
    Delete,
}
