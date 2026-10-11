//! Colours and text sizes. Warm "porch light" palette, original to Porchly. Every colour the UI
//! uses comes from [`Tokens`]; light and dark follow the system setting.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Main background.
    pub bg: Color32,
    /// Cards, the grid and dialogs.
    pub surface: Color32,
    /// Sidebar and top bar.
    pub chrome: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// The porch light: primary buttons, selection, "on" states.
    pub accent: Color32,
    /// Text drawn on top of `accent`.
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub hover: Color32,
    pub row_alt: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub off: Color32,
    /// Marks in charts (bars). Checked for both surfaces with the dataviz palette validator:
    /// the dark accent is too light for marks, so dark mode has its own step.
    pub chart: Color32,
}

impl Tokens {
    pub fn light() -> Self {
        Tokens {
            dark: false,
            bg: Color32::from_rgb(0xFA, 0xF8, 0xF5),
            surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            chrome: Color32::from_rgb(0xF2, 0xEE, 0xE8),
            border: Color32::from_rgb(0xE2, 0xDC, 0xD2),
            text: Color32::from_rgb(0x24, 0x21, 0x1E),
            text_dim: Color32::from_rgb(0x6B, 0x65, 0x5E),
            accent: Color32::from_rgb(0xB0, 0x65, 0x07),
            on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            accent_soft: Color32::from_rgb(0xFC, 0xEE, 0xD6),
            hover: Color32::from_rgb(0xEC, 0xE6, 0xDD),
            row_alt: Color32::from_rgb(0xFB, 0xF9, 0xF6),
            ok: Color32::from_rgb(0x23, 0x7A, 0x3B),
            ok_soft: Color32::from_rgb(0xE3, 0xF4, 0xE7),
            danger: Color32::from_rgb(0xB4, 0x23, 0x18),
            danger_soft: Color32::from_rgb(0xFD, 0xE8, 0xE6),
            off: Color32::from_rgb(0x9A, 0x93, 0x8A),
            chart: Color32::from_rgb(0xB0, 0x65, 0x07),
        }
    }

    pub fn dark() -> Self {
        Tokens {
            dark: true,
            bg: Color32::from_rgb(0x1C, 0x1A, 0x18),
            surface: Color32::from_rgb(0x25, 0x22, 0x1F),
            chrome: Color32::from_rgb(0x2B, 0x28, 0x24),
            border: Color32::from_rgb(0x3D, 0x38, 0x33),
            text: Color32::from_rgb(0xEE, 0xEA, 0xE4),
            text_dim: Color32::from_rgb(0xA8, 0xA0, 0x96),
            accent: Color32::from_rgb(0xF2, 0xA7, 0x3B),
            on_accent: Color32::from_rgb(0x24, 0x1A, 0x0C),
            accent_soft: Color32::from_rgb(0x3E, 0x30, 0x1C),
            hover: Color32::from_rgb(0x34, 0x30, 0x2C),
            row_alt: Color32::from_rgb(0x29, 0x26, 0x23),
            ok: Color32::from_rgb(0x55, 0xC6, 0x7F),
            ok_soft: Color32::from_rgb(0x1E, 0x33, 0x25),
            danger: Color32::from_rgb(0xF9, 0x73, 0x69),
            danger_soft: Color32::from_rgb(0x3D, 0x21, 0x1E),
            off: Color32::from_rgb(0x7A, 0x73, 0x6B),
            chart: Color32::from_rgb(0xBF, 0x7E, 0x22),
        }
    }

    pub fn get(ctx: &egui::Context) -> Self {
        if ctx.theme() == Theme::Dark { Self::dark() } else { Self::light() }
    }
}

/// Installs Porchly's visuals and larger, easy-to-read text sizes.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Light, visuals(&Tokens::light()));
    ctx.set_visuals_of(Theme::Dark, visuals(&Tokens::dark()));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        // Always-visible scrollbars: people shouldn't have to discover hidden content by hovering.
        style.spacing.scroll = egui::style::ScrollStyle::solid();
    });
}

fn visuals(t: &Tokens) -> Visuals {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    let radius = CornerRadius::same(6);
    v.override_text_color = Some(t.text);
    v.panel_fill = t.bg;
    v.window_fill = t.surface;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = t.surface;
    v.faint_bg_color = t.row_alt;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.inactive.weak_bg_fill = t.chrome;
    v.widgets.inactive.bg_fill = t.chrome;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.widgets.active.bg_fill = t.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    v
}
