//! The selected table, shown and edited like a spreadsheet.

use egui::{RichText, Ui};
use porchly_core::FieldKind;
use serde_json::{Map, Value, json};

use crate::icons::{self, Icon};
use crate::model::{FieldInfo, TableInfo, display, parse};
use crate::names::pretty;
use crate::theme::Tokens;
use crate::{Dialog, LinkChoices, PorchlyApp};

/// The first column: a checkbox for choosing the row, then its number.
const ID_WIDTH: f32 = 52.0;
const ROW_HEIGHT: f32 = 30.0;
/// The checkbox inside the first column.
const PICK_WIDTH: f32 = 18.0;

enum Action {
    Update {
        id: i64,
        field: String,
        value: Value,
    },
    Insert(Map<String, Value>),
    Ask(Dialog),
    Error(String),
    Export,
    LinkQuery(String, String),
    Page(u64),
    Search(String),
    /// Tick or untick one row.
    Pick(i64, bool),
    /// Tick or untick every row on this page.
    PickAll(bool),
}

pub(crate) fn show(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens) {
    let Some(table) = app.selected_table().cloned() else {
        welcome(app, ui, t);
        return;
    };
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.label(RichText::new(pretty(&table.name)).heading().strong());
        let count = match (app.total_rows, app.search.trim().is_empty()) {
            (n, false) => format!("{n} found"),
            (1, true) => "1 row".to_owned(),
            (n, true) => format!("{n} rows"),
        };
        ui.label(RichText::new(count).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::icon_button(ui, Icon::Trash, "Delete this table…").clicked() {
                actions.push(Action::Ask(Dialog::DeleteTable { table: table.name.clone() }));
            }
            if icons::icon_button(ui, Icon::Export, "Save this table as a CSV file…").clicked() {
                actions.push(Action::Export);
            }
            if icons::button(ui, Some(Icon::Plus), "Add field", false).clicked() {
                actions.push(Action::Ask(Dialog::add_field(&table.name)));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Apps reach this table at /api/{}", table.name)).small().color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut text = app.search.clone();
            let search =
                ui.add(egui::TextEdit::singleline(&mut text).hint_text("Search this table").desired_width(240.0));
            if search.changed() {
                actions.push(Action::Search(text));
            }
        });
    });
    ui.add_space(6.0);

    egui::Frame::NONE
        .fill(t.surface)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .max_height(ui.available_height() - 56.0)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    egui::Grid::new(("grid", &table.name))
                        .striped(true)
                        .spacing([10.0, 4.0])
                        .min_row_height(ROW_HEIGHT)
                        .show(ui, |ui| {
                            header(app, ui, t, &table, &mut actions);
                            rows(app, ui, t, &table, &mut actions);
                            new_row(app, ui, t, &table, &mut actions);
                        });
                });
        });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let picked = app.picked_rows.len();
        if picked > 0 {
            // Rows are ticked: offer what can be done with them.
            let what = if picked == 1 { "1 row selected".to_owned() } else { format!("{picked} rows selected") };
            ui.label(RichText::new(what).strong());
            if icons::button(ui, Some(Icon::Trash), "Delete selected\u{2026}", false).clicked() {
                let ids = app.picked_rows.iter().copied().collect();
                actions.push(Action::Ask(Dialog::DeleteRows { table: table.name.clone(), ids }));
            }
            if icons::button(ui, None, "Clear selection", false).clicked() {
                actions.push(Action::PickAll(false));
            }
            return;
        }
        let can_add = !table.fields.is_empty();
        if ui.add_enabled_ui(can_add, |ui| icons::button(ui, Some(Icon::Plus), "Add row", true)).inner.clicked() {
            add_row(app, &table, &mut actions);
        }
        ui.label(RichText::new("Fill in the bottom line, then press Add row (or Enter).").color(t.text_dim));
    });
    paging(app, ui, t, &mut actions);

    apply(app, &table, actions);
}

/// "Rows 101–200 of 1,234" with Previous / Next, when there's more than one page.
fn paging(app: &PorchlyApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let page = u64::from(crate::PAGE_SIZE);
    let searching = !app.search.trim().is_empty();
    if app.total_rows <= page && app.offset == 0 {
        if searching {
            let n = app.total_rows;
            ui.label(RichText::new(format!("{n} matching {}.", if n == 1 { "row" } else { "rows" })).color(t.text_dim));
        }
        return;
    }
    let first = app.offset + 1;
    let last = app.offset + app.rows.len() as u64;
    ui.horizontal(|ui| {
        let what = if searching { "matching rows" } else { "rows" };
        ui.label(
            RichText::new(format!("Showing {first}\u{2013}{last} of {} {what}.", app.total_rows)).color(t.text_dim),
        );
        if ui.add_enabled(app.offset > 0, egui::Button::new("Previous")).clicked() {
            actions.push(Action::Page(app.offset.saturating_sub(page)));
        }
        if ui.add_enabled(last < app.total_rows, egui::Button::new("Next")).clicked() {
            actions.push(Action::Page(app.offset + page));
        }
    });
}

/// The first cell of a row: a checkbox for choosing it (named for screen readers and tests),
/// then `text` (the row's number). Returns the new state when the checkbox was clicked.
fn pick_cell(ui: &mut Ui, t: &Tokens, on: Option<bool>, label: &str, text: &str) -> Option<bool> {
    let mut clicked = None;
    ui.allocate_ui_with_layout(
        egui::vec2(ID_WIDTH, ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            match on {
                Some(on) => {
                    let mut value = on;
                    let response = ui.add_sized([PICK_WIDTH, ROW_HEIGHT], egui::Checkbox::without_text(&mut value));
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), value, label)
                    });
                    if response.on_hover_text(label).changed() {
                        clicked = Some(value);
                    }
                }
                None => {
                    ui.allocate_exact_size(egui::vec2(PICK_WIDTH, ROW_HEIGHT), egui::Sense::hover());
                }
            }
            ui.add_sized(
                [ID_WIDTH - PICK_WIDTH - 2.0, ROW_HEIGHT],
                egui::Label::new(RichText::new(text).color(t.text_dim)),
            );
        },
    );
    clicked
}

fn header(app: &PorchlyApp, ui: &mut Ui, t: &Tokens, table: &TableInfo, actions: &mut Vec<Action>) {
    let all = !app.rows.is_empty()
        && app.rows.iter().filter_map(|r| r.get("id").and_then(Value::as_i64)).all(|id| app.picked_rows.contains(&id));
    let state = (!app.rows.is_empty()).then_some(all);
    if let Some(on) = pick_cell(ui, t, state, "Select all rows on this page", "#") {
        actions.push(Action::PickAll(on));
    }
    for f in &table.fields {
        let edit = || Dialog::edit_field(&table.name, &f.name, f.kind, f.required, f.links_to.clone());
        let response = field_header(ui, t, f);
        if response.clicked() {
            actions.push(Action::Ask(edit()));
        }
        response.context_menu(|ui| {
            if ui.button("Change this field\u{2026}").clicked() {
                actions.push(Action::Ask(edit()));
                ui.close();
            }
            if ui.button("Delete this field\u{2026}").clicked() {
                actions.push(Action::Ask(Dialog::DeleteField { table: table.name.clone(), field: f.name.clone() }));
                ui.close();
            }
        });
    }
    ui.label("");
    ui.end_row();
}

/// A column header: the field's name and type, with a pencil showing it can be changed.
/// Clicking it opens "Change field" (rename, change type, delete).
fn field_header(ui: &mut Ui, t: &Tokens, f: &FieldInfo) -> egui::Response {
    let name = pretty(&f.name);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width(f.kind), 40.0), egui::Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect.expand(2.0), 6.0, t.hover);
    }
    let pencil =
        egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0, rect.top() + 11.0), egui::Vec2::splat(13.0));
    icons::paint(ui.painter(), pencil, Icon::Edit, if hovered { t.accent } else { t.text_dim.gamma_multiply(0.7) });
    // The pencil sits on the name's line; the type line below can use the full width.
    let name_clip = egui::Rect::from_min_max(rect.min, egui::pos2(pencil.left() - 4.0, rect.bottom()));
    let painter = ui.painter().with_clip_rect(name_clip);
    let body = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    let title = format!("{name}{}", if f.required { " *" } else { "" });
    painter.text(rect.left_top() + egui::vec2(2.0, 2.0), egui::Align2::LEFT_TOP, title, body, t.text);
    let kind = match &f.links_to {
        Some(target) => format!("Link to {}", pretty(target)),
        None => f.kind.label().to_owned(),
    };
    ui.painter().with_clip_rect(rect).text(
        rect.left_top() + egui::vec2(2.0, 22.0),
        egui::Align2::LEFT_TOP,
        kind,
        small,
        t.text_dim,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Change field {name}")));
    response.on_hover_text("Change or delete this field")
}

fn rows(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens, table: &TableInfo, actions: &mut Vec<Action>) {
    let options = &app.link_options;
    let queries = &app.link_queries;
    let label_fields: std::collections::HashMap<&str, Option<String>> =
        table.fields.iter().filter_map(|f| f.links_to.as_deref().map(|t| (t, app.label_field(t)))).collect();
    let edits = &mut app.edits;
    for row in &app.rows {
        let Some(id) = row.get("id").and_then(Value::as_i64) else { continue };
        let picked = Some(app.picked_rows.contains(&id));
        if let Some(on) = pick_cell(ui, t, picked, &format!("Select row {id}"), &id.to_string()) {
            actions.push(Action::Pick(id, on));
        }
        for f in &table.fields {
            let current = row.get(&f.name);
            if f.kind == FieldKind::Boolean {
                let mut on = current.and_then(Value::as_bool).unwrap_or(false);
                let cell = ui.add_sized([width(f.kind), ROW_HEIGHT], egui::Checkbox::without_text(&mut on));
                if cell.changed() {
                    actions.push(Action::Update { id, field: f.name.clone(), value: Value::Bool(on) });
                }
                continue;
            }
            if f.kind == FieldKind::Link {
                let current = current.and_then(Value::as_i64);
                let target = f.links_to.as_deref().unwrap_or_default();
                // The linked row came along with this row (expand), so its name is known.
                let shown = match (current, row.get("expand").and_then(|e| e.get(&f.name))) {
                    (None, _) => "\u{2014}".to_owned(),
                    (Some(link), Some(linked)) if !linked.is_null() => {
                        crate::row_label(linked, label_fields.get(target).cloned().flatten().as_deref(), link)
                    }
                    (Some(link), _) => format!("Row {link}"),
                };
                let empty = LinkChoices::default();
                let choices = options.get(target).unwrap_or(&empty);
                let query = queries.get(target).map(String::as_str).unwrap_or("");
                let out = link_picker(ui, (id, &f.name), shown, choices, query, current);
                if let Some(picked) = out.picked {
                    actions.push(Action::Update {
                        id,
                        field: f.name.clone(),
                        value: picked.map_or(Value::Null, Value::from),
                    });
                }
                if let Some(q) = out.query {
                    actions.push(Action::LinkQuery(target.to_owned(), q));
                }
                continue;
            }
            let key = (id, f.name.clone());
            let original = display(current);
            let mut text = edits.get(&key).cloned().unwrap_or_else(|| original.clone());
            let cell = ui.add_sized(
                [width(f.kind), ROW_HEIGHT - 4.0],
                egui::TextEdit::singleline(&mut text).hint_text(hint(f)).margin(egui::vec2(6.0, 4.0)),
            );
            if cell.changed() {
                edits.insert(key.clone(), text);
            }
            if cell.lost_focus()
                && let Some(typed) = edits.remove(&key)
                && typed != original
            {
                match parse(f.kind, &typed) {
                    Ok(value) => actions.push(Action::Update { id, field: f.name.clone(), value }),
                    Err(e) => actions.push(Action::Error(format!("{}: {e}", pretty(&f.name)))),
                }
            }
        }
        if icons::icon_button(ui, Icon::Trash, "Delete this row…").clicked() {
            actions.push(Action::Ask(Dialog::DeleteRow { table: table.name.clone(), id }));
        }
        ui.end_row();
    }
}

fn new_row(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens, table: &TableInfo, actions: &mut Vec<Action>) {
    let mut enter_pressed = false;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ID_WIDTH, ROW_HEIGHT), egui::Sense::hover());
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(12.0)),
        Icon::Plus,
        t.accent,
    );
    for f in &table.fields {
        if f.kind == FieldKind::Boolean {
            let on = app.new_row_flags.entry(f.name.clone()).or_insert(false);
            ui.add_sized([width(f.kind), ROW_HEIGHT], egui::Checkbox::without_text(on));
            continue;
        }
        if f.kind == FieldKind::Link {
            let current = app.new_row.get(&f.name).and_then(|s| s.parse::<i64>().ok());
            let target = f.links_to.as_deref().unwrap_or_default();
            let empty = LinkChoices::default();
            let choices = app.link_options.get(target).unwrap_or(&empty);
            let shown = match current {
                None => "\u{2014}".to_owned(),
                Some(id) => app.new_row_link_labels.get(&f.name).cloned().unwrap_or_else(|| format!("Row {id}")),
            };
            let query = app.link_queries.get(target).cloned().unwrap_or_default();
            let out = link_picker(ui, (0, &f.name), shown, choices, &query, current);
            if let Some(picked) = out.picked {
                app.new_row.insert(f.name.clone(), picked.map(|id| id.to_string()).unwrap_or_default());
                let label =
                    picked.and_then(|id| choices.options.iter().find(|(i, _)| *i == id)).map(|(_, l)| l.clone());
                match label {
                    Some(l) => app.new_row_link_labels.insert(f.name.clone(), l),
                    None => app.new_row_link_labels.remove(&f.name),
                };
            }
            if let Some(q) = out.query {
                actions.push(Action::LinkQuery(target.to_owned(), q));
            }
            continue;
        }
        let text = app.new_row.entry(f.name.clone()).or_default();
        let cell = ui.add_sized(
            [width(f.kind), ROW_HEIGHT - 4.0],
            egui::TextEdit::singleline(text).hint_text(pretty(&f.name)).margin(egui::vec2(6.0, 4.0)),
        );
        if cell.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            enter_pressed = true;
        }
    }
    if enter_pressed {
        add_row(app, table, actions);
    }
    ui.end_row();
}

/// Reads the "new row" line into values, or reports what's wrong with it.
fn new_row_values(app: &PorchlyApp, table: &TableInfo) -> Result<Map<String, Value>, String> {
    let mut values = Map::new();
    for f in &table.fields {
        if f.kind == FieldKind::Boolean {
            values.insert(f.name.clone(), Value::Bool(app.new_row_flags.get(&f.name).copied().unwrap_or(false)));
            continue;
        }
        let typed = app.new_row.get(&f.name).map(String::as_str).unwrap_or("");
        match parse(f.kind, typed) {
            Ok(Value::Null) => {}
            Ok(v) => {
                values.insert(f.name.clone(), v);
            }
            Err(e) => return Err(format!("{}: {e}", pretty(&f.name))),
        }
    }
    Ok(values)
}

fn add_row(app: &PorchlyApp, table: &TableInfo, actions: &mut Vec<Action>) {
    match new_row_values(app, table) {
        Ok(values) => actions.push(Action::Insert(values)),
        Err(e) => actions.push(Action::Error(e)),
    }
}

fn apply(app: &mut PorchlyApp, table: &TableInfo, actions: Vec<Action>) {
    let mut reload = false;
    for action in actions {
        match action {
            Action::Update { id, field, value } => {
                let mut values = Map::new();
                values.insert(field, value);
                app.run("row.update", json!({"table": table.name, "id": id, "values": values}));
                reload = true;
            }
            Action::Insert(values) => {
                if app.run("row.insert", json!({"table": table.name, "values": values})).is_some() {
                    app.new_row.clear();
                    app.new_row_flags.clear();
                    app.new_row_link_labels.clear();
                    app.info("Row added.");
                }
                reload = true;
            }
            Action::Ask(dialog) => app.open_dialog(dialog),
            Action::Pick(id, on) => {
                if on {
                    app.picked_rows.insert(id);
                } else {
                    app.picked_rows.remove(&id);
                }
            }
            Action::PickAll(on) => {
                app.picked_rows.clear();
                if on {
                    app.picked_rows.extend(app.rows.iter().filter_map(|r| r.get("id").and_then(Value::as_i64)));
                }
            }
            Action::Error(e) => app.error(e),
            Action::LinkQuery(target, text) => app.set_link_query(&target, text),
            Action::Page(offset) => app.go_to_page(offset),
            Action::Search(text) => app.set_search(&text),
            Action::Export => {
                let picked = rfd::FileDialog::new()
                    .set_file_name(format!("{}.csv", table.name))
                    .add_filter("CSV", &["csv"])
                    .save_file();
                if let Some(path) = picked
                    && let Some(done) =
                        app.run("csv.export", json!({"table": table.name, "path": path, "overwrite": true}))
                {
                    let rows = done.get("rows").and_then(Value::as_u64).unwrap_or(0);
                    app.info(format!("Saved {rows} rows to {}.", path.display()));
                }
            }
        }
    }
    if reload {
        app.reload_rows();
    }
}

/// What happened in a link dropdown this frame.
#[derive(Default)]
struct PickerOut {
    /// A new choice (`Some(None)` means "no link").
    picked: Option<Option<i64>>,
    /// New text in the dropdown's search box.
    query: Option<String>,
}

/// A dropdown of the linked table's rows, with a search box for tables too big to list.
/// `shown` is the name of the row currently linked.
fn link_picker(
    ui: &mut Ui,
    salt: (i64, &str),
    shown: String,
    choices: &LinkChoices,
    query: &str,
    current: Option<i64>,
) -> PickerOut {
    let mut out = PickerOut::default();
    let mut selected = current;
    egui::ComboBox::from_id_salt(("link", salt.0, salt.1))
        .selected_text(shown)
        .width(width(FieldKind::Link) - 8.0)
        .height(340.0)
        // Typing in the search box must not close the list; picking a row does (below).
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let mut text = query.to_owned();
            let search =
                ui.add(egui::TextEdit::singleline(&mut text).hint_text("Search\u{2026}").desired_width(f32::INFINITY));
            if search.changed() {
                out.query = Some(text);
            }
            if ui.selectable_value(&mut selected, None, "\u{2014} none").clicked() {
                out.picked = Some(None);
                ui.close();
            }
            for (id, label) in &choices.options {
                if ui.selectable_value(&mut selected, Some(*id), label).clicked() {
                    out.picked = Some(Some(*id));
                    ui.close();
                }
            }
            if choices.more > 0 {
                ui.label(RichText::new(format!("{} more: type to search", choices.more)).small().weak());
            }
        });
    out.picked = out.picked.filter(|p| *p != current);
    out
}

fn width(kind: FieldKind) -> f32 {
    match kind {
        FieldKind::Text => 190.0,
        FieldKind::Integer | FieldKind::Number => 104.0,
        FieldKind::Boolean => 96.0,
        FieldKind::Link => 180.0,
        FieldKind::Datetime => 130.0,
    }
}

fn hint(f: &FieldInfo) -> &'static str {
    match f.kind {
        FieldKind::Datetime => "2026-01-15",
        _ => "",
    }
}

/// Shown when the project has no tables yet.
fn welcome(app: &mut PorchlyApp, ui: &mut Ui, t: &Tokens) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(20.0));
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(64.0), egui::Sense::hover());
        icons::paint(ui.painter(), rect, Icon::Lamp, t.accent);
        ui.add_space(8.0);
        ui.label(RichText::new("Welcome to Porchly").size(26.0).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("Make a table, and Porchly turns it into an API that apps and websites can use.")
                .color(t.text_dim),
        );
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            let total = 196.0 + 186.0 + 8.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            if icons::button(ui, Some(Icon::Plus), "Make your first table", true).clicked() {
                app.open_dialog(Dialog::new_table());
            }
            if icons::button(ui, Some(Icon::Table), "Add a sample table", false).clicked()
                && app.run("sample.load", json!({})).is_some()
            {
                app.reload_tables();
                app.select_table("recipes");
                app.info("Added a sample table of recipes. Change anything you like.");
            }
        });
    });
}
