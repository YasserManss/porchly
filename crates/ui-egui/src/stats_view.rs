//! The Stats window: how much the project is used and how big it is. Headline numbers as
//! tiles, requests per minute over the last hour as one small column chart, and plain
//! tables by app and by table. It refreshes itself while open.

use egui::{RichText, Ui};
use serde_json::Value;

use crate::names::pretty;
use crate::theme::Tokens;

/// Rows shown in each table (the busiest first).
const TOP: usize = 8;
const CHART_HEIGHT: f32 = 84.0;
/// The gap between columns, in the surface colour.
const GAP: f32 = 2.0;

pub(crate) fn show(ui: &mut Ui, t: &Tokens, data: &Value) {
    let n = |path: &[&str]| path.iter().try_fold(data, |v, k| v.get(*k)).and_then(Value::as_u64).unwrap_or(0);
    ui.label(
        RichText::new(format!(
            "Counted since Porchly opened, {} ago. These numbers stay on this computer.",
            duration(n(&["seconds"]))
        ))
        .small()
        .color(t.text_dim),
    );
    ui.add_space(10.0);

    // The headline numbers.
    ui.columns(4, |cols| {
        let tiles = [
            ("Requests", compact(n(&["requests", "total"]))),
            ("Rows", compact(n(&["project", "rows"]))),
            ("Tables", compact(n(&["project", "tables"]))),
            ("Size on disk", bytes(n(&["project", "size_bytes"]))),
        ];
        for (col, (label, value)) in cols.iter_mut().zip(tiles) {
            tile(col, t, label, &value);
        }
    });
    ui.add_space(14.0);

    let minutes: Vec<u64> = data
        .get("per_minute")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_u64().unwrap_or(0)).collect())
        .unwrap_or_default();
    chart(ui, t, &minutes);
    ui.add_space(10.0);

    // What went wrong, in words; a number alone never carries it.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        trouble(ui, t, "Refused (no key, or a wrong one)", n(&["requests", "refused"]));
        trouble(ui, t, "Told to slow down", n(&["requests", "rate_limited"]));
        trouble(ui, t, "Errors", n(&["requests", "failed"]));
        let quiet = |ui: &mut Ui, label: &str, value: u64| {
            // Each item stays in one piece when the row wraps.
            let text = RichText::new(format!("{label}: {}", compact(value))).small().color(t.text_dim);
            ui.add(egui::Label::new(text).extend());
        };
        quiet(ui, "Changes made", n(&["requests", "writes"]));
        quiet(ui, "Through the porch light", n(&["requests", "public"]));
        quiet(ui, "By AI assistants", n(&["requests", "mcp"]));
    });
    ui.add_space(14.0);

    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        left.label(RichText::new("By app").strong());
        let keys = data.get("keys").and_then(Value::as_array).cloned().unwrap_or_default();
        if keys.is_empty() {
            left.label(RichText::new("No app has used a key yet.").color(t.text_dim));
        }
        egui::Grid::new("stats_keys").num_columns(3).spacing([16.0, 3.0]).min_row_height(20.0).show(left, |ui| {
            if !keys.is_empty() {
                head(ui, t, &["Key", "Requests", "Last used"]);
            }
            for k in keys.iter().take(TOP) {
                ui.label(k.get("name").and_then(Value::as_str).unwrap_or_default());
                number(ui, k.get("requests").and_then(Value::as_u64).unwrap_or(0));
                let ago = k.get("last_used_seconds_ago").and_then(Value::as_u64).unwrap_or(0);
                ui.label(RichText::new(format!("{} ago", duration(ago))).color(t.text_dim));
                ui.end_row();
            }
        });
        more(left, t, keys.len());

        right.label(RichText::new("By table").strong());
        let mut tables = data.get("tables").and_then(Value::as_array).cloned().unwrap_or_default();
        let of = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        tables.sort_by_key(|v| std::cmp::Reverse((of(v, "requests"), of(v, "rows"))));
        if tables.is_empty() {
            right.label(RichText::new("No tables yet.").color(t.text_dim));
        }
        egui::Grid::new("stats_tables").num_columns(3).spacing([16.0, 3.0]).min_row_height(20.0).show(right, |ui| {
            if !tables.is_empty() {
                head(ui, t, &["Table", "Rows", "Requests"]);
            }
            for table in tables.iter().take(TOP) {
                ui.label(pretty(table.get("name").and_then(Value::as_str).unwrap_or_default()));
                number(ui, of(table, "rows"));
                number(ui, of(table, "requests"));
                ui.end_row();
            }
        });
        more(right, t, tables.len());
    });

    let functions = data.get("functions").and_then(Value::as_array).cloned().unwrap_or_default();
    if !functions.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new("Functions").strong());
        egui::Grid::new("stats_functions").num_columns(2).spacing([16.0, 3.0]).min_row_height(20.0).show(ui, |ui| {
            head(ui, t, &["Function", "Times run"]);
            for f in functions.iter().take(TOP) {
                ui.label(pretty(f.get("name").and_then(Value::as_str).unwrap_or_default()));
                number(ui, f.get("calls").and_then(Value::as_u64).unwrap_or(0));
                ui.end_row();
            }
        });
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(format!(
            "{} · {} ({}) · {}",
            plural(n(&["project", "keys"]), "key", "keys"),
            plural(n(&["project", "backups"]), "backup", "backups"),
            bytes(n(&["project", "backups_bytes"])),
            plural(n(&["project", "changes"]), "change you can undo", "changes you can undo"),
        ))
        .small()
        .color(t.text_dim),
    );
}

/// One headline number: a quiet label over a large value.
fn tile(ui: &mut Ui, t: &Tokens, label: &str, value: &str) {
    egui::Frame::NONE
        .fill(t.row_alt)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(label).small().color(t.text_dim));
            ui.label(RichText::new(value).size(24.0).strong().color(t.text));
        });
}

/// Requests per minute for the last hour: one series, so one colour and no legend (the
/// title names it). Each column answers the pointer with its own number.
fn chart(ui: &mut Ui, t: &Tokens, minutes: &[u64]) {
    let total: u64 = minutes.iter().sum();
    let peak = minutes.iter().copied().max().unwrap_or(0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Requests in the last hour").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let note = if total == 0 {
                "none yet".to_owned()
            } else {
                format!("{} in all, {} in the busiest minute", compact(total), compact(peak))
            };
            ui.label(RichText::new(note).small().color(t.text_dim));
        });
    });
    let (rect, whole) = ui.allocate_exact_size(egui::vec2(ui.available_width(), CHART_HEIGHT), egui::Sense::hover());
    // For screen readers, the chart as a sentence.
    whole.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            format!("Requests in the last hour: {total} in all, {peak} in the busiest minute"),
        )
    });
    let painter = ui.painter_at(rect);
    let base = rect.bottom() - 1.0;
    painter.line_segment(
        [egui::pos2(rect.left(), base), egui::pos2(rect.right(), base)],
        egui::Stroke::new(1.0, t.border),
    );
    let count = minutes.len().max(1);
    let slot = rect.width() / count as f32;
    let bar = (slot - GAP).max(1.0);
    let room = CHART_HEIGHT - 6.0;
    for (i, value) in minutes.iter().enumerate() {
        let left = rect.left() + slot * i as f32;
        let column = egui::Rect::from_min_max(egui::pos2(left, rect.top()), egui::pos2(left + slot, rect.bottom()));
        // The whole column is the target, not just the bar: thin bars are hard to point at.
        let response = ui.interact(column, ui.id().with(("minute", i)), egui::Sense::hover());
        if response.hovered() {
            painter.rect_filled(column, 2.0, t.hover);
        }
        if *value > 0 {
            // Never less than 2 px: a minute with one request must still show.
            let height = (room * *value as f32 / peak.max(1) as f32).max(2.0);
            let shape = egui::Rect::from_min_max(
                egui::pos2(left + GAP / 2.0, base - height),
                egui::pos2(left + GAP / 2.0 + bar, base),
            );
            let round = (bar / 2.0).min(3.0) as u8;
            painter.rect_filled(shape, egui::CornerRadius { nw: round, ne: round, sw: 0, se: 0 }, t.chart);
        }
        let ago = count - 1 - i;
        let when = match ago {
            0 => "this minute".to_owned(),
            1 => "1 minute ago".to_owned(),
            m => format!("{m} minutes ago"),
        };
        response.on_hover_text(format!("{}, {when}", plural(*value, "request", "requests")));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("60 minutes ago").small().color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("now").small().color(t.text_dim));
        });
    });
}

/// A count of something that went wrong: marked when it's not zero, always with words.
fn trouble(ui: &mut Ui, t: &Tokens, label: &str, value: u64) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if value > 0 {
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, t.danger);
        }
        let text = RichText::new(format!("{label}: {}", compact(value))).small();
        ui.label(if value > 0 { text.color(t.text) } else { text.color(t.text_dim) });
    });
}

/// Column titles. Number columns are padded like the numbers under them.
fn head(ui: &mut Ui, t: &Tokens, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        if i == 0 || *name == "Last used" {
            ui.label(RichText::new(*name).small().color(t.text_dim));
        } else {
            ui.label(RichText::new(format!("{name:>NUMBER_WIDTH$}")).monospace().size(11.5).color(t.text_dim));
        }
    }
    ui.end_row();
}

/// Width of a number column, in characters.
const NUMBER_WIDTH: usize = 9;

/// A number in a table column: exact, in digits of equal width, padded on the left so the
/// column lines up on the right.
fn number(ui: &mut Ui, value: u64) {
    ui.label(RichText::new(format!("{:>NUMBER_WIDTH$}", thousands(value))).monospace());
}

fn more(ui: &mut Ui, t: &Tokens, len: usize) {
    if len > TOP {
        ui.label(RichText::new(format!("and {} more", len - TOP)).small().color(t.text_dim));
    }
}

/// 1,284 with separators.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A headline number: exact up to 9,999, then 12.9K, 4.2M.
pub(crate) fn compact(n: u64) -> String {
    match n {
        0..=9_999 => thousands(n),
        10_000..=999_999 => format!("{:.1}K", n as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1_000_000.0),
        _ => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

/// 532 KB, 4.2 MB.
pub(crate) fn bytes(n: u64) -> String {
    match n {
        0..=1_023 => format!("{n} bytes"),
        1_024..=1_048_575 => format!("{} KB", n / 1_024),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", n as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", n as f64 / 1_073_741_824.0),
    }
}

/// "a moment", "5 minutes", "2 hours", "3 days".
pub(crate) fn duration(seconds: u64) -> String {
    match seconds {
        0..=59 => "a moment".to_owned(),
        60..=3_599 => plural(seconds / 60, "minute", "minutes"),
        3_600..=86_399 => plural(seconds / 3_600, "hour", "hours"),
        _ => plural(seconds / 86_400, "day", "days"),
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{} {many}", thousands(n)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_well() {
        assert_eq!((thousands(0), thousands(999), thousands(1_284)), ("0".into(), "999".into(), "1,284".into()));
        assert_eq!(thousands(12_345_678), "12,345,678");
        assert_eq!(
            (compact(9_999), compact(12_900), compact(4_200_000)),
            ("9,999".into(), "12.9K".into(), "4.2M".into())
        );
        assert_eq!(
            (bytes(512), bytes(545_000), bytes(4_400_000)),
            ("512 bytes".into(), "532 KB".into(), "4.2 MB".into())
        );
        assert_eq!(
            (duration(5), duration(60), duration(7_300), duration(200_000)),
            ("a moment".into(), "1 minute".into(), "2 hours".into(), "2 days".into())
        );
    }
}
