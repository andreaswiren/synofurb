//! The one place where the look of the app is decided.
//!
//! Source of truth: `design/v2/tokens-v2.json` and `design/v2/synofurb-theme.json` (the UI designer's design system).
//! * [`super::tokens`] is generated from the JSON (`assets/theme/gen_tokens.py`): the [`Palette`] our own few custom
//!   pieces read (status tones, log colours, the diagram), and tests that keep it identical to the JSON.
//! * The GPUI Kit theme itself is NOT hand-mapped any more: the designer's ThemeSet (`assets/theme/synofurb-theme.json`,
//!   one light and one dark `ThemeConfig` with Inter, JetBrains Mono, radius and 120+ colour keys) is loaded straight into
//!   gpui-component, so every stock widget (buttons, inputs, tabs, sidebar, tables, dialogs, scrollbars...) is themed
//!   by the same file.
//!
//! Screens never contain a hex value or a magic size; sizes come from [`layout`], [`space`], [`radius`], text from the
//! type styles below.

use gpui_kit::base::{ScrollbarStyles, Theme as BaseTheme};
use gpui_kit::component::{ActiveTheme, Theme, ThemeConfig, ThemeMode, ThemeSet};
use gpui_kit::{App, Global, Hsla, Rems, WindowAppearance, px, rems};
use serde::{Deserialize, Serialize};
use std::rc::Rc;

pub use super::tokens::{DARK, Hexes, LIGHT, Palette};

/// The designer's GPUI theme set (light + dark). Copied from `design/v2/synofurb-theme.json`.
pub const THEME_JSON: &str = include_str!("../../assets/theme/synofurb-theme.json");

// ------------------------------------------------------------------------------------------------ tones

impl Palette {
    pub fn for_mode(dark: bool) -> Palette {
        Palette::from_hex(if dark { &DARK } else { &LIGHT }, dark)
    }

    /// Colour family of a [`Tone`] (text, tint, border).
    pub fn tone(&self, tone: Tone) -> ToneColors {
        match tone {
            Tone::Ok => ToneColors { solid: self.ok_solid, border: self.ok_border, fg: self.ok_fg, bg: self.ok_bg },
            Tone::Warn => ToneColors { solid: self.warn_solid, border: self.warn_border, fg: self.warn_fg, bg: self.warn_bg },
            Tone::Err => ToneColors { solid: self.err_solid, border: self.err_border, fg: self.err_fg, bg: self.err_bg },
            Tone::Info => ToneColors { solid: self.info_solid, border: self.info_border, fg: self.info_fg, bg: self.info_bg },
            Tone::Accent => ToneColors { solid: self.accent, border: self.accent_border, fg: self.accent_fg, bg: self.accent_bg },
            Tone::Neutral => ToneColors { solid: self.border_strong, border: self.border, fg: self.fg_2, bg: self.surface_2 },
        }
    }
}

/// Semantic colour family. Status is never colour alone: widgets that take a `Tone` always pair it with an icon and a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Err,
    Info,
    Accent,
    Neutral,
}

#[derive(Clone, Copy, Debug)]
pub struct ToneColors {
    pub solid: Hsla,
    /// Border at about half strength (never the full solid colour).
    pub border: Hsla,
    pub fg: Hsla,
    pub bg: Hsla,
}

/// The palette currently in use (a GPUI global; replaced by [`apply`]).
pub struct ActivePalette(pub Palette);
impl Global for ActivePalette {}

/// `cx.pal()` for anything that derefs to `App`.
pub trait Themed {
    fn pal(&self) -> Palette;
}
impl Themed for App {
    fn pal(&self) -> Palette {
        self.try_global::<ActivePalette>().map(|p| p.0).unwrap_or_else(|| Palette::for_mode(self.theme().is_dark()))
    }
}

// ------------------------------------------------------------------------------------------------ sizes

/// Spacing scale (px).
#[allow(dead_code)] // the full scale is kept so a design revision can use any step
pub mod space {
    pub const S2: f32 = 2.0;
    pub const S4: f32 = 4.0;
    pub const S6: f32 = 6.0;
    pub const S8: f32 = 8.0;
    pub const S12: f32 = 12.0;
    pub const S16: f32 = 16.0;
    pub const S20: f32 = 20.0;
    pub const S24: f32 = 24.0;
    pub const S32: f32 = 32.0;
}

/// Corner radii (px), `radius.*` in tokens-v2.json.
#[allow(dead_code)]
pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
}

/// Layout constants (px), `size.*` in tokens-v2.json.
#[allow(dead_code)]
pub mod layout {
    pub const WINDOW_MIN_W: f32 = 960.0;
    pub const WINDOW_MIN_H: f32 = 640.0;
    pub const WINDOW_W: f32 = 1180.0;
    pub const WINDOW_H: f32 = 760.0;
    pub const SIDEBAR_W: f32 = 240.0;
    pub const PAGE_PAD: f32 = 24.0;
    pub const DIALOG_W: f32 = 448.0;
    pub const DIALOG_W_WIDE: f32 = 640.0;
    pub const SHEET_W: f32 = 350.0;
    pub const SHEET_W_WIDE: f32 = 480.0;
    pub const FORM_LABEL_W: f32 = 160.0;
    pub const CONTROL_MD: f32 = 32.0;
}

/// Type styles (`type.*`): size in px at 100 % text scale. They are expressed in rems (1 rem = 16 px) so the
/// "text size" setting, which changes the window's rem size, scales every style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    pub size_px: f32,
    pub line_px: f32,
    pub weight: f32,
}

impl TypeStyle {
    pub fn size(&self) -> Rems {
        rems(self.size_px / 16.0)
    }
    pub fn line(&self) -> Rems {
        rems(self.line_px / 16.0)
    }
}

/// Page title (at most 4 words).
pub const TITLE: TypeStyle = TypeStyle { size_px: 24.0, line_px: 32.0, weight: 700.0 };
/// Card, dialog and drawer titles.
pub const SECTION: TypeStyle = TypeStyle { size_px: 16.0, line_px: 24.0, weight: 600.0 };
pub const BODY: TypeStyle = TypeStyle { size_px: 14.0, line_px: 20.0, weight: 400.0 };
pub const CAPTION: TypeStyle = TypeStyle { size_px: 12.0, line_px: 16.0, weight: 400.0 };
const _: () = assert!(CAPTION.size_px >= 12.0, "never below 12 px");
/// Hashes, IPs, serials, commands.
pub const MONO: TypeStyle = TypeStyle { size_px: 13.0, line_px: 20.0, weight: 400.0 };
pub const LOG: TypeStyle = TypeStyle { size_px: 12.0, line_px: 18.0, weight: 400.0 };

// ------------------------------------------------------------------------------------------------ user choices

/// Light / dark / follow the operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    /// Whether the dark palette is used, given what the OS reports.
    pub fn is_dark(self, os_dark: bool) -> bool {
        match self {
            ThemeChoice::System => os_dark,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        }
    }
}

/// Text size setting (100 / 115 / 130 %).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextScale {
    #[default]
    Default,
    Large,
    Largest,
}

impl TextScale {
    pub const ALL: [TextScale; 3] = [TextScale::Default, TextScale::Large, TextScale::Largest];

    pub fn factor(self) -> f32 {
        match self {
            TextScale::Default => 1.0,
            TextScale::Large => 1.15,
            TextScale::Largest => 1.30,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TextScale::Default => "100%",
            TextScale::Large => "115%",
            TextScale::Largest => "130%",
        }
    }
    /// Window rem size in px.
    pub fn rem_px(self) -> f32 {
        16.0 * self.factor()
    }
}

pub fn os_is_dark(cx: &App) -> bool {
    matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

// ------------------------------------------------------------------------------------------------ applying

/// The parsed light and dark `ThemeConfig` of [`THEME_JSON`].
struct Configs {
    light: Rc<ThemeConfig>,
    dark: Rc<ThemeConfig>,
}
impl Global for Configs {}

/// Parse [`THEME_JSON`] into (light, dark). Panics on a malformed bundled file: a unit test guards it.
pub fn theme_configs() -> (Rc<ThemeConfig>, Rc<ThemeConfig>) {
    let set: ThemeSet = serde_json::from_str(THEME_JSON).expect("assets/theme/synofurb-theme.json is not a valid ThemeSet");
    let mut light = None;
    let mut dark = None;
    for t in set.themes {
        if t.mode.is_dark() {
            dark = Some(Rc::new(t));
        } else {
            light = Some(Rc::new(t));
        }
    }
    (light.expect("theme set has no light theme"), dark.expect("theme set has no dark theme"))
}

/// Install the synofurb theme in the GPUI Kit, pick light or dark, scale text and theme the scrollbars.
/// Call again to switch mode or text size; it repaints every window.
pub fn apply(dark: bool, scale: TextScale, cx: &mut App) {
    if !cx.has_global::<Configs>() {
        let (light, dark) = theme_configs();
        cx.set_global(Configs { light, dark });
    }
    let (light, dark_cfg) = {
        let c = cx.global::<Configs>();
        (c.light.clone(), c.dark.clone())
    };
    {
        let t = Theme::global_mut(cx);
        t.light_theme = light;
        t.dark_theme = dark_cfg;
    }
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    cx.set_global(ActivePalette(Palette::for_mode(dark)));
    Theme::update(cx, |t| {
        // The window's rem size follows the theme's font size (see gpui-component's Root), so this is the text-size setting.
        t.font_size = px(scale.rem_px());
        t.scrollbar_mode = gpui_kit::component::scroll::ScrollbarMode::Always;
    });
    style_scrollbars(cx);
}

/// Thin themed scrollbars: 6 px thumb (8 px on hover and drag), 4 px inset, rounded, token colours, transparent track
/// (`scrollbar.*` in tokens-v2.json). Runs after every theme change because the Kit rebuilds its base theme then.
fn style_scrollbars(cx: &mut App) {
    let (thumb, thumb_hover, border) = {
        let t = cx.theme();
        (t.tokens.scrollbar_thumb, t.tokens.scrollbar_thumb_hover, t.border)
    };
    let styles = ScrollbarStyles::default()
        .track(|s| s.bg(gpui_kit::transparent_black()))
        .track_hover(|s| s.bg(gpui_kit::transparent_black()))
        .track_active(|s| s.bg(gpui_kit::transparent_black()).border_color(border))
        .thumb(|s| {
            s.bg(thumb).width(px(SCROLLBAR_THUMB)).inset(px(SCROLLBAR_INSET)).radius(px(SCROLLBAR_RADIUS)).min_length(px(SCROLLBAR_MIN))
        })
        .thumb_hover(|s| s.bg(thumb_hover).width(px(SCROLLBAR_THUMB_HOT)).inset(px(SCROLLBAR_INSET - 1.0)).radius(px(SCROLLBAR_RADIUS)))
        .thumb_active(|s| s.bg(thumb_hover).width(px(SCROLLBAR_THUMB_HOT)).inset(px(SCROLLBAR_INSET - 1.0)).radius(px(SCROLLBAR_RADIUS)));
    let base = cx.global_mut::<BaseTheme>();
    base.scrollbar = base.scrollbar.clone().with_styles(styles);
}

pub const SCROLLBAR_THUMB: f32 = 6.0;
pub const SCROLLBAR_THUMB_HOT: f32 = 8.0;
pub const SCROLLBAR_INSET: f32 = 4.0;
pub const SCROLLBAR_RADIUS: f32 = 8.0;
pub const SCROLLBAR_MIN: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.x relative luminance contrast of two opaque sRGB colours.
    pub(crate) fn contrast(a: u32, b: u32) -> f64 {
        fn lum(h: u32) -> f64 {
            let ch = |v: u32| {
                let c = (v & 0xff) as f64 / 255.0;
                if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(h >> 16) + 0.7152 * ch(h >> 8) + 0.0722 * ch(h)
        }
        let (la, lb) = (lum(a), lum(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn tokens_json() -> Option<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/v2/tokens-v2.json");
        std::fs::read_to_string(path).ok()
    }

    #[test]
    fn contrast_helper_is_correct() {
        assert!((contrast(0x000000, 0xFFFFFF) - 21.0).abs() < 0.01);
        assert!((contrast(0x777777, 0x777777) - 1.0).abs() < 0.001);
    }

    #[test]
    fn declared_text_pairs_pass_wcag_aa() {
        for (name, h) in [("light", &LIGHT), ("dark", &DARK)] {
            let pairs: Vec<(&str, u32, u32, f64)> = vec![
                ("fg/canvas", h.fg, h.canvas, 4.5),
                ("fg/surface", h.fg, h.surface, 4.5),
                ("fg_2/surface", h.fg_2, h.surface, 4.5),
                ("fg_2/surface_2", h.fg_2, h.surface_2, 4.5),
                ("fg_2/sidebar", h.fg_2, h.sidebar, 4.5),
                ("placeholder/surface", h.placeholder, h.surface, 4.5),
                ("on_danger/danger_pressed", h.on_danger, h.danger_pressed, 4.5),
                ("on_accent/accent_pressed", h.on_accent, h.accent_pressed, 4.5),
                ("on_accent/accent", h.on_accent, h.accent, 4.5),
                ("accent_fg/accent_bg", h.accent_fg, h.accent_bg, 4.5),
                ("on_danger/danger", h.on_danger, h.danger, 4.5),
                ("ok_fg/ok_bg", h.ok_fg, h.ok_bg, 4.5),
                ("warn_fg/warn_bg", h.warn_fg, h.warn_bg, 4.5),
                ("err_fg/err_bg", h.err_fg, h.err_bg, 4.5),
                ("info_fg/info_bg", h.info_fg, h.info_bg, 4.5),
                ("on_ok/ok_fg", h.on_ok, h.ok_fg, 4.5),
                ("on_warn/warn_fg", h.on_warn, h.warn_fg, 4.5),
                ("on_err/err_fg", h.on_err, h.err_fg, 4.5),
                ("on_info/info_fg", h.on_info, h.info_fg, 4.5),
                ("log_fg/log_bg", h.log_fg, h.log_bg, 4.5),
                ("log_dim/log_bg", h.log_dim, h.log_bg, 4.5),
                ("log_inf/log_bg", h.log_inf, h.log_bg, 4.5),
                ("log_wrn/log_bg", h.log_wrn, h.log_bg, 4.5),
                ("log_err/log_bg", h.log_err, h.log_bg, 4.5),
                ("border_strong/surface (UI component)", h.border_strong, h.surface, 3.0),
                ("ring/surface (focus ring)", h.ring, h.surface, 3.0),
                ("scrollbar_thumb/surface (UI component)", h.scrollbar_thumb, h.surface, 3.0),
            ];
            for (label, fg, bg, min) in pairs {
                let r = contrast(fg, bg);
                assert!(r >= min, "{name} {label}: {r:.2} < {min}");
            }
        }
    }

    #[test]
    fn generated_tokens_match_tokens_v2_json() {
        let Some(json) = tokens_json() else {
            eprintln!("tokens-v2.json not found; skipping");
            return;
        };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for (theme, hexes) in [("light", &LIGHT), ("dark", &DARK)] {
            for (name, value) in hexes.fields() {
                let key = format!("color.{theme}.{name}");
                let want = v[&key].as_str().unwrap_or_else(|| panic!("token {key} missing from tokens-v2.json"));
                let want = u32::from_str_radix(want.trim_start_matches('#'), 16).unwrap();
                assert_eq!(value, want, "{key}: tokens.rs has {value:06X}, tokens-v2.json has {want:06X}; run assets/theme/gen_tokens.py");
            }
        }
    }

    #[test]
    fn bundled_theme_set_is_the_designers_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/v2/synofurb-theme.json");
        if let Ok(design) = std::fs::read_to_string(path) {
            assert_eq!(design.trim(), THEME_JSON.trim(), "assets/theme/synofurb-theme.json is stale; run assets/theme/gen_tokens.py");
        }
        let (light, dark) = theme_configs();
        assert!(!light.mode.is_dark() && dark.mode.is_dark());
        for c in [&light, &dark] {
            assert_eq!(c.font_family.as_deref(), Some(super::super::assets::FONT_UI));
            assert_eq!(c.mono_font_family.as_deref(), Some(super::super::assets::FONT_MONO));
        }
    }

    #[test]
    fn theme_json_colours_follow_the_token_map() {
        let Some(json) = tokens_json() else { return };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let set: serde_json::Value = serde_json::from_str(THEME_JSON).unwrap();
        for (theme, hexes) in [("synofurb Light", &LIGHT), ("synofurb Dark", &DARK)] {
            let colors = set["themes"].as_array().unwrap().iter().find(|t| t["name"] == theme).unwrap()["colors"].clone();
            let mut checked = 0;
            for (k, val) in v.as_object().unwrap() {
                let Some(key) = k.strip_prefix("map.") else { continue };
                let Some(token) = val.as_str() else { continue };
                let Some((_, hex)) = hexes.fields().into_iter().find(|(n, _)| *n == token) else { continue };
                let got = colors[key].as_str().unwrap_or_else(|| panic!("{theme}: colour key {key} missing")).to_uppercase();
                assert!(got.starts_with(&format!("#{hex:06X}")), "{theme} {key}: theme has {got}, token {token} is {hex:06X}");
                checked += 1;
            }
            assert!(checked > 60, "{theme}: only {checked} mapped keys checked");
        }
    }

    #[test]
    fn theme_choice_resolution() {
        assert!(ThemeChoice::System.is_dark(true));
        assert!(!ThemeChoice::System.is_dark(false));
        assert!(!ThemeChoice::Light.is_dark(true));
        assert!(ThemeChoice::Dark.is_dark(false));
    }

    #[test]
    fn text_scale_factors() {
        assert_eq!(TextScale::Default.rem_px(), 16.0);
        assert!((TextScale::Largest.rem_px() - 20.8).abs() < 0.01);
    }

    #[test]
    fn type_styles_convert_to_rems() {
        assert_eq!(BODY.size(), rems(14.0 / 16.0));
    }

    #[test]
    fn tone_mapping_is_distinct() {
        let p = Palette::for_mode(false);
        assert_ne!(p.tone(Tone::Ok).bg, p.tone(Tone::Err).bg);
        assert!(!p.dark && Palette::for_mode(true).dark);
    }

    #[test]
    fn type_styles_match_tokens_v2() {
        let Some(json) = tokens_json() else { return };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for (name, ts) in [("title", TITLE), ("section", SECTION), ("body", BODY), ("caption", CAPTION), ("mono", MONO), ("log", LOG)] {
            let f = |k: &str| v[format!("type.{name}.{k}")].as_f64().unwrap() as f32;
            assert_eq!((ts.size_px, ts.line_px, ts.weight), (f("size_px"), f("line_px"), f("weight")), "type.{name}");
        }
    }
}
