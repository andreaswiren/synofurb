//! Update check outcomes, as the UI shows them. Pure (no GPUI), so the mapping is unit-tested.
//!
//! The engine's `UpdateError` says *what* went wrong; the UI needs to know whether to call it "offline". Only a
//! transport failure (the PC cannot reach GitHub) may switch the Offline chip on: a 404 (private repository), a rate
//! limit, an unreadable answer or an HTTP error means GitHub was reached. The engine has one `Http` variant for both a
//! failed request and an unexpected status, so for that variant the worker asks a typed question (can a TCP connection
//! to the API host be opened?) instead of reading the message text; see `worker::github_reachable`.

use super::copy::{self, Item};
use synofurb_core::updater::{UpdateError, UpdateInfo};

/// What kind of failure a check ended in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailKind {
    /// This computer cannot reach GitHub (no Internet, DNS, firewall). The only kind that means "offline".
    Offline,
    /// GitHub answered 404: no such repository or releases, or the repository is private.
    NotFound,
    RateLimited,
    /// The release list has nothing for this channel.
    NoRelease,
    /// GitHub was reached but answered with an error.
    Server,
    /// The answer could not be read.
    Unreadable,
    Other,
}

/// A failed check: the kind (typed) and the engine's own text (shown in the info popover).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckFailure {
    pub kind: FailKind,
    pub detail: String,
}

/// Map an engine error to a failure kind. `reachable` is asked only for the ambiguous `Http` variant.
pub fn classify(e: &UpdateError, reachable: impl FnOnce() -> bool) -> CheckFailure {
    let kind = match e {
        UpdateError::NotFound(_) => FailKind::NotFound,
        UpdateError::RateLimited { .. } => FailKind::RateLimited,
        UpdateError::NoRelease => FailKind::NoRelease,
        UpdateError::Parse(_) => FailKind::Unreadable,
        UpdateError::Http(_) => {
            if reachable() {
                FailKind::Server
            } else {
                FailKind::Offline
            }
        }
        _ => FailKind::Other,
    };
    CheckFailure { kind, detail: e.to_string() }
}

impl CheckFailure {
    /// Only a transport failure counts as "the app is offline".
    pub fn offline(&self) -> bool {
        self.kind == FailKind::Offline
    }

    /// The few words after "Couldn't check:".
    pub fn reason(&self) -> &'static Item {
        match self.kind {
            FailKind::Offline => &copy::UPD_FAIL_OFFLINE,
            FailKind::NotFound => &copy::UPD_FAIL_NOTFOUND,
            FailKind::RateLimited => &copy::UPD_FAIL_RATE,
            FailKind::NoRelease => &copy::UPD_FAIL_NORELEASE,
            FailKind::Server => &copy::UPD_FAIL_SERVER,
            FailKind::Unreadable => &copy::UPD_FAIL_PARSE,
            FailKind::Other => &copy::UPD_FAIL_OTHER,
        }
    }

    /// The full explanation (info popover): our own sentence for the kinds that have one, the engine's text otherwise
    /// (the rate limit hint with the time is the engine's).
    pub fn full(&self) -> String {
        match self.kind {
            FailKind::NotFound | FailKind::NoRelease => self.reason().body.to_string(),
            FailKind::Offline => format!("{} ({})", self.reason().body, self.detail),
            _ => self.detail.clone(),
        }
    }
}

/// Can the app install this update itself? Not while the build cannot verify signatures (placeholder key), and not
/// when the release lacks the files a verified install needs.
pub fn can_install(info: &UpdateInfo, key_is_placeholder: bool) -> bool {
    !key_is_placeholder && info.installable()
}

/// The page to open for a release: GitHub's own link when it is an https github.com URL, else built from the tag.
pub fn release_url(info: &UpdateInfo, repo: &str) -> String {
    match info.html_url.as_deref() {
        Some(u) if u.starts_with("https://github.com/") => u.to_string(),
        _ => {
            let tag = if info.tag.is_empty() { format!("v{}", info.latest) } else { info.tag.clone() };
            format!("https://github.com/{repo}/releases/tag/{tag}")
        }
    }
}

/// What the About page says after the version: `release` for a build made from a tag that matches the version, the
/// commit for any other build (`dev <commit>`), and `local build` when the build knew nothing about git.
pub fn build_label(describe: &str, version: &str) -> String {
    let d = describe.trim();
    if d.is_empty() {
        copy::ABOUT_BUILD_LOCAL.short.to_string()
    } else if d.trim_start_matches('v') == version {
        copy::ABOUT_BUILD_RELEASE.short.to_string()
    } else {
        format!("{} {d}", copy::ABOUT_BUILD_DEV.short)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::updater::{AssetInfo, Channel};

    fn never() -> bool {
        panic!("the reachability probe must not run for this error")
    }

    fn info() -> UpdateInfo {
        UpdateInfo {
            current: "0.2.0".into(),
            latest: "0.2.1".into(),
            tag: "v0.2.1".into(),
            update_available: true,
            channel: Channel::Stable,
            prerelease: false,
            published_at: None,
            notes: String::new(),
            html_url: Some("https://github.com/andreaswiren/synofurb/releases/tag/v0.2.1".into()),
            asset: Some(AssetInfo { name: "a".into(), url: "https://github.com/a".into(), size: 1 }),
            checksums: Some(AssetInfo { name: "s".into(), url: "https://github.com/s".into(), size: 1 }),
            signature: Some(AssetInfo { name: "m".into(), url: "https://github.com/m".into(), size: 1 }),
            problems: vec![],
        }
    }

    #[test]
    fn a_private_repository_is_not_offline() {
        let f = classify(&UpdateError::NotFound("andreaswiren/synofurb".into()), never);
        assert_eq!(f.kind, FailKind::NotFound);
        assert!(!f.offline());
        assert_eq!(f.reason().short, "Releases not found");
        assert!(f.full().contains("GitHub did not find the releases"));
        assert!(f.full().contains("If the repository is private, the update check cannot see them."));
    }

    #[test]
    fn only_a_transport_failure_is_offline() {
        let http = UpdateError::Http("request to https://api.github.com/... failed: dns error".into());
        let off = classify(&http, || false);
        assert_eq!(off.kind, FailKind::Offline);
        assert!(off.offline());
        assert!(off.full().starts_with("synofurb could not reach GitHub."));
        // the same engine variant while GitHub is reachable is a server error, not "offline"
        let srv = classify(&http, || true);
        assert_eq!(srv.kind, FailKind::Server);
        assert!(!srv.offline());
        assert_eq!(srv.full(), http.to_string(), "the engine text is shown as it is");
        for e in [
            UpdateError::NotFound("x".into()),
            UpdateError::RateLimited { reset_at: None, retry_after: Some(120) },
            UpdateError::NoRelease,
            UpdateError::Parse("bad".into()),
            UpdateError::NoSigningKey,
            UpdateError::TooManyRedirects,
        ] {
            assert!(!classify(&e, never).offline(), "{e}");
        }
    }

    #[test]
    fn every_kind_has_text_and_the_engine_hint_survives() {
        let rl = classify(&UpdateError::RateLimited { reset_at: None, retry_after: Some(120) }, never);
        assert_eq!(rl.kind, FailKind::RateLimited);
        assert!(rl.full().contains("rate limit") && rl.full().contains("2 minute"), "the engine's hint with the time: {}", rl.full());
        let nr = classify(&UpdateError::NoRelease, never);
        assert_eq!(nr.full(), "No release found for this system.");
        let parse = classify(&UpdateError::Parse("expected value".into()), never);
        assert_eq!((parse.kind, parse.reason().short), (FailKind::Unreadable, "Unreadable answer"));
        assert!(parse.full().contains("expected value"));
        assert_eq!(classify(&UpdateError::TooManyRedirects, never).kind, FailKind::Other);
        let kinds = [
            FailKind::Offline,
            FailKind::NotFound,
            FailKind::RateLimited,
            FailKind::NoRelease,
            FailKind::Server,
            FailKind::Unreadable,
            FailKind::Other,
        ];
        let mut shorts: Vec<&str> = kinds.iter().map(|k| CheckFailure { kind: *k, detail: "d".into() }.reason().short).collect();
        assert!(shorts.iter().all(|s| copy::words(s) <= 4 && !s.is_empty()));
        shorts.sort_unstable();
        shorts.dedup();
        assert_eq!(shorts.len(), kinds.len(), "every kind has its own words");
    }

    #[test]
    fn an_unsigned_build_never_installs_by_itself() {
        let i = info();
        assert!(can_install(&i, false));
        assert!(!can_install(&i, true), "placeholder key: no automatic install");
        let mut no_sig = info();
        no_sig.signature = None;
        assert!(!can_install(&no_sig, false), "a release without a signature file cannot be verified");
    }

    #[test]
    fn the_release_page_is_github_or_built_from_the_tag() {
        let repo = "andreaswiren/synofurb";
        assert_eq!(release_url(&info(), repo), "https://github.com/andreaswiren/synofurb/releases/tag/v0.2.1");
        let mut i = info();
        i.html_url = Some("https://evil.example/x".into());
        assert_eq!(release_url(&i, repo), "https://github.com/andreaswiren/synofurb/releases/tag/v0.2.1", "only github.com links are opened");
        i.html_url = None;
        i.tag.clear();
        assert_eq!(release_url(&i, repo), "https://github.com/andreaswiren/synofurb/releases/tag/v0.2.1");
    }

    #[test]
    fn a_release_build_does_not_say_dev() {
        assert_eq!(build_label("v0.2.1", "0.2.1"), "release", "the workflow sets SYNOFURB_GIT_DESCRIBE to the tag");
        assert_eq!(build_label("0.2.1", "0.2.1"), "release");
        assert_eq!(build_label("e8d3413", "0.2.1"), "dev e8d3413");
        assert_eq!(build_label("v0.2.0-5-gabc1234", "0.2.1"), "dev v0.2.0-5-gabc1234");
        assert_eq!(build_label("", "0.2.1"), "local build");
        assert_eq!(build_label("  ", "0.2.1"), "local build");
    }
}
