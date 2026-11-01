//! The synofurb logo. Everything that draws the mark or the logotype goes through [`mark`] and [`logo`], so a new
//! logo is a file swap in `crates/synofurb/assets/brand/` (the designer's SVGs, copied from `assets/brand/`).
//!
//! The SVGs are multi-colour, so they are drawn as images (GPUI's `svg()` element is a single-colour mask and would
//! flatten them). There are light and dark variants of each; the right one follows the active theme.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{AnyElement, App, Image, ImageFormat, IntoElement, ObjectFit, Styled, StyledImage, img, px};
use std::sync::{Arc, LazyLock};

const MARK_LIGHT: &str = include_str!("../../assets/brand/synofurb-mark.svg");
const MARK_DARK: &str = include_str!("../../assets/brand/synofurb-mark-dark.svg");
const LOGO_LIGHT: &str = include_str!("../../assets/brand/synofurb-logo.svg");
const LOGO_DARK: &str = include_str!("../../assets/brand/synofurb-logo-dark.svg");
/// The app icon (mark on a tile), for the window icon and the About page.
pub const APP_ICON_SVG: &str = include_str!("../../assets/brand/app-icon.svg");

/// Width / height of the logotype artwork (`viewBox="0 0 390 108"`).
pub const LOGO_ASPECT: f32 = 390.0 / 108.0;

fn image(svg: &'static str) -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec()))
}

static IMAGES: LazyLock<[Arc<Image>; 4]> = LazyLock::new(|| [image(MARK_LIGHT), image(MARK_DARK), image(LOGO_LIGHT), image(LOGO_DARK)]);

fn pick(cx: &App, light: usize) -> Arc<Image> {
    IMAGES[if cx.theme().is_dark() { light + 1 } else { light }].clone()
}

/// The recycle mark alone, `size` px square.
pub fn mark(cx: &App, size: f32) -> AnyElement {
    img(pick(cx, 0)).w(px(size)).h(px(size)).object_fit(ObjectFit::Contain).flex_none().into_any_element()
}

/// Mark plus the "synofurb" logotype, `height` px tall.
pub fn logo(cx: &App, height: f32) -> AnyElement {
    img(pick(cx, 2)).w(px(height * LOGO_ASPECT)).h(px(height)).object_fit(ObjectFit::Contain).flex_none().into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_svgs_are_present_and_well_formed() {
        for (name, s) in
            [("mark", MARK_LIGHT), ("mark-dark", MARK_DARK), ("logo", LOGO_LIGHT), ("logo-dark", LOGO_DARK), ("icon", APP_ICON_SVG)]
        {
            assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"), "{name}");
            assert!(!s.contains("<text"), "{name}: the logotype must be outlined paths, not live text (no font dependency)");
        }
    }

    #[test]
    fn logo_aspect_matches_the_artwork() {
        assert!(LOGO_LIGHT.contains("viewBox=\"0 0 390 108\""));
    }
}
