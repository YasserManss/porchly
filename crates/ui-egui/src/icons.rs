//! Icons drawn in code (original, no asset files), and small buttons that use them.

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Table,
    Plus,
    Trash,
    Key,
    Lamp,
    Copy,
    Close,
    Share,
    Edit,
    Import,
    Export,
    /// Functions: `</>`.
    Code,
    /// Settings: a gear.
    Gear,
}

/// Paints `icon` inside `rect` with `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let stroke = Stroke::new((s / 11.0).max(1.4), color);
    let p = |x: f32, y: f32| pos2(c.x + x * s / 2.0, c.y + y * s / 2.0);
    match icon {
        Icon::Table => {
            let r = Rect::from_min_max(p(-0.8, -0.7), p(0.8, 0.7));
            painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment([p(-0.8, -0.2), p(0.8, -0.2)], stroke);
            painter.line_segment([p(-0.8, 0.25), p(0.8, 0.25)], stroke);
            painter.line_segment([p(-0.2, -0.2), p(-0.2, 0.7)], stroke);
        }
        Icon::Plus => {
            painter.line_segment([p(-0.7, 0.0), p(0.7, 0.0)], stroke);
            painter.line_segment([p(0.0, -0.7), p(0.0, 0.7)], stroke);
        }
        Icon::Trash => {
            painter.line_segment([p(-0.75, -0.5), p(0.75, -0.5)], stroke);
            painter.line_segment([p(-0.25, -0.75), p(0.25, -0.75)], stroke);
            let body = [p(-0.55, -0.5), p(-0.42, 0.8), p(0.42, 0.8), p(0.55, -0.5)];
            painter.add(Shape::line(body.to_vec(), stroke));
            painter.line_segment([p(-0.15, -0.2), p(-0.12, 0.5)], stroke);
            painter.line_segment([p(0.15, -0.2), p(0.12, 0.5)], stroke);
        }
        Icon::Key => {
            painter.circle_stroke(p(-0.4, 0.0), s * 0.2, stroke);
            painter.line_segment([p(-0.0, 0.0), p(0.8, 0.0)], stroke);
            painter.line_segment([p(0.55, 0.0), p(0.55, 0.3)], stroke);
            painter.line_segment([p(0.8, 0.0), p(0.8, 0.35)], stroke);
        }
        Icon::Lamp => {
            // A porch lantern: cap, glass, base, and a glow.
            painter.line_segment([p(-0.45, -0.6), p(0.45, -0.6)], stroke);
            painter.line_segment([p(0.0, -0.85), p(0.0, -0.6)], stroke);
            let glass = [p(-0.4, -0.6), p(-0.3, 0.45), p(0.3, 0.45), p(0.4, -0.6)];
            painter.add(Shape::closed_line(glass.to_vec(), stroke));
            painter.line_segment([p(-0.4, 0.7), p(0.4, 0.7)], stroke);
            painter.circle_filled(p(0.0, -0.05), s * 0.12, color);
        }
        Icon::Copy => {
            let back = Rect::from_min_max(p(-0.7, -0.7), p(0.3, 0.35));
            let front = Rect::from_min_max(p(-0.3, -0.3), p(0.7, 0.75));
            painter.rect_stroke(back, 2.0, stroke, egui::StrokeKind::Middle);
            painter.rect_filled(front, 2.0, painter.ctx().global_style().visuals.window_fill);
            painter.rect_stroke(front, 2.0, stroke, egui::StrokeKind::Middle);
        }
        Icon::Close => {
            painter.line_segment([p(-0.55, -0.55), p(0.55, 0.55)], stroke);
            painter.line_segment([p(0.55, -0.55), p(-0.55, 0.55)], stroke);
        }
        Icon::Edit => {
            // A pencil, tip at the bottom left.
            let body = [p(-0.55, 0.55), p(-0.45, 0.2), p(0.35, -0.6), p(0.6, -0.35), p(-0.2, 0.45)];
            painter.add(Shape::closed_line(body.to_vec(), stroke));
            painter.line_segment([p(0.15, -0.4), p(0.4, -0.15)], stroke);
        }
        Icon::Import | Icon::Export => {
            // A tray with an arrow going in (import) or out (export).
            let tray = [p(-0.75, 0.15), p(-0.75, 0.75), p(0.75, 0.75), p(0.75, 0.15)];
            painter.add(Shape::line(tray.to_vec(), stroke));
            let (from, to) =
                if icon == Icon::Import { (p(0.0, -0.8), p(0.0, 0.35)) } else { (p(0.0, 0.35), p(0.0, -0.8)) };
            painter.line_segment([from, to], stroke);
            let tip = to;
            let dir = if icon == Icon::Import { -1.0 } else { 1.0 };
            painter.line_segment([tip, pos2(tip.x - s * 0.18, tip.y + dir * s * 0.18)], stroke);
            painter.line_segment([tip, pos2(tip.x + s * 0.18, tip.y + dir * s * 0.18)], stroke);
        }
        Icon::Gear => {
            painter.circle_stroke(c, s * 0.2, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (sin, cos) = a.sin_cos();
                painter.line_segment([p(cos * 0.5, sin * 0.5), p(cos * 0.8, sin * 0.8)], stroke);
            }
            painter.circle_stroke(c, s * 0.3, stroke);
        }
        Icon::Code => {
            painter.add(Shape::line(vec![p(-0.35, -0.55), p(-0.85, 0.0), p(-0.35, 0.55)], stroke));
            painter.add(Shape::line(vec![p(0.35, -0.55), p(0.85, 0.0), p(0.35, 0.55)], stroke));
            painter.line_segment([p(0.15, -0.75), p(-0.15, 0.75)], stroke);
        }
        Icon::Share => {
            let a = p(0.55, -0.6);
            let b = p(-0.55, 0.0);
            let d = p(0.55, 0.6);
            painter.line_segment([a, b], stroke);
            painter.line_segment([b, d], stroke);
            for q in [a, b, d] {
                painter.circle_filled(q, s * 0.17, color);
            }
        }
    }
}

/// A square button showing only an icon, with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, t.hover);
    }
    let color = if response.hovered() { t.text } else { t.text_dim };
    paint(ui.painter(), rect.shrink(6.0), icon, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip));
    response.on_hover_text(tooltip)
}

/// A button with an icon before its label. `primary` uses the porch-light colour.
pub fn button(ui: &mut Ui, icon: Option<Icon>, label: &str, primary: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (fg, bg, border) = if primary { (t.on_accent, t.accent, t.accent) } else { (t.text, t.surface, t.border) };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, fg);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 22.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let fill = match (primary, response.hovered()) {
        (true, true) => t.accent.gamma_multiply(0.88),
        (false, true) => t.hover,
        _ => bg,
    };
    ui.painter().rect(rect, 7.0, fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let mut x = rect.left() + 11.0;
    if let Some(i) = icon {
        let r = Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(15.0));
        paint(ui.painter(), r, i, fg);
        x += icon_w;
    }
    let text_pos = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, fg);
    response
}

/// A small coloured dot followed by text: the porch light status.
pub fn status_dot(ui: &mut Ui, on: bool, text: &str) {
    let t = Tokens::get(ui.ctx());
    let dot = |ui: &mut Ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        if on {
            ui.painter().circle_filled(rect.center(), 6.0, t.ok_soft);
        }
        ui.painter().circle_filled(rect.center(), 4.0, if on { t.ok } else { t.off });
    };
    let label = egui::RichText::new(text).color(if on { t.text } else { t.text_dim });
    ui.horizontal(|ui| {
        // In a right-to-left row the first item lands rightmost, so draw in reverse.
        if ui.layout().prefer_right_to_left() {
            ui.label(label);
            dot(ui);
        } else {
            dot(ui);
            ui.label(label);
        }
    });
}
