//! Everything that is compiled into the binary besides code: fonts, the logo and the icon vocabulary.
//!
//! * Fonts live in `crates/synofurb/assets/fonts/` (Inter 400/500/600/700 and JetBrains Mono 400/700, Latin subsets,
//!   SIL OFL 1.1, licences next to them) and are registered with GPUI's text system at start-up, so the look never
//!   depends on what the operating system has installed.
//! * Brand SVGs live in `crates/synofurb/assets/brand/` (see [`super::brand`]).
//! * Icons are Lucide, selected in [`super::icons`], with the Kit's default bundle as fallback for stock widgets.

use super::icons::RoleIcons;
use gpui_kit::{App, AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Interface font family (registered from [`FONT_FILES`]).
pub const FONT_UI: &str = "Inter";
/// Monospace family for code, logs, hashes, device paths and addresses.
pub const FONT_MONO: &str = "JetBrains Mono";

/// `(file name, bytes)` of every bundled font.
pub static FONT_FILES: &[(&str, &[u8])] = &[
    ("Inter-Regular.ttf", include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
    ("Inter-Medium.ttf", include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
    ("Inter-SemiBold.ttf", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
    ("Inter-Bold.ttf", include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
    ("JetBrainsMono-Regular.ttf", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf")),
    ("JetBrainsMono-Bold.ttf", include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf")),
];

/// The two licence texts shown on the About page.
pub const OFL_INTER: &str = include_str!("../../assets/fonts/OFL-Inter.txt");
pub const OFL_JETBRAINS_MONO: &str = include_str!("../../assets/fonts/OFL-JetBrainsMono.txt");

/// Register the bundled fonts with the text system. Call once, before the first window.
pub fn register_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = FONT_FILES.iter().map(|(_, b)| Cow::Borrowed(*b)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("warning: cannot register the bundled fonts: {e:#}");
    }
}

/// Asset source: our Lucide roles first, then the Kit's default icon bundle (used by stock widgets such as
/// checkboxes, selects and dialog close buttons).
pub struct UiAssets;

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(b) = RoleIcons.load(path)? {
            return Ok(Some(b));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v = RoleIcons.list(path)?;
        v.extend(gpui_kit::assets::Assets.list(path)?);
        v.sort();
        v.dedup();
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_are_real_fonts_with_the_right_family() {
        for (name, bytes) in FONT_FILES {
            assert!(bytes.len() > 20_000, "{name} looks truncated");
            assert_eq!(&bytes[..4], &[0, 1, 0, 0], "{name} is not a TrueType file");
        }
        let family = |b: &[u8]| String::from_utf8_lossy(b).contains("Inter") as u8;
        assert!(family(FONT_FILES[0].1) == 1);
    }

    #[test]
    fn licences_are_bundled() {
        assert!(OFL_INTER.contains("SIL OPEN FONT LICENSE"));
        assert!(OFL_JETBRAINS_MONO.contains("SIL OPEN FONT LICENSE"));
    }

    #[test]
    fn role_icons_win_and_defaults_still_load() {
        let a = UiAssets;
        assert!(a.load("icons/shield-check.svg").unwrap().is_some());
        assert!(a.load("icons/chevron-down.svg").unwrap().is_some());
    }
}
