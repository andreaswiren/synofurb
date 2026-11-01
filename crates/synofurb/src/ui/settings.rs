//! Small per-user UI preferences (theme, text size, update channel), stored as JSON next to the inventory.
//! Pure data + file IO; no GPUI types, so it is unit-tested directly.

use super::theme::{Accent, TextScale, ThemeChoice};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub theme: ThemeChoice,
    pub text_scale: TextScale,
    pub accent: Accent,
    /// `stable` or `prerelease` (matches `updater::Channel`).
    pub update_channel: String,
    /// Look for a new release once at startup (and daily while running).
    pub check_updates: bool,
    /// A release the user chose to skip.
    pub skip_version: Option<String>,
    /// Unix seconds before which the update banner stays hidden ("remind me tomorrow").
    pub remind_after: Option<u64>,
    /// Replace the looping spinner by a static glyph.
    pub reduce_motion: bool,
    /// Offer to report an install result (off by default; a report always asks first).
    pub send_results: bool,
    /// The "Before you start" dialog has been shown once.
    pub first_launch_done: bool,
    /// Folder of the offline bundle chosen for Source: Offline.
    pub bundle_dir: Option<String>,
}

impl Default for UiSettings {
    fn default() -> Self {
        UiSettings {
            theme: ThemeChoice::System,
            text_scale: TextScale::Default,
            accent: Accent::Purple,
            update_channel: "stable".into(),
            check_updates: true,
            skip_version: None,
            remind_after: None,
            reduce_motion: false,
            send_results: false,
            first_launch_done: false,
            bundle_dir: None,
        }
    }
}

impl UiSettings {
    /// Demo runs never read or write the user's preferences; `SYNOFURB_THEME=light|dark` picks the theme (screenshots).
    pub fn for_demo() -> UiSettings {
        let theme = match std::env::var("SYNOFURB_THEME").unwrap_or_default().to_ascii_lowercase().as_str() {
            "dark" => ThemeChoice::Dark,
            "light" => ThemeChoice::Light,
            _ => ThemeChoice::System,
        };
        let text_scale = match std::env::var("SYNOFURB_TEXT").unwrap_or_default().to_ascii_lowercase().as_str() {
            "large" => TextScale::Large,
            "largest" => TextScale::Largest,
            _ => TextScale::Default,
        };
        UiSettings { theme, text_scale, ..Default::default() }
    }

    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("ui-settings.json")
    }

    /// Missing or corrupt files give the defaults (a bad preferences file must never stop the app).
    pub fn load(path: &Path) -> UiSettings {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }

    /// True if an update to `version` should be announced now.
    pub fn should_announce(&self, version: &str, now_unix: u64) -> bool {
        self.check_updates && self.skip_version.as_deref() != Some(version) && self.remind_after.is_none_or(|t| now_unix >= t)
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sf-ui-{tag}-{}", std::process::id()))
    }

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tmp("settings");
        let path = UiSettings::path(&dir);
        let s =
            UiSettings { theme: ThemeChoice::Dark, text_scale: TextScale::Large, skip_version: Some("1.2.3".into()), ..Default::default() };
        s.save(&path).unwrap();
        assert_eq!(UiSettings::load(&path), s);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(UiSettings::load(&path), UiSettings::default(), "corrupt file falls back to defaults");
        assert_eq!(UiSettings::load(&dir.join("missing.json")), UiSettings::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_files_keep_defaults_for_new_fields() {
        let s: UiSettings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(s.check_updates);
        assert_eq!(s.update_channel, "stable");
    }

    #[test]
    fn announcement_rules() {
        let mut s = UiSettings::default();
        assert!(s.should_announce("2.0.0", 100));
        s.skip_version = Some("2.0.0".into());
        assert!(!s.should_announce("2.0.0", 100));
        assert!(s.should_announce("2.0.1", 100), "skipping one version does not skip the next");
        s.skip_version = None;
        s.remind_after = Some(500);
        assert!(!s.should_announce("2.0.0", 499));
        assert!(s.should_announce("2.0.0", 500));
        s.check_updates = false;
        assert!(!s.should_announce("2.0.0", 900));
    }
}
