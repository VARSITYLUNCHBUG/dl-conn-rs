//! Colours and spacing for light and dark mode. Every colour the UI uses comes from a `Palette`, so
//! the two modes can't drift apart.

use eframe::egui::{
    self, Color32, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme,
    ThemePreference, ViewportCommand, Visuals, viewport::SystemTheme,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub page: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub hover: Color32,
    pub ok: Color32,
    pub check: Color32,
    pub note: Color32,
}

const DARK: Palette = Palette {
    page: Color32::from_rgb(24, 26, 31),
    surface: Color32::from_rgb(32, 35, 41),
    raised: Color32::from_rgb(42, 46, 54),
    border: Color32::from_rgb(62, 67, 77),
    text: Color32::from_rgb(232, 234, 238),
    muted: Color32::from_rgb(160, 166, 177),
    accent: Color32::from_rgb(77, 141, 245),
    on_accent: Color32::WHITE,
    hover: Color32::from_rgb(50, 55, 65),
    ok: Color32::from_rgb(90, 200, 130),
    check: Color32::from_rgb(240, 180, 80),
    note: Color32::from_rgb(120, 175, 245),
};

const LIGHT: Palette = Palette {
    page: Color32::from_rgb(246, 247, 249),
    surface: Color32::from_rgb(255, 255, 255),
    raised: Color32::from_rgb(236, 238, 242),
    border: Color32::from_rgb(205, 209, 216),
    text: Color32::from_rgb(28, 31, 36),
    muted: Color32::from_rgb(88, 94, 105),
    accent: Color32::from_rgb(28, 100, 216),
    on_accent: Color32::WHITE,
    hover: Color32::from_rgb(228, 232, 238),
    ok: Color32::from_rgb(20, 120, 65),
    check: Color32::from_rgb(160, 90, 0),
    note: Color32::from_rgb(25, 90, 190),
};

pub fn palette(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

fn visuals(theme: Theme) -> Visuals {
    let colours = palette(theme == Theme::Dark);
    let mut visuals = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    let round = CornerRadius::same(6);
    visuals.override_text_color = Some(colours.text);
    visuals.panel_fill = colours.page;
    visuals.window_fill = colours.surface;
    visuals.extreme_bg_color = colours.surface;
    visuals.faint_bg_color = colours.raised;
    visuals.window_stroke = Stroke::new(1.0, colours.border);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.selection.bg_fill = colours.accent;
    visuals.selection.stroke = Stroke::new(1.0, colours.on_accent);
    visuals.hyperlink_color = colours.accent;
    // A blinking caret would keep the window drawing while it sits next to a game.
    visuals.text_cursor.blink = false;
    let states = [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ];
    for state in states {
        state.corner_radius = round;
    }
    for state in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        state.fg_stroke = Stroke::new(1.0, colours.text);
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colours.border);
    visuals.widgets.inactive.bg_fill = colours.raised;
    visuals.widgets.inactive.weak_bg_fill = colours.raised;
    visuals.widgets.hovered.bg_fill = colours.hover;
    visuals.widgets.hovered.weak_bg_fill = colours.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, colours.border);
    visuals.widgets.active.bg_fill = colours.hover;
    visuals.widgets.active.weak_bg_fill = colours.hover;
    visuals.widgets.open.bg_fill = colours.raised;
    visuals.widgets.open.weak_bg_fill = colours.raised;
    visuals
}

/// Sets colours for both modes, and text sizes and spacing for everything.
pub fn install(ctx: &Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(Theme::Dark));
    ctx.set_visuals_of(Theme::Light, visuals(Theme::Light));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.5, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(18.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// What the `theme` setting means to egui and to the title bar. Anything unknown follows Windows.
pub fn preference(value: &str) -> (ThemePreference, SystemTheme) {
    match value {
        "light" => (ThemePreference::Light, SystemTheme::Light),
        "dark" => (ThemePreference::Dark, SystemTheme::Dark),
        _ => (ThemePreference::System, SystemTheme::SystemDefault),
    }
}

/// Applies the `theme` setting, including the window's title bar.
pub fn apply(ctx: &Context, value: &str) {
    let (preference, title_bar) = preference(value);
    ctx.set_theme(preference);
    ctx.send_viewport_cmd(ViewportCommand::SetTheme(title_bar));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance, for checking text stays readable.
    fn luminance(colour: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_is_readable_in_both_modes() {
        for colours in [palette(true), palette(false)] {
            for text in [
                colours.text,
                colours.muted,
                colours.ok,
                colours.check,
                colours.note,
            ] {
                for background in [colours.page, colours.surface] {
                    let ratio = contrast(text, background);
                    assert!(ratio >= 4.5, "{text:?} on {background:?} is only {ratio}");
                }
            }
            assert!(contrast(colours.on_accent, colours.accent) >= 3.0);
        }
    }

    #[test]
    fn the_setting_maps_to_a_theme() {
        assert_eq!(preference("dark").0, ThemePreference::Dark);
        assert_eq!(preference("light").0, ThemePreference::Light);
        assert_eq!(preference("system").0, ThemePreference::System);
        assert_eq!(preference("anything").0, ThemePreference::System);
    }
}
