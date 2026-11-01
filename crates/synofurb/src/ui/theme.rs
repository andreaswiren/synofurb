//! The one place where the look of the app is decided.
//!
//! Every colour, size, radius and type style used by the UI comes from here; screens never contain a hex
//! value or a magic pixel size. The values are the "Direction 1 / ForgeVault family" tokens of
//! `design/mockups/ui/tokens.json` (a unit test keeps this file and the JSON in sync). To change the look,
//! edit the tables below (or swap [`LIGHT`]/[`DARK`] for another direction) and re-run the contrast test.
//!
//! Two layers:
//! * [`Palette`] - our semantic tokens (accent, ok/warn/err/info families, log colours...). Screens read
//!   them through [`Themed::pal`] (`cx.pal()`).
//! * [`apply`] - maps the palette onto GPUI Kit's own `Theme` (`gpui_kit::component::Theme`), so stock
//!   components (buttons, inputs, checkboxes, sliders, selects, dialogs) pick up the same look.

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Rems, WindowAppearance, px, rems, rgb};
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------------------------------------ colours

macro_rules! tokens {
    ($($f:ident),* $(,)?) => {
        /// Opaque sRGB hex values, one per colour token (names match `tokens.json` without the theme prefix).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Hexes { $(pub $f: u32,)* }

        impl Hexes {
            /// `(token name, value)` pairs, used by tests and tooling.
            #[allow(dead_code)] // used by the token tests
            pub fn fields(&self) -> Vec<(&'static str, u32)> {
                vec![$((stringify!($f), self.$f),)*]
            }
        }

        /// The tokens converted to GPUI colours.
        #[derive(Clone, Copy, Debug)]
        #[allow(dead_code)] // every design token is kept, even those no widget uses yet
        pub struct Palette { $(pub $f: Hsla,)* pub dark: bool }

        impl Palette {
            pub fn from_hex(h: &Hexes, dark: bool) -> Palette {
                Palette { $($f: rgb(h.$f).into(),)* dark }
            }
        }
    };
}

tokens!(
    canvas,
    surface,
    surface_2,
    surface_3,
    sidebar,
    border,
    border_strong,
    fg,
    fg_2,
    placeholder,
    accent,
    accent_hover,
    accent_pressed,
    on_accent,
    accent_bg,
    accent_fg,
    ring,
    ring_inverse,
    danger,
    danger_hover,
    danger_pressed,
    danger_edge,
    on_danger,
    ok_solid,
    ok_fg,
    ok_bg,
    warn_solid,
    warn_fg,
    warn_bg,
    err_solid,
    err_fg,
    err_bg,
    info_solid,
    info_fg,
    info_bg,
    log_bg,
    log_fg,
    log_dim,
    log_dbg,
    log_inf,
    log_wrn,
    log_err,
    ok_border,
    warn_border,
    err_border,
    info_border,
    accent_border,
    log_wrn_bg,
    log_err_bg,
    scrim,
);

pub const LIGHT: Hexes = Hexes {
    canvas: 0xFAFAFA,
    surface: 0xFFFFFF,
    surface_2: 0xF3F3F3,
    surface_3: 0xE9E9E9,
    sidebar: 0xF7F7F7,
    border: 0xE5E5E5,
    border_strong: 0x808080,
    fg: 0x0A0A0A,
    fg_2: 0x525252,
    placeholder: 0x696969,
    accent: 0x7139CB,
    accent_hover: 0x622CB5,
    accent_pressed: 0x541EA0,
    on_accent: 0xFFFFFF,
    accent_bg: 0xF1EDFE,
    accent_fg: 0x5C29AA,
    ring: 0x7139CB,
    ring_inverse: 0xA37BFE,
    danger: 0xC60014,
    danger_hover: 0xAC0311,
    danger_pressed: 0x94020D,
    danger_edge: 0xC60014,
    on_danger: 0xFFFFFF,
    ok_solid: 0x047E39,
    ok_fg: 0x005725,
    ok_bg: 0xDEF8E2,
    warn_solid: 0xA76C01,
    warn_fg: 0x6F4606,
    warn_bg: 0xFFF3D3,
    err_solid: 0xC60014,
    err_fg: 0xA70311,
    err_bg: 0xFEEFEF,
    info_solid: 0x1570D1,
    info_fg: 0x004B96,
    info_bg: 0xE7F1FF,
    log_bg: 0x121114,
    log_fg: 0xDEDEDE,
    log_dim: 0x9E9E9E,
    log_dbg: 0x9DA5B1,
    log_inf: 0x89C3FE,
    log_wrn: 0xF7C243,
    log_err: 0xFE8A88,
    ok_border: 0x66B585,
    warn_border: 0xCFA95F,
    err_border: 0xDF6C77,
    info_border: 0x73AAE6,
    accent_border: 0xB193E4,
    log_wrn_bg: 0x322A1B,
    log_err_bg: 0x3C2729,
    scrim: 0x000000,
};

pub const DARK: Hexes = Hexes {
    canvas: 0x0A0A0A,
    surface: 0x171717,
    surface_2: 0x202020,
    surface_3: 0x2B2B2B,
    sidebar: 0x101010,
    border: 0x2A2A2A,
    border_strong: 0x868686,
    fg: 0xFAFAFA,
    fg_2: 0xB1B1B1,
    placeholder: 0x9E9E9E,
    accent: 0xA37BFE,
    accent_hover: 0xB091FE,
    accent_pressed: 0x946AEE,
    on_accent: 0x0E0C15,
    accent_bg: 0x31254D,
    accent_fg: 0xC8B7FF,
    ring: 0xA37BFE,
    ring_inverse: 0xA37BFE,
    danger: 0xB7191C,
    danger_hover: 0xA50E15,
    danger_pressed: 0x8D0B11,
    danger_edge: 0xFF9592,
    on_danger: 0xFFFFFF,
    ok_solid: 0x37BB62,
    ok_fg: 0x6ED889,
    ok_bg: 0x0F3118,
    warn_solid: 0xF0B135,
    warn_fg: 0xF1C45E,
    warn_bg: 0x3D2A02,
    err_solid: 0xFC5855,
    err_fg: 0xFFA09C,
    err_bg: 0x4F1A18,
    info_solid: 0x539AF2,
    info_fg: 0x91C1FF,
    info_bg: 0x132946,
    log_bg: 0x060607,
    log_fg: 0xDEDEDE,
    log_dim: 0x989898,
    log_dbg: 0x979FAB,
    log_inf: 0x89C3FE,
    log_wrn: 0xF7C243,
    log_err: 0xFE8A88,
    ok_border: 0x257D41,
    warn_border: 0x9F741E,
    err_border: 0xAE3C3A,
    info_border: 0x3667A5,
    accent_border: 0x6A50A6,
    log_wrn_bg: 0x28200F,
    log_err_bg: 0x331E1E,
    scrim: 0x000000,
};

impl Palette {
    pub fn light() -> Palette {
        Palette::from_hex(&LIGHT, false)
    }
    pub fn dark() -> Palette {
        Palette::from_hex(&DARK, true)
    }
    pub fn for_mode(dark: bool) -> Palette {
        if dark { Palette::dark() } else { Palette::light() }
    }

    /// Colour of the 3 px edge / icon / text for a [`Tone`].
    pub fn tone(&self, tone: Tone) -> ToneColors {
        match tone {
            Tone::Ok => ToneColors { solid: self.ok_solid, border: self.ok_border, fg: self.ok_fg, bg: self.ok_bg },
            Tone::Warn => ToneColors { solid: self.warn_solid, border: self.warn_border, fg: self.warn_fg, bg: self.warn_bg },
            Tone::Err => ToneColors { solid: self.err_solid, border: self.err_border, fg: self.err_fg, bg: self.err_bg },
            Tone::Info => ToneColors { solid: self.info_solid, border: self.info_border, fg: self.info_fg, bg: self.info_bg },
            Tone::Accent => ToneColors { solid: self.accent, border: self.accent_border, fg: self.accent_fg, bg: self.accent_bg },
            Tone::Neutral => ToneColors { solid: self.border_strong, border: self.border_strong, fg: self.fg_2, bg: self.surface_2 },
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

/// Spacing scale (px), `space.*` in the tokens.
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

/// Corner radii (px), `radius.*`.
pub mod radius {
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
}

/// Control heights (px), `control.height.*`.
#[allow(dead_code)]
pub mod control {
    pub const SM: f32 = 28.0;
    pub const MD: f32 = 36.0;
    pub const LG: f32 = 42.0;
}

/// Layout constants (px), `layout.*`.
pub mod layout {
    pub const WINDOW_MIN_W: f32 = 960.0;
    pub const WINDOW_MIN_H: f32 = 640.0;
    pub const WINDOW_W: f32 = 1180.0;
    pub const WINDOW_H: f32 = 800.0;
    pub const RAIL_W: f32 = 212.0;
    pub const PAGE_PAD: f32 = 24.0;
    pub const DIALOG_W: f32 = 520.0;
    pub const FORM_LABEL_W: f32 = 168.0;
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

pub const DISPLAY: TypeStyle = TypeStyle { size_px: 28.0, line_px: 34.0, weight: 700.0 };
pub const H1: TypeStyle = TypeStyle { size_px: 24.0, line_px: 30.0, weight: 700.0 };
pub const H2: TypeStyle = TypeStyle { size_px: 16.0, line_px: 21.0, weight: 600.0 };
pub const BODY: TypeStyle = TypeStyle { size_px: 14.0, line_px: 21.0, weight: 400.0 };
pub const BODY_SM: TypeStyle = TypeStyle { size_px: 13.0, line_px: 19.0, weight: 400.0 };
pub const CAPTION: TypeStyle = TypeStyle { size_px: 12.0, line_px: 16.0, weight: 400.0 };
const _: () = assert!(CAPTION.size_px >= 12.0, "never below 12 px");
pub const MONO: TypeStyle = TypeStyle { size_px: 13.0, line_px: 20.0, weight: 400.0 };
pub const LOG: TypeStyle = TypeStyle { size_px: 12.0, line_px: 20.0, weight: 400.0 };

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
            ThemeChoice::System => "Follow system",
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

/// Text size setting (design: 100 / 115 / 130 %).
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
            TextScale::Default => "Default (100%)",
            TextScale::Large => "Large (115%)",
            TextScale::Largest => "Largest (130%)",
        }
    }
    /// Window rem size in px.
    pub fn rem_px(self) -> f32 {
        16.0 * self.factor()
    }
}

/// Alpha of the scrim behind dialogs (`scrim.alpha`).
pub const SCRIM_ALPHA: f32 = 0.6;

pub fn os_is_dark(cx: &App) -> bool {
    matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

// ------------------------------------------------------------------------------------------------ mapping onto GPUI Kit

/// Install `palette` into the GPUI Kit theme and into the [`ActivePalette`] global, then repaint every window.
pub fn apply(palette: Palette, cx: &mut App) {
    Theme::change(if palette.dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    cx.set_global(ActivePalette(palette));
    let p = palette;
    Theme::update(cx, |t| {
        let c = &mut t.colors;
        c.background = p.canvas;
        c.foreground = p.fg;
        c.border = p.border;
        c.input = p.border_strong;
        c.muted = p.surface_2;
        c.muted_foreground = p.fg_2;
        c.accent = p.accent_bg;
        c.accent_foreground = p.accent_fg;
        c.ring = p.ring;
        c.caret = p.fg;
        c.selection = p.accent_bg;
        c.link = p.accent_fg;
        c.link_hover = p.accent_hover;
        c.link_active = p.accent_hover;
        c.overlay = p.scrim.opacity(SCRIM_ALPHA);
        c.window_border = p.border;
        // buttons
        c.primary = p.accent;
        c.primary_hover = p.accent_hover;
        c.primary_active = p.accent_pressed;
        c.primary_foreground = p.on_accent;
        c.secondary = p.surface;
        c.secondary_hover = p.surface_2;
        c.secondary_active = p.surface_3;
        c.secondary_foreground = p.fg;
        c.danger = p.danger;
        c.danger_hover = p.danger_hover;
        c.danger_active = p.danger_pressed;
        c.danger_foreground = p.on_danger;
        c.success = p.ok_solid;
        c.success_hover = p.ok_solid;
        c.success_active = p.ok_solid;
        c.success_foreground = p.on_accent;
        c.warning = p.warn_solid;
        c.warning_hover = p.warn_solid;
        c.warning_active = p.warn_solid;
        c.warning_foreground = p.on_accent;
        c.info = p.info_solid;
        c.info_hover = p.info_solid;
        c.info_active = p.info_solid;
        c.info_foreground = p.on_accent;
        // stock button variants read their own tokens first
        c.button = p.surface;
        c.button_hover = p.surface_2;
        c.button_active = p.surface_3;
        c.button_foreground = p.fg;
        c.button_primary = p.accent;
        c.button_primary_hover = p.accent_hover;
        c.button_primary_active = p.accent_pressed;
        c.button_primary_foreground = p.on_accent;
        c.button_secondary = p.surface;
        c.button_secondary_hover = p.surface_2;
        c.button_secondary_active = p.surface_3;
        c.button_secondary_foreground = p.fg;
        c.button_danger = p.danger;
        c.button_danger_hover = p.danger_hover;
        c.button_danger_active = p.danger_pressed;
        c.button_danger_foreground = p.on_danger;
        c.button_success = p.ok_solid;
        c.button_success_foreground = p.on_accent;
        c.button_warning = p.warn_solid;
        c.button_warning_foreground = p.on_accent;
        c.button_info = p.info_solid;
        c.button_info_foreground = p.on_accent;
        // surfaces
        c.popover = p.surface;
        c.popover_foreground = p.fg;
        c.list = p.surface;
        c.list_even = p.surface;
        c.list_head = p.surface_2;
        c.list_hover = p.surface_2;
        c.list_active = p.accent_bg;
        c.list_active_border = p.accent;
        c.table = p.surface;
        c.table_even = p.surface;
        c.table_head = p.surface_2;
        c.table_head_foreground = p.fg_2;
        c.table_hover = p.surface_2;
        c.table_active = p.accent_bg;
        c.table_active_border = p.accent;
        c.table_row_border = p.border;
        c.group_box = p.surface;
        c.group_box_foreground = p.fg;
        c.sidebar = p.sidebar;
        c.sidebar_foreground = p.fg;
        c.sidebar_border = p.border;
        c.sidebar_accent = p.accent_bg;
        c.sidebar_accent_foreground = p.accent_fg;
        c.sidebar_primary = p.accent;
        c.sidebar_primary_foreground = p.on_accent;
        c.title_bar = p.surface;
        c.title_bar_border = p.border;
        c.status_bar = p.surface;
        c.status_bar_border = p.border;
        // form controls
        c.progress_bar = p.accent;
        c.slider_bar = p.accent;
        c.slider_thumb = p.accent;
        c.switch = p.surface_3;
        c.switch_thumb = p.surface;
        c.skeleton = p.surface_3;
        c.scrollbar = gpui_kit::transparent_black();
        c.scrollbar_thumb = p.border_strong;
        c.scrollbar_thumb_hover = p.fg_2;
        c.tab = p.surface;
        c.tab_active = p.accent_bg;
        c.tab_active_foreground = p.accent_fg;
        c.tab_foreground = p.fg_2;
        c.tab_bar = p.surface_2;
        c.tab_bar_segmented = p.surface_2;
        t.radius = px(radius::MD);
        t.radius_lg = px(radius::LG);
        t.shadow = false;
        t.font_size = px(BODY.size_px);
        t.mono_font_size = px(MONO.size_px);
    });
}

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
                ("placeholder/surface", h.placeholder, h.surface, 4.5),
                ("on_danger/danger_pressed", h.on_danger, h.danger_pressed, 4.5),
                ("on_accent/accent_pressed", h.on_accent, h.accent_pressed, 4.5),
                ("log_fg/log_wrn_bg", h.log_fg, h.log_wrn_bg, 4.5),
                ("log_fg/log_err_bg", h.log_fg, h.log_err_bg, 4.5),
                ("on_accent/accent", h.on_accent, h.accent, 4.5),
                ("accent_fg/accent_bg", h.accent_fg, h.accent_bg, 4.5),
                ("on_danger/danger", h.on_danger, h.danger, 4.5),
                ("ok_fg/ok_bg", h.ok_fg, h.ok_bg, 4.5),
                ("warn_fg/warn_bg", h.warn_fg, h.warn_bg, 4.5),
                ("err_fg/err_bg", h.err_fg, h.err_bg, 4.5),
                ("info_fg/info_bg", h.info_fg, h.info_bg, 4.5),
                ("log_fg/log_bg", h.log_fg, h.log_bg, 4.5),
                ("log_dim/log_bg", h.log_dim, h.log_bg, 4.5),
                ("log_inf/log_bg", h.log_inf, h.log_bg, 4.5),
                ("log_wrn/log_bg", h.log_wrn, h.log_bg, 4.5),
                ("log_err/log_bg", h.log_err, h.log_bg, 4.5),
                ("border_strong/surface (UI component)", h.border_strong, h.surface, 3.0),
                ("ring/surface (focus ring)", h.ring, h.surface, 3.0),
            ];
            for (label, fg, bg, min) in pairs {
                let r = contrast(fg, bg);
                assert!(r >= min, "{name} {label}: {r:.2} < {min}");
            }
        }
    }

    /// Minimal flat-JSON reader: `"key": "#RRGGBB"`.
    fn token_hex(json: &str, key: &str) -> Option<u32> {
        let at = json.find(&format!("\"{key}\""))?;
        let rest = &json[at + key.len() + 2..];
        let q = rest.find('"')?;
        let v = &rest[q + 1..];
        let v = &v[..v.find('"')?];
        u32::from_str_radix(v.trim_start_matches('#'), 16).ok()
    }

    #[test]
    fn palettes_match_design_tokens_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/mockups/ui/tokens.json");
        let Ok(json) = std::fs::read_to_string(&path) else {
            eprintln!("tokens.json not found at {}; skipping", path.display());
            return;
        };
        for (theme, hexes) in [("light", &LIGHT), ("dark", &DARK)] {
            for (name, value) in hexes.fields() {
                let key = format!("color.{theme}.{name}");
                let want = token_hex(&json, &key).unwrap_or_else(|| panic!("token {key} missing from tokens.json"));
                assert_eq!(value, want, "{key}: theme.rs has {value:06X}, tokens.json has {want:06X}");
            }
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
        let p = Palette::light();
        assert_ne!(p.tone(Tone::Ok).bg, p.tone(Tone::Err).bg);
        assert!(!p.dark && Palette::dark().dark);
    }
}
