//! The studio's look: Inter, roomier spacing, softer rounding and one
//! accent colour, in a light and a dark palette close to GNOME's Adwaita.
//!
//! egui draws every widget itself, so the studio cannot take on the
//! desktop's own widgets; this sets the colours, fonts and spacing egui
//! draws with. The light or dark palette follows the system's preference.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin,
    Shadow, Stroke, Style, TextStyle, Theme, Visuals,
};

/// Inter, the proportional font (SIL Open Font License; the licence is
/// beside the font in `assets/fonts`).
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

/// The font family headings are drawn in.
pub const SEMIBOLD: &str = "Inter SemiBold";

/// Corner rounding of widgets and panels.
pub const ROUNDING: u8 = 6;

/// The accent: selection, focus, links and the chosen tab's text.
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x1C, 0x71, 0xD8);
/// The accent on dark backgrounds.
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0x78, 0xAE, 0xED);

/// Applies the studio's fonts, spacing and both palettes to `context`, and
/// follows the system's light or dark preference.
pub fn apply(context: &Context) {
    context.set_fonts(fonts());
    for theme in [Theme::Light, Theme::Dark] {
        context.style_mut_of(theme, |style| restyle(style, theme));
    }
    context.set_theme(egui::ThemePreference::System);
}

/// egui's own fonts with Inter put first, so egui's emoji and icon fonts
/// still draw what Inter lacks.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        Arc::new(FontData::from_static(INTER_REGULAR)),
    );
    fonts.font_data.insert(
        SEMIBOLD.to_owned(),
        Arc::new(FontData::from_static(INTER_SEMIBOLD)),
    );
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut proportional = vec!["Inter".to_owned()];
    proportional.extend(fallbacks.iter().cloned());
    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    let mut semibold = vec![SEMIBOLD.to_owned()];
    semibold.extend(fallbacks);
    fonts
        .families
        .insert(FontFamily::Name(Arc::from(SEMIBOLD)), semibold);
    fonts
}

/// Restyles one theme's `style`: text sizes, spacing, and the palette.
fn restyle(style: &mut Style, theme: Theme) {
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (
            TextStyle::Heading,
            FontId::new(19.0, FontFamily::Name(Arc::from(SEMIBOLD))),
        ),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 6.0);
    spacing.button_padding = egui::vec2(10.0, 4.0);
    spacing.interact_size.y = 26.0;
    spacing.indent = 20.0;
    spacing.window_margin = Margin::same(12);
    spacing.menu_margin = Margin::same(8);
    spacing.combo_width = 120.0;
    style.visuals = palette(theme);
}

/// The palette for `theme`.
pub fn palette(theme: Theme) -> Visuals {
    let dark = theme == Theme::Dark;
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let grey = |v: u8| Color32::from_rgb(v, v, v);
    // Window, field and faint greys, a panel border and a stronger control
    // edge, then the controls' fills when resting, hovered and pressed, and
    // the text.
    let (window, view, faint, border, edge, rest, hover, press, text, strong) = if dark {
        (
            grey(0x24),
            grey(0x1E),
            grey(0x2C),
            grey(0x3D),
            grey(0x5A),
            grey(0x38),
            grey(0x45),
            grey(0x52),
            grey(0xDE),
            grey(0xFF),
        )
    } else {
        (
            grey(0xFA),
            grey(0xFF),
            grey(0xF0),
            grey(0xD8),
            grey(0xB8),
            grey(0xE6),
            grey(0xDA),
            grey(0xCC),
            grey(0x2E),
            grey(0x00),
        )
    };
    let accent = if dark { ACCENT_DARK } else { ACCENT_LIGHT };
    let rounding = CornerRadius::same(ROUNDING);

    visuals.panel_fill = window;
    visuals.window_fill = view;
    visuals.extreme_bg_color = view;
    visuals.faint_bg_color = faint;
    visuals.code_bg_color = faint;
    visuals.window_stroke = Stroke::new(1.0_f32, border);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = rounding;
    let shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 40 }),
    };
    visuals.window_shadow = shadow;
    visuals.popup_shadow = Shadow { blur: 10, ..shadow };
    visuals.hyperlink_color = accent;
    visuals.selection.bg_fill = accent.linear_multiply(if dark { 0.45 } else { 0.25 });
    visuals.selection.stroke = Stroke::new(1.5_f32, accent);
    visuals.text_cursor.stroke = Stroke::new(2.0_f32, accent);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = window;
    widgets.noninteractive.weak_bg_fill = window;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);
    // Controls keep a thin edge: text fields draw theirs from the same
    // states, and a white field on a near-white panel needs one.
    for (state, fill, outline) in [
        (&mut widgets.inactive, rest, border),
        (&mut widgets.hovered, hover, edge),
        (&mut widgets.active, press, accent),
        (&mut widgets.open, hover, edge),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0_f32, outline);
        state.fg_stroke = Stroke::new(1.0_f32, text);
        state.expansion = 0.0;
    }
    widgets.hovered.fg_stroke = Stroke::new(1.5_f32, strong);
    widgets.active.fg_stroke = Stroke::new(1.5_f32, strong);
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = rounding;
    }
    visuals
}

#[cfg(test)]
mod tests {
    use super::{apply, fonts, palette, ACCENT_DARK, ACCENT_LIGHT, ROUNDING, SEMIBOLD};
    use eframe::egui::{Context, CornerRadius, FontFamily, TextStyle, Theme};

    #[test]
    fn inter_comes_first_and_egui_fonts_still_back_it() {
        let fonts = fonts();
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], "Inter");
        assert!(proportional.len() > 1, "egui's own fonts follow");
        let semibold = &fonts.families[&FontFamily::Name(SEMIBOLD.into())];
        assert_eq!(semibold[0], SEMIBOLD);
        for family in proportional.iter().chain(semibold) {
            assert!(fonts.font_data.contains_key(family), "{family}");
        }
    }

    #[test]
    fn both_palettes_share_the_shape_and_differ_in_light() {
        let light = palette(Theme::Light);
        let dark = palette(Theme::Dark);
        assert!(!light.dark_mode && dark.dark_mode);
        assert_eq!(light.hyperlink_color, ACCENT_LIGHT);
        assert_eq!(dark.hyperlink_color, ACCENT_DARK);
        for visuals in [&light, &dark] {
            assert_eq!(
                visuals.widgets.inactive.corner_radius,
                CornerRadius::same(ROUNDING)
            );
            // Text stands out from what it is drawn on.
            let text = visuals.widgets.inactive.fg_stroke.color;
            assert_ne!(text, visuals.widgets.inactive.bg_fill);
            assert_ne!(text, visuals.panel_fill);
        }
        assert!(
            light.panel_fill.r() > dark.panel_fill.r(),
            "light is lighter"
        );
    }

    #[test]
    fn applying_sets_both_themes_and_follows_the_system() {
        let context = Context::default();
        apply(&context);
        for theme in [Theme::Light, Theme::Dark] {
            context.style_mut_of(theme, |style| {
                assert_eq!(
                    style.visuals.widgets.inactive.corner_radius,
                    CornerRadius::same(ROUNDING)
                );
                let heading = &style.text_styles[&TextStyle::Heading];
                assert_eq!(heading.family, FontFamily::Name(SEMIBOLD.into()));
            });
        }
        assert_eq!(
            context.options(|options| options.theme_preference),
            eframe::egui::ThemePreference::System
        );
    }
}
