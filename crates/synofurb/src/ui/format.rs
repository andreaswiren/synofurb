//! Display formatting helpers. Pure functions, unit-tested; units are binary (GiB/TiB) with two decimals, as the
//! UX copy rules ask.

use synofurb_core::model::{GIB, MIB};

/// `10.90 TiB`, `3.64 GiB`, `512 MiB`, `700 B`.
pub fn size_2dp(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const TIB: u64 = 1024 * GIB;
    if bytes >= TIB {
        format!("{:.2} TiB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.0} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// File size for downloads: `36.6 MiB` (one decimal), `812 KiB`.
pub fn human(bytes: u64) -> String {
    synofurb_core::model::human_bytes(bytes)
}

/// MiB (as the plan uses) to `x GiB`, no decimals when whole.
pub fn gib_from_mib(mib: u64) -> String {
    let g = mib as f64 / 1024.0;
    if (g - g.round()).abs() < 0.005 { format!("{} GiB", g.round() as u64) } else { format!("{g:.1} GiB") }
}

/// `1 disk`, `4 disks`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `0.42` -> `42%`; clamps to 0..=100.
pub fn percent(fraction: f32) -> String {
    format!("{:.0}%", (fraction.clamp(0.0, 1.0) * 100.0).round())
}

/// Characters shown for a secret that is hidden.
pub fn mask(s: &str) -> String {
    "\u{2022}".repeat(s.chars().count().min(32))
}

/// `192.168.1.20 (demo)` style hosts are never valid addresses; strip a trailing parenthetical for display only.
pub fn display_host(h: &str) -> String {
    h.split(" (").next().unwrap_or(h).trim().to_string()
}

/// Shorten a long path in the middle for narrow rows: `/home/me/.../ds918_ed25519`.
pub fn shorten_path(p: &str, max: usize) -> String {
    let n = p.chars().count();
    if n <= max || max < 8 {
        return p.to_string();
    }
    let keep = (max - 3) / 2;
    let head: String = p.chars().take(keep).collect();
    let tail: String = p.chars().skip(n - (max - 3 - keep)).collect();
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_binary_units_and_two_decimals() {
        assert_eq!(size_2dp(0), "0 B");
        assert_eq!(size_2dp(1023), "1023 B");
        assert_eq!(size_2dp(2048), "2 KiB");
        assert_eq!(size_2dp(512 * MIB), "512 MiB");
        assert_eq!(size_2dp(4 * GIB), "4.00 GiB");
        assert_eq!(size_2dp(11 * 1024 * GIB + 900 * GIB), "11.88 TiB");
    }

    #[test]
    fn mib_to_gib() {
        assert_eq!(gib_from_mib(8192), "8 GiB");
        assert_eq!(gib_from_mib(9728), "9.5 GiB");
    }

    #[test]
    fn plurals_percent_duration() {
        assert_eq!(plural(1, "disk", "disks"), "1 disk");
        assert_eq!(plural(4, "disk", "disks"), "4 disks");
        assert_eq!(percent(0.424), "42%");
        assert_eq!(percent(7.0), "100%");
        assert_eq!(percent(-1.0), "0%");
    }

    #[test]
    fn masking_and_hosts() {
        assert_eq!(mask("abc"), "\u{2022}\u{2022}\u{2022}");
        assert_eq!(mask(&"x".repeat(100)).chars().count(), 32, "never reveals the length of long secrets");
        assert_eq!(display_host("192.168.1.20 (demo)"), "192.168.1.20");
        assert_eq!(display_host("nas.local"), "nas.local");
    }

    #[test]
    fn path_shortening() {
        assert_eq!(shorten_path("/a/b", 20), "/a/b");
        let s = shorten_path("/home/someone/.local/share/synofurb/keys/nas_ed25519", 30);
        assert_eq!(s.chars().count(), 30);
        assert!(s.contains("...") && s.ends_with("_ed25519"));
    }
}
