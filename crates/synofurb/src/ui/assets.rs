//! Asset source: the handful of Lucide icons this app uses (embedded, `assets/icons/*.svg`) on top of the
//! default GPUI Kit bundle (`gpui_kit::assets::Assets`, used by stock components such as checkboxes and selects).
//!
//! To add an icon: copy the SVG from `gpui-kit-assets/assets/icons/` into `src/ui/assets/icons/`, add a line to
//! [`ICONS`] and refer to it as `icon("name")` in widgets.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Embedded icons: `(asset path, svg bytes)`.
pub static ICONS: &[(&str, &[u8])] = &[
    ("icons/arrow-left.svg", include_bytes!("assets/icons/arrow-left.svg")),
    ("icons/arrow-right.svg", include_bytes!("assets/icons/arrow-right.svg")),
    ("icons/ban.svg", include_bytes!("assets/icons/ban.svg")),
    ("icons/bug.svg", include_bytes!("assets/icons/bug.svg")),
    ("icons/cable.svg", include_bytes!("assets/icons/cable.svg")),
    ("icons/check.svg", include_bytes!("assets/icons/check.svg")),
    ("icons/chevron-down.svg", include_bytes!("assets/icons/chevron-down.svg")),
    ("icons/chevron-right.svg", include_bytes!("assets/icons/chevron-right.svg")),
    ("icons/circle-check.svg", include_bytes!("assets/icons/circle-check.svg")),
    ("icons/circle-dashed.svg", include_bytes!("assets/icons/circle-dashed.svg")),
    ("icons/circle-dot.svg", include_bytes!("assets/icons/circle-dot.svg")),
    ("icons/circle-play.svg", include_bytes!("assets/icons/circle-play.svg")),
    ("icons/circle-question-mark.svg", include_bytes!("assets/icons/circle-question-mark.svg")),
    ("icons/circle-x.svg", include_bytes!("assets/icons/circle-x.svg")),
    ("icons/clipboard-check.svg", include_bytes!("assets/icons/clipboard-check.svg")),
    ("icons/copy.svg", include_bytes!("assets/icons/copy.svg")),
    ("icons/cpu.svg", include_bytes!("assets/icons/cpu.svg")),
    ("icons/database.svg", include_bytes!("assets/icons/database.svg")),
    ("icons/download.svg", include_bytes!("assets/icons/download.svg")),
    ("icons/external-link.svg", include_bytes!("assets/icons/external-link.svg")),
    ("icons/eye.svg", include_bytes!("assets/icons/eye.svg")),
    ("icons/eye-off.svg", include_bytes!("assets/icons/eye-off.svg")),
    ("icons/file-text.svg", include_bytes!("assets/icons/file-text.svg")),
    ("icons/flag.svg", include_bytes!("assets/icons/flag.svg")),
    ("icons/folder-open.svg", include_bytes!("assets/icons/folder-open.svg")),
    ("icons/git-branch.svg", include_bytes!("assets/icons/git-branch.svg")),
    ("icons/hard-drive.svg", include_bytes!("assets/icons/hard-drive.svg")),
    ("icons/heart-pulse.svg", include_bytes!("assets/icons/heart-pulse.svg")),
    ("icons/info.svg", include_bytes!("assets/icons/info.svg")),
    ("icons/key-round.svg", include_bytes!("assets/icons/key-round.svg")),
    ("icons/layers.svg", include_bytes!("assets/icons/layers.svg")),
    ("icons/life-buoy.svg", include_bytes!("assets/icons/life-buoy.svg")),
    ("icons/loader-circle.svg", include_bytes!("assets/icons/loader-circle.svg")),
    ("icons/lock.svg", include_bytes!("assets/icons/lock.svg")),
    ("icons/monitor.svg", include_bytes!("assets/icons/monitor.svg")),
    ("icons/moon.svg", include_bytes!("assets/icons/moon.svg")),
    ("icons/octagon-x.svg", include_bytes!("assets/icons/octagon-x.svg")),
    ("icons/package.svg", include_bytes!("assets/icons/package.svg")),
    ("icons/play.svg", include_bytes!("assets/icons/play.svg")),
    ("icons/plug-zap.svg", include_bytes!("assets/icons/plug-zap.svg")),
    ("icons/power.svg", include_bytes!("assets/icons/power.svg")),
    ("icons/radar.svg", include_bytes!("assets/icons/radar.svg")),
    ("icons/refresh-cw.svg", include_bytes!("assets/icons/refresh-cw.svg")),
    ("icons/rocket.svg", include_bytes!("assets/icons/rocket.svg")),
    ("icons/save.svg", include_bytes!("assets/icons/save.svg")),
    ("icons/scan-search.svg", include_bytes!("assets/icons/scan-search.svg")),
    ("icons/search.svg", include_bytes!("assets/icons/search.svg")),
    ("icons/server.svg", include_bytes!("assets/icons/server.svg")),
    ("icons/settings.svg", include_bytes!("assets/icons/settings.svg")),
    ("icons/shield.svg", include_bytes!("assets/icons/shield.svg")),
    ("icons/shield-alert.svg", include_bytes!("assets/icons/shield-alert.svg")),
    ("icons/shield-check.svg", include_bytes!("assets/icons/shield-check.svg")),
    ("icons/sparkles.svg", include_bytes!("assets/icons/sparkles.svg")),
    ("icons/square-terminal.svg", include_bytes!("assets/icons/square-terminal.svg")),
    ("icons/sun.svg", include_bytes!("assets/icons/sun.svg")),
    ("icons/terminal.svg", include_bytes!("assets/icons/terminal.svg")),
    ("icons/timer.svg", include_bytes!("assets/icons/timer.svg")),
    ("icons/trash.svg", include_bytes!("assets/icons/trash.svg")),
    ("icons/triangle-alert.svg", include_bytes!("assets/icons/triangle-alert.svg")),
    ("icons/undo-2.svg", include_bytes!("assets/icons/undo-2.svg")),
    ("icons/upload.svg", include_bytes!("assets/icons/upload.svg")),
    ("icons/usb.svg", include_bytes!("assets/icons/usb.svg")),
    ("icons/users.svg", include_bytes!("assets/icons/users.svg")),
    ("icons/wrench.svg", include_bytes!("assets/icons/wrench.svg")),
    ("icons/x.svg", include_bytes!("assets/icons/x.svg")),
];

pub struct UiAssets;

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ICONS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(*bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect();
        v.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_icon_is_an_svg_with_currentcolor() {
        for (path, bytes) in ICONS {
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.contains("<svg") && text.contains("currentColor"), "{path} must be a recolourable svg");
        }
    }

    #[test]
    fn own_icons_win_and_defaults_still_load() {
        let a = UiAssets;
        assert!(a.load("icons/shield-check.svg").unwrap().is_some());
        assert!(a.load("icons/chevron-down.svg").unwrap().is_some(), "falls back to the Kit bundle");
    }
}
