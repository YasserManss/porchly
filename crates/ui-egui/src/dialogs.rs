//! Dialogs. Each one only collects input; confirming runs an engine command. If the command
//! fails, the dialog stays open with the error shown, so nothing typed is lost.

use egui::{RichText, Ui};
use porchly_core::FieldKind;
use serde_json::json;

use crate::PorchlyApp;
use crate::icons;
use crate::names::{pretty, to_ident};
use crate::theme::Tokens;

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    NewTable {
        label: String,
        with_name_field: bool,
    },
    AddField {
        table: String,
        label: String,
        kind: FieldKind,
        required: bool,
        links_to: Option<String>,
    },
    NewKey {
        name: String,
        can_write: bool,
    },
    DeleteRow {
        table: String,
        id: i64,
    },
    DeleteTable {
        table: String,
    },
    DeleteField {
        table: String,
        field: String,
    },
    /// Bring rows in from a CSV file, into a new table or an existing one.
    ImportCsv {
        path: String,
        file: String,
        total_rows: u64,
        columns: Vec<ImportColumn>,
        sample: Vec<Vec<String>>,
        into_new: bool,
        new_label: String,
        existing: Option<String>,
    },
    /// A few plain settings.
    Settings {
        /// Requests per second, per app, as typed.
        rate: String,
        function_timeout: u64,
        port: String,
        start_api: bool,
        /// Whether to show the functions' time limit.
        functions_on: bool,
    },
    /// Explains the porch light before turning it on.
    PorchLight {
        allow_changes: bool,
        /// Whether some functions may be run by anyone (then the warning says so).
        public_functions: bool,
    },
    /// Explains Functions and offers to turn them on (which downloads Bun once).
    FunctionsOptIn {
        /// While downloading: bytes done and total.
        progress: Option<(u64, u64)>,
    },
    NewFunction {
        label: String,
        template: String,
    },
    RenameFunction {
        name: String,
        label: String,
    },
    DeleteFunction {
        name: String,
    },
    TurnOffFunctions {
        remove_runtime: bool,
    },
    /// Rename a field, change its type, link target or whether it's required; or delete it.
    EditField {
        table: String,
        field: String,
        label: String,
        kind: FieldKind,
        original_kind: FieldKind,
        required: bool,
        links_to: Option<String>,
    },
}

/// A column of a file being imported.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportColumn {
    pub header: String,
    /// The field it becomes, or `None` if skipped (id, created_at, updated_at).
    pub name: Option<String>,
    pub kind: FieldKind,
}

impl Dialog {
    /// The import dialog for a file, from the engine's `csv.preview`. `current` is the table
    /// that's open, offered when adding to an existing table.
    pub fn import_csv(path: &std::path::Path, preview: &serde_json::Value, current: Option<String>) -> Option<Self> {
        let columns = preview
            .get("columns")?
            .as_array()?
            .iter()
            .map(|c| {
                Some(ImportColumn {
                    header: c.get("header")?.as_str()?.to_owned(),
                    name: c.get("name").and_then(|n| n.as_str()).map(str::to_owned),
                    kind: c.get("type")?.as_str()?.parse().ok()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let sample = preview
            .get("rows")?
            .as_array()?
            .iter()
            .map(|r| r.as_array().map(|cells| cells.iter().map(|c| c.as_str().unwrap_or("").to_owned()).collect()))
            .collect::<Option<Vec<Vec<String>>>>()?;
        let suggested = preview.get("suggested_table").and_then(|s| s.as_str()).unwrap_or("imported");
        Some(Dialog::ImportCsv {
            path: path.display().to_string(),
            file: preview.get("file").and_then(|f| f.as_str()).unwrap_or("the file").to_owned(),
            total_rows: preview.get("total_rows").and_then(|n| n.as_u64()).unwrap_or(0),
            columns,
            sample,
            into_new: true,
            new_label: pretty(suggested),
            existing: current,
        })
    }

    pub fn new_table() -> Self {
        Dialog::NewTable { label: String::new(), with_name_field: true }
    }

    pub fn add_field(table: &str) -> Self {
        Dialog::AddField {
            table: table.to_owned(),
            label: String::new(),
            kind: FieldKind::Text,
            required: false,
            links_to: None,
        }
    }

    /// The "Change field" dialog, filled in with the field's current settings.
    pub fn edit_field(table: &str, field: &str, kind: FieldKind, required: bool, links_to: Option<String>) -> Self {
        Dialog::EditField {
            table: table.to_owned(),
            field: field.to_owned(),
            label: pretty(field),
            kind,
            original_kind: kind,
            required,
            links_to,
        }
    }

    pub fn new_key() -> Self {
        Dialog::NewKey { name: String::new(), can_write: false }
    }

    /// The Settings dialog, filled in from `settings.get`.
    pub fn settings(current: &serde_json::Value, functions_on: bool) -> Self {
        Dialog::Settings {
            rate: current.get("requests_per_second").and_then(|v| v.as_u64()).unwrap_or(20).to_string(),
            function_timeout: current.get("function_timeout").and_then(|v| v.as_u64()).unwrap_or(0),
            port: current.get("port").and_then(|v| v.as_u64()).unwrap_or(11911).to_string(),
            start_api: current.get("start_api").and_then(|v| v.as_bool()).unwrap_or(true),
            functions_on,
        }
    }

    pub fn functions_opt_in() -> Self {
        Dialog::FunctionsOptIn { progress: None }
    }

    pub fn new_function() -> Self {
        Dialog::NewFunction { label: String::new(), template: "hello".into() }
    }
}

enum Outcome {
    Keep,
    Cancel,
    Confirm,
    /// Close this dialog and open another (for example "Delete field…" from "Change field").
    Replace(Box<Dialog>),
}

pub(crate) fn show(app: &mut PorchlyApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.dialog.take() else { return };
    let t = Tokens::get(ctx);
    let tables: Vec<String> = app.tables.iter().map(|t| t.name.clone()).collect();
    let error = app.dialog_error.clone();
    let mut outcome = Outcome::Keep;
    let response = egui::Modal::new(egui::Id::new("porchly_dialog"))
        .frame(
            egui::Frame::NONE
                .fill(t.surface)
                .corner_radius(12)
                .inner_margin(egui::Margin::same(22))
                .stroke(egui::Stroke::new(1.0, t.border)),
        )
        .show(ctx, |ui| {
            ui.set_width(match dialog {
                Dialog::ImportCsv { .. } => 680.0,
                Dialog::FunctionsOptIn { .. } | Dialog::PorchLight { .. } => 460.0,
                _ => 400.0,
            });
            outcome = body(&mut dialog, ui, &t, &tables, error.as_deref());
        });
    if response.should_close() && matches!(outcome, Outcome::Keep) {
        outcome = Outcome::Cancel;
    }
    match outcome {
        Outcome::Keep => app.dialog = Some(dialog),
        Outcome::Cancel => app.dialog_error = None,
        Outcome::Replace(next) => {
            app.dialog_error = None;
            app.dialog = Some(*next);
        }
        Outcome::Confirm => {
            // On failure the dialog stays open and shows why, where the person is looking.
            app.dialog = Some(dialog);
            confirm_now(app);
        }
    }
}

/// Confirms the open dialog without drawing it (screenshots and tests).
pub(crate) fn confirm_now(app: &mut PorchlyApp) {
    let Some(dialog) = app.dialog.take() else { return };
    if confirm(app, &dialog) {
        app.dialog_error = None;
    } else {
        if let Some(notice) = app.notice.take_if(|n| n.error) {
            app.dialog_error = Some(notice.text);
        }
        app.dialog = Some(dialog);
    }
}

fn body(dialog: &mut Dialog, ui: &mut Ui, t: &Tokens, tables: &[String], error: Option<&str>) -> Outcome {
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    fields(dialog, ui, t, tables, enter, error)
}

fn fields(
    dialog: &mut Dialog,
    ui: &mut Ui,
    t: &Tokens,
    tables: &[String],
    enter: bool,
    error: Option<&str>,
) -> Outcome {
    let buttons = |ui: &mut Ui, label: &str, danger: bool, ok: bool, enter: bool| {
        show_error(ui, t, error);
        buttons(ui, t, label, danger, ok, enter)
    };
    match dialog {
        Dialog::NewTable { label, with_name_field } => {
            title(ui, "New table");
            ui.label("What will you keep in it?");
            let field = ui.add(
                egui::TextEdit::singleline(label).hint_text("For example: Guest list").desired_width(f32::INFINITY),
            );
            focus_first(ui, &field);
            let ident = to_ident(label);
            link_preview(ui, t, ident.as_deref());
            ui.add_space(6.0);
            ui.checkbox(with_name_field, "Start with a \u{201c}Name\u{201d} field");
            buttons(ui, "Make table", false, ident.is_some(), enter)
        }
        Dialog::AddField { table, label, kind, required, links_to } => {
            title(ui, &format!("Add a field to {}", pretty(table)));
            ui.label("Field name");
            let field = ui.add(
                egui::TextEdit::singleline(label).hint_text("For example: Phone number").desired_width(f32::INFINITY),
            );
            focus_first(ui, &field);
            ui.add_space(6.0);
            ui.label("What goes in it?");
            egui::ComboBox::from_id_salt("field_kind").selected_text(kind.label()).width(220.0).show_ui(ui, |ui| {
                for k in FieldKind::ALL {
                    ui.selectable_value(kind, k, k.label());
                }
            });
            if *kind == FieldKind::Link {
                ui.add_space(6.0);
                ui.label("Which table does it link to?");
                let shown = links_to.as_deref().map_or_else(|| "Choose a table".to_owned(), pretty);
                egui::ComboBox::from_id_salt("link_target").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                    for name in tables {
                        ui.selectable_value(links_to, Some(name.clone()), pretty(name));
                    }
                });
                ui.label(
                    RichText::new(
                        "Each row can then point to one row of that table, like an order pointing to its customer.",
                    )
                    .small()
                    .color(t.text_dim),
                );
            } else {
                *links_to = None;
            }
            ui.add_space(6.0);
            ui.checkbox(required, "Every row must fill this in");
            let ident = to_ident(label);
            let ready = ident.is_some() && (*kind != FieldKind::Link || links_to.is_some());
            buttons(ui, "Add field", false, ready, enter)
        }
        Dialog::NewKey { name, can_write } => {
            title(ui, "New key");
            ui.label("Who is this key for?");
            let field = ui.add(
                egui::TextEdit::singleline(name).hint_text("For example: My website").desired_width(f32::INFINITY),
            );
            focus_first(ui, &field);
            ui.add_space(6.0);
            ui.checkbox(can_write, "Can make changes (add, change and delete rows)");
            ui.label(
                RichText::new("Leave this off unless the app really needs to change your data.")
                    .small()
                    .color(t.text_dim),
            );
            buttons(ui, "Make key", false, !name.trim().is_empty(), enter)
        }
        Dialog::ImportCsv { file, total_rows, columns, sample, into_new, new_label, existing, .. } => {
            title(ui, &format!("Import \u{201c}{file}\u{201d}"));
            let used = columns.iter().filter(|c| c.name.is_some()).count();
            ui.label(format!("{total_rows} rows, {used} columns."));
            ui.add_space(6.0);
            ui.radio_value(into_new, true, "Make a new table");
            if *into_new {
                ui.indent("new_table", |ui| {
                    ui.add(egui::TextEdit::singleline(new_label).desired_width(260.0));
                    link_preview(ui, t, to_ident(new_label).as_deref());
                });
            }
            ui.radio_value(into_new, false, "Add the rows to a table I already have");
            if !*into_new {
                ui.indent("existing_table", |ui| {
                    let shown = existing.as_deref().map_or_else(|| "Choose a table".to_owned(), pretty);
                    egui::ComboBox::from_id_salt("import_target").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                        for name in tables {
                            ui.selectable_value(existing, Some(name.clone()), pretty(name));
                        }
                    });
                    ui.label(RichText::new("Column names must match the table's fields.").small().color(t.text_dim));
                });
            }
            ui.add_space(8.0);
            egui::ScrollArea::horizontal().max_height(220.0).show(ui, |ui| {
                egui::Grid::new("import_preview").striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                    for col in columns.iter_mut() {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&col.header).strong());
                            match (&col.name, *into_new) {
                                (None, _) => {
                                    ui.label(RichText::new("skipped").small().color(t.text_dim));
                                }
                                (Some(_), true) => {
                                    egui::ComboBox::from_id_salt(("import_kind", &col.header))
                                        .selected_text(col.kind.label())
                                        .width(120.0)
                                        .show_ui(ui, |ui| {
                                            for k in FieldKind::ALL.into_iter().filter(|k| *k != FieldKind::Link) {
                                                ui.selectable_value(&mut col.kind, k, k.label());
                                            }
                                        });
                                }
                                (Some(_), false) => {}
                            }
                        });
                    }
                    ui.end_row();
                    for row in sample.iter() {
                        for cell in row {
                            ui.label(cell.chars().take(40).collect::<String>());
                        }
                        ui.end_row();
                    }
                });
            });
            let ready = if *into_new { to_ident(new_label).is_some() } else { existing.is_some() };
            buttons(ui, "Import", false, ready, enter)
        }
        Dialog::Settings { rate, function_timeout, port, start_api, functions_on } => {
            title(ui, "Settings");
            ui.label(RichText::new("Requests per second, per app").strong());
            ui.add(egui::TextEdit::singleline(rate).desired_width(90.0));
            let rate_ok = rate.trim().parse::<u32>().is_ok_and(|r| (1..=1000).contains(&r));
            let note = if rate_ok {
                "Apps that ask faster are told to slow down. This keeps your computer from being overwhelmed. Usually 20."
            } else {
                "Use a whole number from 1 to 1000."
            };
            ui.label(RichText::new(note).small().color(if rate_ok { t.text_dim } else { t.danger }));
            if *functions_on {
                ui.add_space(10.0);
                ui.label(RichText::new("Stop a function that hasn't answered after").strong());
                let name = |s: u64| match s {
                    0 => "No limit".to_owned(),
                    60 => "1 minute".to_owned(),
                    s if s > 60 => format!("{} minutes", s / 60),
                    s => format!("{s} seconds"),
                };
                egui::ComboBox::from_id_salt("function_timeout")
                    .selected_text(name(*function_timeout))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for s in [10, 30, 60, 300, 0] {
                            ui.selectable_value(function_timeout, s, name(s));
                        }
                    });
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Port").strong());
            ui.add(egui::TextEdit::singleline(port).desired_width(90.0));
            let port_ok = port.trim().parse::<u16>().is_ok_and(|p| p >= 1024);
            let note = if port_ok {
                format!(
                    "The number at the end of your API's address (http://127.0.0.1:{}). Change it only if another program uses it.",
                    port.trim()
                )
            } else {
                "Use a number from 1024 to 65535.".to_owned()
            };
            ui.label(RichText::new(note).small().color(if port_ok { t.text_dim } else { t.danger }));
            ui.add_space(10.0);
            ui.checkbox(start_api, "Turn on the API when Porchly opens");
            show_error(ui, t, error);
            buttons(ui, "Save", false, port_ok && rate_ok, enter)
        }
        Dialog::PorchLight { allow_changes, public_functions } => {
            title(ui, "Turn on the porch light?");
            ui.label("Your API gets a public address on the internet, like https://\u{2026}.trycloudflare.com. Then apps and websites anywhere can reach it, not just this computer.");
            ui.add_space(8.0);
            let point = |ui: &mut Ui, text: &str| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("\u{2022}").color(t.accent));
                    ui.label(text);
                });
            };
            point(
                ui,
                "Anyone with the address and one of your keys can use your tables. Without a key, they get nothing.",
            );
            if *public_functions {
                point(
                    ui,
                    "Your functions set to \u{201c}Anyone, no key needed\u{201d} can be run by anyone on the internet.",
                );
            }
            point(
                ui,
                "The link is free, from Cloudflare. It changes every time you turn the light on, and it's meant for trying things out and sharing for a while.",
            );
            ui.add_space(8.0);
            ui.checkbox(allow_changes, "Allow changes through the link");
            let note = if *allow_changes {
                "Keys that may make changes can change your tables from anywhere."
            } else {
                "Apps can only read through the link (safer). You can change this later."
            };
            ui.label(RichText::new(note).small().color(t.text_dim));
            ui.add_space(4.0);
            ui.label(
                RichText::new("The first time, Porchly downloads a free helper from Cloudflare (about 40 MB).")
                    .small()
                    .color(t.text_dim),
            );
            show_error(ui, t, error);
            buttons(ui, "Turn on", false, true, false)
        }
        Dialog::FunctionsOptIn { progress } => {
            title(ui, "Functions");
            ui.label("Functions let a developer add small programs to your API, for example to send an email when someone places an order.");
            ui.add_space(6.0);
            ui.label("To use them, Porchly needs to download a free helper called Bun (about 40 MB) once. It's kept in your Porchly folder. You can turn this off again at any time.");
            ui.add_space(6.0);
            ui.label(
                RichText::new("Functions can do anything with your data and the internet, and Porchly doesn't check them. Only add code from people you trust.")
                    .small()
                    .color(t.text_dim),
            );
            show_error(ui, t, error);
            if let Some((done, total)) = progress {
                ui.add_space(14.0);
                let fraction = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                ui.scope(|ui| {
                    // The empty part of the bar, visible on the white dialog.
                    ui.visuals_mut().extreme_bg_color = t.hover;
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(ui.available_width())
                            .fill(t.accent)
                            .text(RichText::new(format!("{:.0}%", fraction * 100.0)).color(t.text)),
                    );
                });
                ui.label(
                    RichText::new(format!(
                        "Downloading Bun\u{2026} {} of {} MB",
                        *done / 1_000_000,
                        (*total).max(1) / 1_000_000
                    ))
                    .small()
                    .color(t.text_dim),
                );
                return Outcome::Keep;
            }
            let label = if error.is_some() { "Try again" } else { "Turn on Functions" };
            buttons_labeled(ui, label, "Not now")
        }
        Dialog::NewFunction { label, template } => {
            title(ui, "New function");
            ui.label("Name");
            let field = ui.add(
                egui::TextEdit::singleline(label).hint_text("For example: Send invoice").desired_width(f32::INFINITY),
            );
            focus_first(ui, &field);
            let ident = to_ident(label);
            let hint = match &ident {
                Some(id) => format!("Apps will reach it at /api/fn/{id}"),
                None => "Use at least one letter or number.".to_owned(),
            };
            ui.label(RichText::new(hint).small().color(t.text_dim));
            ui.add_space(8.0);
            ui.label("Start from");
            ui.radio_value(template, "hello".to_owned(), "A simple example that says hello");
            ui.radio_value(template, "tables".to_owned(), "An example that reads your tables");
            show_error(ui, t, error);
            buttons(ui, "Make function", false, ident.is_some(), enter)
        }
        Dialog::RenameFunction { name, label } => {
            title(ui, &format!("Rename \u{201c}{}\u{201d}", pretty(name)));
            let field = ui.add(egui::TextEdit::singleline(label).desired_width(f32::INFINITY));
            focus_first(ui, &field);
            let ident = to_ident(label);
            let hint = match &ident {
                Some(id) => format!("Its address becomes /api/fn/{id}. Apps using the old address will stop working."),
                None => "Use at least one letter or number.".to_owned(),
            };
            ui.label(RichText::new(hint).small().color(t.text_dim));
            show_error(ui, t, error);
            let changed = ident.as_deref().is_some_and(|i| i != name);
            buttons(ui, "Rename", false, changed, enter)
        }
        Dialog::DeleteFunction { name } => {
            title(ui, &format!("Delete \u{201c}{}\u{201d}?", pretty(name)));
            ui.label(format!("Apps calling /api/fn/{name} will get \u{201c}not found\u{201d}. The code is moved to functions/.deleted, so it can still be found."));
            show_error(ui, t, error);
            buttons(ui, "Delete function", true, true, false)
        }
        Dialog::TurnOffFunctions { remove_runtime } => {
            title(ui, "Turn off Functions?");
            ui.label("Your API stops running functions. Their code stays in the functions folder, so turning Functions on again brings them back.");
            ui.add_space(6.0);
            ui.checkbox(remove_runtime, "Also delete the downloaded helper (Bun) to free about 80 MB");
            show_error(ui, t, error);
            buttons(ui, "Turn off", true, true, false)
        }
        Dialog::EditField { table, field, label, kind, original_kind, required, links_to } => {
            title(ui, &format!("Change \u{201c}{}\u{201d}", pretty(field)));
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(label).desired_width(f32::INFINITY));
            let ident = to_ident(label);
            if let Some(id) = &ident
                && id != field
            {
                ui.label(
                    RichText::new(format!("Apps will use \u{201c}{id}\u{201d} instead of \u{201c}{field}\u{201d}."))
                        .small()
                        .color(t.text_dim),
                );
            }
            ui.add_space(6.0);
            ui.label("What goes in it?");
            egui::ComboBox::from_id_salt("edit_field_kind").selected_text(kind.label()).width(220.0).show_ui(
                ui,
                |ui| {
                    for k in FieldKind::ALL {
                        ui.selectable_value(kind, k, k.label());
                    }
                },
            );
            if *kind == FieldKind::Link {
                ui.add_space(6.0);
                ui.label("Which table does it link to?");
                let shown = links_to.as_deref().map_or_else(|| "Choose a table".to_owned(), pretty);
                egui::ComboBox::from_id_salt("edit_link_target").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                    for name in tables {
                        ui.selectable_value(links_to, Some(name.clone()), pretty(name));
                    }
                });
            }
            if kind != original_kind {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Existing values are converted. If some don't fit, nothing changes and Porchly tells you which rows to fix. A snapshot is saved first.",
                    )
                    .small()
                    .color(t.text_dim),
                );
            }
            ui.add_space(6.0);
            ui.checkbox(required, "Every row must fill this in");
            let ready = ident.is_some() && (*kind != FieldKind::Link || links_to.is_some());
            show_error(ui, t, error);
            ui.add_space(16.0);
            let mut outcome = Outcome::Keep;
            ui.horizontal(|ui| {
                let delete = egui::Button::new(RichText::new("Delete field\u{2026}").color(t.danger))
                    .stroke(egui::Stroke::new(1.0, t.danger));
                if ui.add(delete).clicked() {
                    outcome =
                        Outcome::Replace(Box::new(Dialog::DeleteField { table: table.clone(), field: field.clone() }));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save = ui.add_enabled_ui(ready, |ui| icons::button(ui, None, "Save", true)).inner.clicked();
                    if save || (enter && ready) {
                        outcome = Outcome::Confirm;
                    }
                    if icons::button(ui, None, "Cancel", false).clicked() {
                        outcome = Outcome::Cancel;
                    }
                });
            });
            outcome
        }
        Dialog::DeleteRow { id, .. } => {
            title(ui, &format!("Delete row {id}?"));
            ui.label("The row is removed right away. This can't be undone.");
            buttons(ui, "Delete row", true, true, false)
        }
        Dialog::DeleteTable { table } => {
            title(ui, &format!("Delete \u{201c}{}\u{201d}?", pretty(table)));
            ui.label("The table and all its rows are removed, and apps can no longer reach it.");
            ui.label(
                RichText::new("Porchly saves a snapshot of your project first, so it can be recovered.")
                    .color(t.text_dim),
            );
            buttons(ui, "Delete table", true, true, false)
        }
        Dialog::DeleteField { table, field } => {
            title(ui, &format!("Delete the field \u{201c}{}\u{201d}?", pretty(field)));
            ui.label(format!("Its values are removed from every row of {}.", pretty(table)));
            ui.label(
                RichText::new("Porchly saves a snapshot of your project first, so it can be recovered.")
                    .color(t.text_dim),
            );
            buttons(ui, "Delete field", true, true, false)
        }
    }
}

/// Runs the dialog's command. Returns whether it worked (and the dialog can close).
fn confirm(app: &mut PorchlyApp, dialog: &Dialog) -> bool {
    match dialog {
        Dialog::NewTable { label, with_name_field } => {
            let Some(name) = to_ident(label) else { return false };
            let fields = if *with_name_field { json!([{"name": "name", "type": "text"}]) } else { json!([]) };
            if app.run("table.create", json!({"name": name, "fields": fields})).is_none() {
                return false;
            }
            app.reload_tables();
            app.select_table(&name);
            app.info(format!("Made the table \u{201c}{}\u{201d}.", pretty(&name)));
            true
        }
        Dialog::AddField { table, label, kind, required, links_to } => {
            let Some(name) = to_ident(label) else { return false };
            let params = json!({"table": table, "name": name, "type": kind.as_str(), "required": required, "links_to": links_to});
            if app.run("field.add", params).is_none() {
                return false;
            }
            app.reload_tables();
            true
        }
        Dialog::NewKey { name, can_write } => {
            let Some(created) = app.run("api.key.create", json!({"name": name, "write": can_write})) else {
                return false;
            };
            let secret = created.get("secret").and_then(|s| s.as_str()).unwrap_or_default().to_owned();
            app.fresh_key = Some((name.trim().to_owned(), secret));
            app.show_share = true;
            app.reload_keys();
            true
        }
        Dialog::ImportCsv { path, columns, into_new, new_label, existing, .. } => {
            let table = if *into_new { to_ident(new_label) } else { existing.clone() };
            let Some(table) = table else { return false };
            let mut params = serde_json::Map::new();
            params.insert("path".into(), json!(path));
            params.insert("table".into(), json!(table));
            params.insert("create".into(), json!(into_new));
            if *into_new {
                let types: serde_json::Map<String, serde_json::Value> = columns
                    .iter()
                    .filter_map(|c| c.name.as_ref().map(|n| (n.clone(), json!(c.kind.as_str()))))
                    .collect();
                params.insert("types".into(), serde_json::Value::Object(types));
            }
            let Some(done) = app.run("csv.import", serde_json::Value::Object(params)) else { return false };
            let n = done.get("imported").and_then(|n| n.as_u64()).unwrap_or(0);
            app.reload_tables();
            app.select_table(&table);
            app.info(format!("Imported {n} rows into \u{201c}{}\u{201d}.", pretty(&table)));
            true
        }
        Dialog::Settings { rate, function_timeout, port, start_api, .. } => {
            let params = json!({
                "requests_per_second": rate.trim(),
                "function_timeout": function_timeout,
                "port": port.trim(),
                "start_api": start_api,
            });
            if app.run("settings.set", params).is_none() {
                return false;
            }
            app.refresh_status();
            app.info("Settings saved.");
            true
        }
        Dialog::PorchLight { allow_changes, .. } => {
            app.start_porch_light(*allow_changes);
            true
        }
        Dialog::FunctionsOptIn { .. } => {
            // Downloads in the background; the dialog shows progress and closes when done.
            app.dialog_error = None;
            app.start_enabling_functions();
            false
        }
        Dialog::NewFunction { label, template } => {
            let Some(name) = to_ident(label) else { return false };
            if app.run("function.create", json!({"name": name, "template": template})).is_none() {
                return false;
            }
            app.reload_functions();
            app.open_function(&name);
            true
        }
        Dialog::RenameFunction { name, label } => {
            let Some(new_name) = to_ident(label) else { return false };
            if app.run("function.rename", json!({"name": name, "new_name": new_name})).is_none() {
                return false;
            }
            app.reload_functions();
            app.open_function(&new_name);
            true
        }
        Dialog::DeleteFunction { name } => {
            if app.run("function.delete", json!({"name": name})).is_none() {
                return false;
            }
            app.open_function = None;
            app.reload_functions();
            app.info(format!("Deleted \u{201c}{}\u{201d}. Its code is in functions/.deleted.", pretty(name)));
            true
        }
        Dialog::TurnOffFunctions { remove_runtime } => {
            if app.run("functions.disable", json!({"remove_runtime": remove_runtime})).is_none() {
                return false;
            }
            app.open_function = None;
            app.reload_functions();
            app.info("Functions are off. Their code is still in the functions folder.");
            true
        }
        Dialog::EditField { table, field, label, kind, required, links_to, .. } => {
            let Some(new_name) = to_ident(label) else { return false };
            let mut params = serde_json::Map::new();
            params.insert("table".into(), json!(table));
            params.insert("name".into(), json!(field));
            params.insert("type".into(), json!(kind.as_str()));
            params.insert("required".into(), json!(required));
            if &new_name != field {
                params.insert("new_name".into(), json!(new_name));
            }
            if *kind == FieldKind::Link {
                params.insert("links_to".into(), json!(links_to));
            }
            let params = serde_json::Value::Object(params);
            if app.run("field.update", params).is_none() {
                return false;
            }
            app.reload_tables();
            app.info(format!("Saved the field \u{201c}{}\u{201d}.", pretty(&new_name)));
            true
        }
        Dialog::DeleteRow { table, id } => {
            let ok = app.run("row.delete", json!({"table": table, "id": id})).is_some();
            app.reload_rows();
            ok
        }
        Dialog::DeleteTable { table } => {
            let ok = app.run("table.drop", json!({"name": table})).is_some();
            app.reload_tables();
            if ok {
                app.info(format!("Deleted \u{201c}{}\u{201d}. A snapshot was saved first.", pretty(table)));
            }
            ok
        }
        Dialog::DeleteField { table, field } => {
            let ok = app.run("field.drop", json!({"table": table, "name": field})).is_some();
            app.reload_tables();
            ok
        }
    }
}

/// Why the last attempt failed, shown inside the dialog.
fn show_error(ui: &mut Ui, t: &Tokens, error: Option<&str>) {
    if let Some(e) = error {
        ui.add_space(8.0);
        egui::Frame::NONE.fill(t.danger_soft).corner_radius(6).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.label(RichText::new(e).color(t.danger));
        });
    }
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(20.0).strong());
    ui.add_space(8.0);
}

fn link_preview(ui: &mut Ui, t: &Tokens, ident: Option<&str>) {
    let text = match ident {
        Some(id) => format!("Apps will reach it at /api/{id}"),
        None => "Use at least one letter or number.".to_owned(),
    };
    ui.label(RichText::new(text).small().color(t.text_dim));
}

/// Puts the cursor in the first text field when the dialog opens.
fn focus_first(ui: &Ui, field: &egui::Response) {
    if ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
}

/// A main button and a "not now" button with chosen labels.
fn buttons_labeled(ui: &mut Ui, confirm_label: &str, cancel_label: &str) -> Outcome {
    ui.add_space(16.0);
    let mut outcome = Outcome::Keep;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if icons::button(ui, None, confirm_label, true).clicked() {
            outcome = Outcome::Confirm;
        }
        if icons::button(ui, None, cancel_label, false).clicked() {
            outcome = Outcome::Cancel;
        }
    });
    outcome
}

fn buttons(ui: &mut Ui, t: &Tokens, confirm_label: &str, danger: bool, can_confirm: bool, enter: bool) -> Outcome {
    ui.add_space(16.0);
    let mut outcome = Outcome::Keep;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let clicked = if danger {
            let b = egui::Button::new(RichText::new(confirm_label).color(egui::Color32::WHITE)).fill(t.danger);
            ui.add_enabled(can_confirm, b).clicked()
        } else {
            ui.add_enabled_ui(can_confirm, |ui| icons::button(ui, None, confirm_label, true)).inner.clicked()
        };
        if clicked || (enter && can_confirm) {
            outcome = Outcome::Confirm;
        }
        if icons::button(ui, None, "Cancel", false).clicked() {
            outcome = Outcome::Cancel;
        }
    });
    outcome
}
