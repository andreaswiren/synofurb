//! Self-updater for the desktop app.
//!
//! Flow: [`check`] asks the GitHub releases API for the newest release on a [`Channel`], compares versions
//! (semver, a leading `v` is ignored, build metadata is ignored) and picks the asset for this OS/architecture by
//! the naming contract in `docs/UPDATES.md`. [`download_verified`] downloads that asset, but only after
//!
//! 1. the minisign signature of `SHA256SUMS` verified against [`UPDATE_PUBLIC_KEY`], and
//! 2. the SHA-256 of the downloaded file matched its line in `SHA256SUMS`.
//!
//! [`apply`] then installs it (Windows: start the installer; macOS: open the disk image; Linux: replace the
//! executable atomically and keep the old one as `<name>.old`, or print/run the package command).
//!
//! Nothing is ever installed unverified, and while [`UPDATE_PUBLIC_KEY`] is still the placeholder,
//! [`download_verified`] refuses with [`UpdateError::NoSigningKey`] instead of skipping the check.
//!
//! **The unsigned path.** While (and only while) the embedded key is the placeholder, a person can opt in to
//! [`download_unchecked_signature`]: the same host policy, size limits and streaming SHA-256 check against the
//! `SHA256SUMS` of the same GitHub release, but no signature. That protects against a corrupted download, not against
//! a tampered release. It needs an [`UnsignedConsent`] token, is refused with [`UpdateError::UnsignedNotAllowed`] as
//! soon as a real key is embedded (signatures become mandatory automatically), and the result is labelled
//! ([`Integrity::ChecksumOnly`], `signed: false`) so the UI can say so. [`UpdateInfo::install_mode`] summarises what
//! is possible for a release.
//!
//! The network layer is thin and every decision (version comparison, release selection, asset choice, URL policy,
//! checksum parsing, apply planning) is a pure function with unit tests.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

/// Repository the app updates itself from.
pub const DEFAULT_REPO: &str = "andreaswiren/synofurb";
const API_BASE: &str = "https://api.github.com";

/// ================================================================================================================
/// PLACEHOLDER. This is NOT a real key and nothing can be verified against it.
///
/// Generate the real key pair with `minisign -G` (see `docs/UPDATES.md`), store the secret key as the GitHub
/// secret `MINISIGN_SECRET_KEY`, and replace [`UPDATE_PUBLIC_KEY`] below with the base64 line of the `.pub` file
/// (it starts with `RW`). Until then the updater refuses to download or install anything.
/// ================================================================================================================
pub const UPDATE_PUBLIC_KEY_PLACEHOLDER: &str = "PLACEHOLDER-run-minisign-G-and-paste-the-public-key-here";
/// The minisign public key releases are verified against. Replace the right-hand side with the real key.
pub const UPDATE_PUBLIC_KEY: &str = UPDATE_PUBLIC_KEY_PLACEHOLDER;

/// Name of the checksum file and of its detached minisign signature in every release.
pub const SUMS_ASSET: &str = "SHA256SUMS";
pub const SIGNATURE_ASSET: &str = "SHA256SUMS.minisig";
/// File extensions an installable asset may have: `synofurb-<version>-<os>-<arch>.<ext>`.
pub const EXTENSIONS: [&str; 9] = ["msi", "exe", "dmg", "pkg", "deb", "rpm", "AppImage", "tar.gz", "zip"];

const MAX_API_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SUMS_BYTES: u64 = 1024 * 1024;
const MAX_SIG_BYTES: u64 = 16 * 1024;
const MAX_ASSET_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_NOTES_BYTES: usize = 64 * 1024;
const MAX_REDIRECTS: usize = 5;

// ---------------------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("invalid repository name {0:?} (expected owner/name)")]
    BadRepo(String),
    #[error("invalid version {0:?}: {1}")]
    BadVersion(String, String),
    #[error("{0}")]
    Http(String),
    #[error("GitHub API rate limit reached; {}", rate_limit_hint(.reset_at, .retry_after))]
    RateLimited { reset_at: Option<u64>, retry_after: Option<u64> },
    #[error("repository or releases not found: {0}")]
    NotFound(String),
    #[error("could not understand the release list: {0}")]
    Parse(String),
    #[error("no suitable release found")]
    NoRelease,
    #[error("refusing {0:?}: updates must use https")]
    NotHttps(String),
    #[error("refusing to download from host {0:?}: only github.com and githubusercontent.com are allowed")]
    UnsupportedHost(String),
    #[error("refusing malformed URL {0:?}")]
    BadUrl(String),
    #[error("too many redirects")]
    TooManyRedirects,
    #[error(
        "update signing key is a placeholder, so updates cannot be verified and will not be installed. \
         The maintainer must generate a key with `minisign -G` and set UPDATE_PUBLIC_KEY (see docs/UPDATES.md)"
    )]
    NoSigningKey,
    #[error(
        "this build contains a real update signing key, so releases must be signed; installing without a signature is not possible here"
    )]
    UnsignedNotAllowed,
    #[error("the update signing key is not a valid minisign public key: {0}")]
    BadSigningKey(String),
    #[error("the release has no {0} asset; cannot verify the download")]
    MissingAsset(&'static str),
    #[error("no build of this release matches this system ({0})")]
    NoAssetForTarget(String),
    #[error("unsafe asset name {0:?}")]
    BadAssetName(String),
    #[error("signature of SHA256SUMS is not valid: {0}")]
    SignatureInvalid(String),
    #[error("SHA256SUMS is malformed: {0}")]
    BadChecksumFile(String),
    #[error("SHA256SUMS has no entry for {0}")]
    ChecksumMissing(String),
    #[error("checksum mismatch for {name}: expected {expected}, got {actual}")]
    ChecksumMismatch { name: String, expected: String, actual: String },
    #[error("download of {name} is larger than expected ({limit} bytes)")]
    TooLarge { name: String, limit: u64 },
    #[error("download of {name} has {got} bytes but the release says {want}")]
    SizeMismatch { name: String, got: u64, want: u64 },
    #[error("could not apply the update: {0}")]
    Apply(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, UpdateError>;

fn rate_limit_hint(reset_at: &Option<u64>, retry_after: &Option<u64>) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let secs = retry_after.or_else(|| reset_at.map(|r| r.saturating_sub(now)));
    match secs {
        Some(s) => format!("try again in about {} minute(s)", s.div_ceil(60).max(1)),
        None => "try again later".into(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Channel, target, versions
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Newest release that is neither flagged "pre-release" on GitHub nor has a pre-release version (`-rc.1`).
    #[default]
    Stable,
    /// Newest release of any kind, pre-releases included.
    Prerelease,
}

impl FromStr for Channel {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Channel, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stable" | "release" => Ok(Channel::Stable),
            "prerelease" | "pre-release" | "pre" | "beta" | "rc" => Ok(Channel::Prerelease),
            other => Err(format!("unknown update channel {other:?} (use stable or prerelease)")),
        }
    }
}

impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Prerelease => "prerelease",
        }
    }
}

/// Operating system and CPU architecture in the vocabulary of the asset naming contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `windows`, `macos` or `linux`
    pub os: String,
    /// `x86_64` or `aarch64`
    pub arch: String,
}

impl Target {
    pub fn current() -> Target {
        Target { os: std::env::consts::OS.to_string(), arch: std::env::consts::ARCH.to_string() }
    }
    pub fn new(os: &str, arch: &str) -> Target {
        Target { os: os.into(), arch: arch.into() }
    }
    pub fn label(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
}

/// Parse a release tag or version string: `v0.2.0`, `0.2.0-rc.1`, `V1.0.0+build.5`. Build metadata is dropped,
/// because semver says it must not influence precedence (and the `semver` crate would otherwise order by it).
pub fn parse_version(s: &str) -> std::result::Result<semver::Version, String> {
    let t = s.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    let mut v = semver::Version::parse(t).map_err(|e| e.to_string())?;
    v.build = semver::BuildMetadata::EMPTY;
    Ok(v)
}

/// True if `candidate` is strictly newer than `current` (build metadata ignored, pre-releases sort before the release).
pub fn is_newer(candidate: &semver::Version, current: &semver::Version) -> bool {
    candidate > current
}

// ---------------------------------------------------------------------------------------------------------------
// Release data
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

/// A downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetInfo {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// Everything the UI needs to show an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current: String,
    /// Newest version on the channel (no `v`, no build metadata).
    pub latest: String,
    pub tag: String,
    pub update_available: bool,
    pub channel: Channel,
    pub prerelease: bool,
    /// ISO-8601 timestamp from GitHub.
    pub published_at: Option<String>,
    /// Release notes (markdown), truncated to 64 KiB.
    pub notes: String,
    pub html_url: Option<String>,
    /// The installer/archive for this system, if the release has one.
    pub asset: Option<AssetInfo>,
    pub checksums: Option<AssetInfo>,
    pub signature: Option<AssetInfo>,
    /// Why an update cannot be installed automatically (no asset for this system, no checksums, ...).
    pub problems: Vec<String>,
}

impl UpdateInfo {
    /// True if everything needed for a verified install is attached to the release.
    pub fn installable(&self) -> bool {
        self.asset.is_some() && self.checksums.is_some() && self.signature.is_some()
    }

    /// Does the release carry `SHA256SUMS.minisig`?
    pub fn has_signature_asset(&self) -> bool {
        self.signature.is_some()
    }

    /// The release notes as readable plain text (see [`plain_release_notes`]).
    pub fn notes_plain(&self) -> String {
        plain_release_notes(&self.notes)
    }

    /// How this release can be installed with the key embedded in this build.
    pub fn install_mode(&self) -> InstallMode {
        self.install_mode_for(UPDATE_PUBLIC_KEY)
    }

    /// [`UpdateInfo::install_mode`] for an explicit key (tests).
    ///
    /// * a real key: [`InstallMode::Signed`] when asset, `SHA256SUMS` and `SHA256SUMS.minisig` exist, else `Blocked` (never unsigned);
    /// * the placeholder: [`InstallMode::UnsignedWithConsent`] when asset and `SHA256SUMS` exist (the signature, if any, cannot be checked).
    pub fn install_mode_for(&self, public_key: &str) -> InstallMode {
        if self.asset.is_none() {
            return InstallMode::Blocked(
                self.problems.first().cloned().unwrap_or_else(|| "the release has no build for this system".into()),
            );
        }
        if self.checksums.is_none() {
            return InstallMode::Blocked(format!("the release has no {SUMS_ASSET} asset, so the download cannot be checked"));
        }
        if signing_key_is_placeholder(public_key) {
            return InstallMode::UnsignedWithConsent;
        }
        if self.signature.is_none() {
            return InstallMode::Blocked(format!(
                "the release has no {SIGNATURE_ASSET} asset and this build only installs signed releases"
            ));
        }
        InstallMode::Signed
    }
}

/// What can be done with a release. `Blocked` carries the reason in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "reason", rename_all = "snake_case")]
pub enum InstallMode {
    /// Signature of `SHA256SUMS` and the file's SHA-256 will both be checked ([`download_verified`]).
    Signed,
    /// The embedded key is the placeholder: only the explicit, opt-in [`download_unchecked_signature`] is possible (SHA-256 only).
    UnsignedWithConsent,
    Blocked(String),
}

impl InstallMode {
    pub fn is_blocked(&self) -> bool {
        matches!(self, InstallMode::Blocked(_))
    }
    /// Stable word for JSON and logs: `signed`, `unsigned_with_consent`, `blocked`.
    pub fn name(&self) -> &'static str {
        match self {
            InstallMode::Signed => "signed",
            InstallMode::UnsignedWithConsent => "unsigned_with_consent",
            InstallMode::Blocked(_) => "blocked",
        }
    }
}

/// `synofurb-<version>-<os>-<arch>.<ext>`; `version` has no leading `v`.
pub fn asset_name(version: &str, os: &str, arch: &str, ext: &str) -> String {
    format!("synofurb-{version}-{os}-{arch}.{ext}")
}

/// Extension preference for a target, best first. Linux prefers the package format when the running binary lives
/// in a package-managed location (`/usr/bin`, `/opt`), otherwise a self-replacing AppImage or tarball.
pub fn ext_preference(target: &Target, exe: Option<&Path>, running_as_appimage: bool) -> Vec<&'static str> {
    match target.os.as_str() {
        "windows" => vec!["msi", "exe", "zip"],
        "macos" => vec!["dmg", "pkg", "tar.gz", "zip"],
        "linux" => {
            let packaged = !running_as_appimage
                && exe.is_some_and(|p| (p.starts_with("/usr") && !p.starts_with("/usr/local")) || p.starts_with("/opt"));
            if packaged { vec!["deb", "rpm", "AppImage", "tar.gz"] } else { vec!["AppImage", "tar.gz", "deb", "rpm"] }
        }
        _ => vec![],
    }
}

fn is_plain_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Choose the asset for `target` by exact name. An asset whose embedded version differs from the release version is
/// ignored, so a mislabeled upload can never be picked.
pub fn pick_asset<'a>(assets: &'a [AssetInfo], version: &semver::Version, target: &Target, prefs: &[&str]) -> Option<&'a AssetInfo> {
    let v = version.to_string();
    prefs.iter().find_map(|ext| {
        let want = asset_name(&v, &target.os, &target.arch, ext);
        assets.iter().find(|a| a.name == want)
    })
}

/// Pick the release to offer: the highest version among eligible releases (the API order is not trusted).
fn select_release(releases: &[GhRelease], channel: Channel) -> Option<(semver::Version, &GhRelease)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| parse_version(&r.tag_name).ok().map(|v| (v, r)))
        .filter(|(v, r)| channel == Channel::Prerelease || (!r.prerelease && v.pre.is_empty()))
        .max_by(|a, b| a.0.cmp(&b.0))
}

fn truncate_utf8(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n\n[release notes truncated]");
    }
    s
}

fn build_info(
    current: &semver::Version,
    channel: Channel,
    target: &Target,
    prefs: &[&str],
    version: semver::Version,
    rel: &GhRelease,
) -> UpdateInfo {
    let assets: Vec<AssetInfo> = rel
        .assets
        .iter()
        .filter(|a| a.state.as_deref().is_none_or(|s| s == "uploaded"))
        .map(|a| AssetInfo { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size })
        .collect();
    let by_name = |n: &str| assets.iter().find(|a| a.name == n).cloned();
    let asset = pick_asset(&assets, &version, target, prefs).cloned();
    let checksums = by_name(SUMS_ASSET);
    let signature = by_name(SIGNATURE_ASSET);
    let mut problems = Vec::new();
    if asset.is_none() {
        problems.push(format!(
            "release {} has no build for this system (looked for {})",
            rel.tag_name,
            prefs.iter().map(|e| asset_name(&version.to_string(), &target.os, &target.arch, e)).collect::<Vec<_>>().join(", ")
        ));
    }
    if checksums.is_none() {
        problems.push(format!("release {} has no {SUMS_ASSET}", rel.tag_name));
    }
    if signature.is_none() {
        problems.push(format!("release {} has no {SIGNATURE_ASSET}", rel.tag_name));
    }
    UpdateInfo {
        current: current.to_string(),
        latest: version.to_string(),
        tag: rel.tag_name.clone(),
        update_available: is_newer(&version, current),
        channel,
        prerelease: rel.prerelease || !version.pre.is_empty(),
        published_at: rel.published_at.clone(),
        notes: truncate_utf8(rel.body.clone().unwrap_or_default(), MAX_NOTES_BYTES),
        html_url: rel.html_url.clone(),
        asset,
        checksums,
        signature,
        problems,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// URL policy
// ---------------------------------------------------------------------------------------------------------------

/// Which hosts may be contacted. Every request, and every redirect hop, is checked against it.
#[derive(Clone, Copy)]
pub struct HostPolicy {
    allow_host: fn(&str) -> bool,
    /// Plain http to 127.0.0.1/localhost. Only the unit tests turn this on.
    allow_loopback_http: bool,
}

fn api_host(h: &str) -> bool {
    h == "api.github.com"
}
fn download_host(h: &str) -> bool {
    h == "github.com" || h == "githubusercontent.com" || h.ends_with(".githubusercontent.com")
}
#[cfg(test)]
fn any_host(_: &str) -> bool {
    true
}

impl HostPolicy {
    /// The GitHub REST API.
    pub const API: HostPolicy = HostPolicy { allow_host: api_host, allow_loopback_http: false };
    /// Release downloads: `github.com` and `*.githubusercontent.com` (where GitHub redirects release assets).
    pub const DOWNLOAD: HostPolicy = HostPolicy { allow_host: download_host, allow_loopback_http: false };
    #[cfg(test)]
    pub(crate) const LOOPBACK_TEST: HostPolicy = HostPolicy { allow_host: any_host, allow_loopback_http: true };

    /// Validate `url` and return its lower-cased host.
    pub fn check(&self, url: &str) -> Result<String> {
        let (scheme, rest) = url.split_once("://").ok_or_else(|| UpdateError::BadUrl(url.into()))?;
        let scheme = scheme.to_ascii_lowercase();
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty()
            || authority.contains('@')
            || authority.contains('\\')
            || authority.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(UpdateError::BadUrl(url.into()));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.contains(']') || h.ends_with(']') => (h, Some(p)),
            _ => (authority, None),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        if host.is_empty() {
            return Err(UpdateError::BadUrl(url.into()));
        }
        let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
        match scheme.as_str() {
            "https" => {
                if port.is_some_and(|p| p != "443") {
                    return Err(UpdateError::BadUrl(url.into()));
                }
            }
            "http" if self.allow_loopback_http && loopback => return Ok(host),
            _ => return Err(UpdateError::NotHttps(url.into())),
        }
        if host.ends_with('.') || !(self.allow_host)(&host) {
            return Err(UpdateError::UnsupportedHost(host));
        }
        Ok(host)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// HTTP plumbing
// ---------------------------------------------------------------------------------------------------------------

fn user_agent() -> String {
    format!("synofurb/{} (+https://github.com/andreaswiren/synofurb)", env!("CARGO_PKG_VERSION"))
}

fn agent(read_timeout: Duration, total: Option<Duration>) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(read_timeout)
        .redirects(0)
        .user_agent(&user_agent());
    if let Some(t) = total {
        b = b.timeout(t);
    }
    b.build()
}

/// GET with manual redirect handling so every hop is checked against `policy`. Error statuses are returned, not raised.
fn get(agent: &ureq::Agent, policy: HostPolicy, url: &str, headers: &[(&str, &str)]) -> Result<ureq::Response> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        policy.check(&url)?;
        let mut req = agent.get(&url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(UpdateError::Http(format!("request to {url} failed: {e}"))),
        };
        if matches!(resp.status(), 301 | 302 | 303 | 307 | 308) {
            let loc = resp.header("Location").ok_or_else(|| UpdateError::Http(format!("redirect without Location from {url}")))?;
            url = if let Some(path) = loc.strip_prefix('/') {
                // relative redirect: same origin
                let (scheme, rest) = url.split_once("://").ok_or_else(|| UpdateError::BadUrl(url.clone()))?;
                let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
                format!("{scheme}://{authority}/{path}")
            } else {
                loc.to_string()
            };
            continue;
        }
        return Ok(resp);
    }
    Err(UpdateError::TooManyRedirects)
}

fn read_limited(resp: ureq::Response, limit: u64) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    resp.into_reader().take(limit + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(UpdateError::Http(format!("response larger than {limit} bytes")));
    }
    Ok(buf)
}

/// ETag cache for the releases request: in memory, and on disk when a directory is given.
#[derive(Default)]
pub struct ReleaseCache {
    dir: Option<PathBuf>,
    mem: Mutex<HashMap<String, (String, String)>>,
}

impl ReleaseCache {
    pub fn in_memory() -> ReleaseCache {
        ReleaseCache::default()
    }
    pub fn on_disk(dir: &Path) -> ReleaseCache {
        ReleaseCache { dir: Some(dir.to_path_buf()), mem: Mutex::default() }
    }
    fn file(&self, url: &str) -> Option<PathBuf> {
        let h = hex::encode(Sha256::digest(url.as_bytes()));
        self.dir.as_ref().map(|d| d.join(format!("releases-{}.json", &h[..16])))
    }
    fn get(&self, url: &str) -> Option<(String, String)> {
        if let Some(v) = self.mem.lock().ok()?.get(url) {
            return Some(v.clone());
        }
        let text = std::fs::read_to_string(self.file(url)?).ok()?;
        let v: (String, String) = serde_json::from_str(&text).ok()?;
        Some(v)
    }
    fn put(&self, url: &str, etag: &str, body: &str) {
        if let Ok(mut m) = self.mem.lock() {
            m.insert(url.to_string(), (etag.to_string(), body.to_string()));
        }
        if let Some(f) = self.file(url) {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(s) = serde_json::to_string(&(etag, body)) {
                let _ = std::fs::write(f, s);
            }
        }
    }
}

fn global_cache() -> &'static ReleaseCache {
    static C: std::sync::OnceLock<ReleaseCache> = std::sync::OnceLock::new();
    C.get_or_init(ReleaseCache::in_memory)
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok =
        |s: &str| !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if ok(a) && ok(b))
}

fn fetch_releases(api_base: &str, repo: &str, policy: HostPolicy, cache: &ReleaseCache) -> Result<String> {
    let url = format!("{api_base}/repos/{repo}/releases?per_page=30");
    let cached = cache.get(&url);
    let mut headers: Vec<(&str, String)> =
        vec![("Accept", "application/vnd.github+json".into()), ("X-GitHub-Api-Version", "2022-11-28".into())];
    if let Some((etag, _)) = &cached {
        headers.push(("If-None-Match", etag.clone()));
    }
    let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let resp = get(&agent(Duration::from_secs(20), Some(Duration::from_secs(30))), policy, &url, &h)?;
    match resp.status() {
        200 => {
            let etag = resp.header("ETag").map(str::to_string);
            let body =
                String::from_utf8(read_limited(resp, MAX_API_BYTES)?).map_err(|_| UpdateError::Parse("response is not UTF-8".into()))?;
            if let Some(e) = etag {
                cache.put(&url, &e, &body);
            }
            Ok(body)
        }
        304 => cached.map(|(_, b)| b).ok_or_else(|| UpdateError::Http("304 Not Modified without a cached copy".into())),
        404 => Err(UpdateError::NotFound(repo.to_string())),
        code @ (403 | 429) => {
            let remaining = resp.header("x-ratelimit-remaining").map(str::to_string);
            let retry_after = resp.header("retry-after").and_then(|v| v.trim().parse::<u64>().ok());
            let reset_at = resp.header("x-ratelimit-reset").and_then(|v| v.trim().parse::<u64>().ok());
            if code == 429 || remaining.as_deref() == Some("0") || retry_after.is_some() {
                Err(UpdateError::RateLimited { reset_at, retry_after })
            } else {
                Err(UpdateError::Http(format!("GitHub answered HTTP {code} for {url}")))
            }
        }
        code => Err(UpdateError::Http(format!("GitHub answered HTTP {code} for {url}"))),
    }
}

/// Look for a newer release of `repo` (`owner/name`) on `channel`. Uses an in-memory ETag cache, so polling is cheap
/// and a 304 does not count against the rate limit. See [`check_with_cache`] to persist the cache.
pub fn check(current: &str, repo: &str, channel: Channel) -> Result<UpdateInfo> {
    check_with_cache(current, repo, channel, global_cache())
}

/// Like [`check`] with a caller-supplied cache (for example [`ReleaseCache::on_disk`]).
pub fn check_with_cache(current: &str, repo: &str, channel: Channel, cache: &ReleaseCache) -> Result<UpdateInfo> {
    let exe = std::env::current_exe().ok();
    let appimage = std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty());
    let target = Target::current();
    let prefs = ext_preference(&target, exe.as_deref(), appimage);
    check_inner(current, repo, channel, &target, &prefs, API_BASE, HostPolicy::API, cache)
}

#[allow(clippy::too_many_arguments)]
fn check_inner(
    current: &str,
    repo: &str,
    channel: Channel,
    target: &Target,
    prefs: &[&str],
    api_base: &str,
    policy: HostPolicy,
    cache: &ReleaseCache,
) -> Result<UpdateInfo> {
    if !valid_repo(repo) {
        return Err(UpdateError::BadRepo(repo.into()));
    }
    let current_v = parse_version(current).map_err(|e| UpdateError::BadVersion(current.into(), e))?;
    let body = fetch_releases(api_base, repo, policy, cache)?;
    let releases: Vec<GhRelease> = serde_json::from_str(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let (version, rel) = select_release(&releases, channel).ok_or(UpdateError::NoRelease)?;
    Ok(build_info(&current_v, channel, target, prefs, version, rel))
}

// ---------------------------------------------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------------------------------------------

/// True while [`UPDATE_PUBLIC_KEY`] has not been replaced with a real key.
pub fn signing_key_is_placeholder(public_key: &str) -> bool {
    let k = public_key.trim();
    k.is_empty() || k == UPDATE_PUBLIC_KEY_PLACEHOLDER
}

/// Verify the detached minisign signature `sig_text` (contents of `SHA256SUMS.minisig`) over `data`.
/// `public_key` is either the base64 line of a `.pub` file or the whole file.
///
/// Both the modern pre-hashed (`ED`) and the legacy (`Ed`) signature modes are accepted; both are Ed25519 over the
/// data and the key id must match.
pub fn verify_signature(data: &[u8], sig_text: &str, public_key: &str) -> Result<()> {
    if signing_key_is_placeholder(public_key) {
        return Err(UpdateError::NoSigningKey);
    }
    let pk = if public_key.trim().contains('\n') {
        minisign_verify::PublicKey::decode(public_key.trim())
    } else {
        minisign_verify::PublicKey::from_base64(public_key.trim())
    }
    .map_err(|e| UpdateError::BadSigningKey(e.to_string()))?;
    let sig = minisign_verify::Signature::decode(sig_text).map_err(|e| UpdateError::SignatureInvalid(e.to_string()))?;
    pk.verify(data, &sig, true).map_err(|e| UpdateError::SignatureInvalid(e.to_string()))
}

/// Parse a `sha256sum`-style file into `file name -> lower-case hex digest`.
pub fn parse_sha256sums(text: &str) -> Result<HashMap<String, String>> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hash, rest) = line.split_once(char::is_whitespace).ok_or_else(|| UpdateError::BadChecksumFile(format!("line {}", n + 1)))?;
        let name = rest.trim_start().trim_start_matches('*');
        let name = name.strip_prefix("./").unwrap_or(name);
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(UpdateError::BadChecksumFile(format!("line {}: not a SHA-256 digest", n + 1)));
        }
        if !is_plain_filename(name) {
            return Err(UpdateError::BadChecksumFile(format!("line {}: bad file name {name:?}", n + 1)));
        }
        let hash = hash.to_ascii_lowercase();
        if let Some(prev) = out.insert(name.to_string(), hash.clone())
            && prev != hash
        {
            return Err(UpdateError::BadChecksumFile(format!("{name} is listed twice with different digests")));
        }
    }
    Ok(out)
}

/// Progress of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// How a download was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    /// The minisign signature of `SHA256SUMS` verified and the file's SHA-256 matched.
    Signed,
    /// Only the file's SHA-256 against the `SHA256SUMS` of the same release (no signature): protects against a damaged download, **not**
    /// against a tampered release. Label it as such wherever it is shown.
    ChecksumOnly,
}

/// A downloaded file that passed its checks (see `integrity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDownload {
    pub path: PathBuf,
    pub sha256: String,
    pub asset: AssetInfo,
    pub version: String,
    /// True only when the signature was verified (`integrity == Signed`).
    pub signed: bool,
    pub integrity: Integrity,
}

/// Proof that a person agreed to install **without a signature**. It can only be made with [`UnsignedConsent::acknowledged`], which a
/// front end calls after showing the warning and getting an explicit yes; it is consumed by [`download_unchecked_signature`].
#[derive(Debug)]
pub struct UnsignedConsent(());

impl UnsignedConsent {
    /// Call this only after the person has been told that only the SHA-256 listed in `SHA256SUMS` of the same GitHub release is checked,
    /// and that a tampered release would not be detected.
    pub fn acknowledged() -> UnsignedConsent {
        UnsignedConsent(())
    }
}

/// The warning to show before asking for consent (CLI and GUI use the same words).
pub const UNSIGNED_WARNING: &str = "This release is not signed, so it cannot be verified. Only the SHA-256 listed in SHA256SUMS of the same GitHub release is checked: it catches a damaged download, but a tampered release (or a hijacked GitHub account) would not be detected. Install it only if you trust the release page.";

/// May the unsigned path be used? True exactly while [`UPDATE_PUBLIC_KEY`] is the placeholder. When a real key is embedded this is
/// false and [`download_unchecked_signature`] refuses, so signatures become mandatory without any other change.
pub fn unsigned_allowed() -> bool {
    signing_key_is_placeholder(UPDATE_PUBLIC_KEY)
}

/// Refuse early (before any network traffic) when updates cannot be verified.
pub fn ensure_can_verify(public_key: &str) -> Result<()> {
    if signing_key_is_placeholder(public_key) { Err(UpdateError::NoSigningKey) } else { Ok(()) }
}

/// Download the asset of `info` into `dest_dir`, verified against [`UPDATE_PUBLIC_KEY`]. See the module docs.
pub fn download_verified(info: &UpdateInfo, dest_dir: &Path, progress: &mut dyn FnMut(Progress)) -> Result<VerifiedDownload> {
    download_verified_with(info, dest_dir, UPDATE_PUBLIC_KEY, HostPolicy::DOWNLOAD, progress)
}

/// [`download_verified`] with an explicit key and host policy (tests inject their own).
pub fn download_verified_with(
    info: &UpdateInfo,
    dest_dir: &Path,
    public_key: &str,
    policy: HostPolicy,
    progress: &mut dyn FnMut(Progress),
) -> Result<VerifiedDownload> {
    ensure_can_verify(public_key)?;
    let asset = info
        .asset
        .as_ref()
        .ok_or_else(|| UpdateError::NoAssetForTarget(info.problems.first().cloned().unwrap_or_else(|| "unknown".into())))?;
    let sums = info.checksums.as_ref().ok_or(UpdateError::MissingAsset(SUMS_ASSET))?;
    let sig = info.signature.as_ref().ok_or(UpdateError::MissingAsset(SIGNATURE_ASSET))?;
    for a in [asset, sums, sig] {
        if !is_plain_filename(&a.name) {
            return Err(UpdateError::BadAssetName(a.name.clone()));
        }
        policy.check(&a.url)?;
    }

    // 1. signed checksum file
    let ag = agent(Duration::from_secs(30), Some(Duration::from_secs(60)));
    let fetch_small = |a: &AssetInfo, limit: u64| -> Result<Vec<u8>> {
        let r = get(&ag, policy, &a.url, &[("Accept", "application/octet-stream")])?;
        if r.status() != 200 {
            return Err(UpdateError::Http(format!("{} answered HTTP {}", a.name, r.status())));
        }
        read_limited(r, limit)
    };
    let sums_bytes = fetch_small(sums, MAX_SUMS_BYTES)?;
    let sig_bytes = fetch_small(sig, MAX_SIG_BYTES)?;
    let sig_text = String::from_utf8(sig_bytes).map_err(|_| UpdateError::SignatureInvalid("signature file is not text".into()))?;
    verify_signature(&sums_bytes, &sig_text, public_key)?;

    // 2. the digest promised for this file
    let sums_text = String::from_utf8(sums_bytes).map_err(|_| UpdateError::BadChecksumFile("not UTF-8".into()))?;
    let table = parse_sha256sums(&sums_text)?;
    let expected = table.get(&asset.name).ok_or_else(|| UpdateError::ChecksumMissing(asset.name.clone()))?.clone();

    // 3. stream the asset while hashing
    let (final_path, actual) = stream_asset(asset, &expected, dest_dir, policy, progress)?;
    Ok(VerifiedDownload {
        path: final_path,
        sha256: actual,
        asset: asset.clone(),
        version: info.latest.clone(),
        signed: true,
        integrity: Integrity::Signed,
    })
}

/// Download the asset of `info` **without checking a signature**: only its SHA-256 against the `SHA256SUMS` of the same release (which
/// must exist; `SHA256SUMS.minisig` need not). Needs the person's [`UnsignedConsent`] and is refused with
/// [`UpdateError::UnsignedNotAllowed`] while a real signing key is embedded (see the module docs). Same host policy, size limits and
/// streaming hash as [`download_verified`]; a partial or mismatching file is deleted; nothing is installed here.
pub fn download_unchecked_signature(
    info: &UpdateInfo,
    dest_dir: &Path,
    consent: UnsignedConsent,
    progress: &mut dyn FnMut(Progress),
) -> Result<VerifiedDownload> {
    download_unchecked_signature_with(info, dest_dir, UPDATE_PUBLIC_KEY, consent, HostPolicy::DOWNLOAD, progress)
}

/// [`download_unchecked_signature`] with an explicit key and host policy (tests inject their own).
pub fn download_unchecked_signature_with(
    info: &UpdateInfo,
    dest_dir: &Path,
    public_key: &str,
    consent: UnsignedConsent,
    policy: HostPolicy,
    progress: &mut dyn FnMut(Progress),
) -> Result<VerifiedDownload> {
    let UnsignedConsent(()) = consent;
    if !signing_key_is_placeholder(public_key) {
        return Err(UpdateError::UnsignedNotAllowed);
    }
    let asset = info
        .asset
        .as_ref()
        .ok_or_else(|| UpdateError::NoAssetForTarget(info.problems.first().cloned().unwrap_or_else(|| "unknown".into())))?;
    let sums = info.checksums.as_ref().ok_or(UpdateError::MissingAsset(SUMS_ASSET))?;
    for a in [asset, sums] {
        if !is_plain_filename(&a.name) {
            return Err(UpdateError::BadAssetName(a.name.clone()));
        }
        policy.check(&a.url)?;
    }
    let ag = agent(Duration::from_secs(30), Some(Duration::from_secs(60)));
    let r = get(&ag, policy, &sums.url, &[("Accept", "application/octet-stream")])?;
    if r.status() != 200 {
        return Err(UpdateError::Http(format!("{} answered HTTP {}", sums.name, r.status())));
    }
    let sums_text = String::from_utf8(read_limited(r, MAX_SUMS_BYTES)?).map_err(|_| UpdateError::BadChecksumFile("not UTF-8".into()))?;
    let table = parse_sha256sums(&sums_text)?;
    let expected = table.get(&asset.name).ok_or_else(|| UpdateError::ChecksumMissing(asset.name.clone()))?.clone();
    let (final_path, actual) = stream_asset(asset, &expected, dest_dir, policy, progress)?;
    Ok(VerifiedDownload {
        path: final_path,
        sha256: actual,
        asset: asset.clone(),
        version: info.latest.clone(),
        signed: false,
        integrity: Integrity::ChecksumOnly,
    })
}

/// Stream `asset` into `<dest_dir>/<name>.part` while hashing, enforce the size limits, compare with `expected` and only then move it to its
/// final name. Any failure removes the partial file. Returns the final path and the digest.
fn stream_asset(
    asset: &AssetInfo,
    expected: &str,
    dest_dir: &Path,
    policy: HostPolicy,
    progress: &mut dyn FnMut(Progress),
) -> Result<(PathBuf, String)> {
    std::fs::create_dir_all(dest_dir)?;
    let final_path = dest_dir.join(&asset.name);
    let part_path = dest_dir.join(format!("{}.part", asset.name));
    let _ = std::fs::remove_file(&part_path);
    let dl_agent = agent(Duration::from_secs(60), None);
    let resp = get(&dl_agent, policy, &asset.url, &[("Accept", "application/octet-stream")])?;
    if resp.status() != 200 {
        return Err(UpdateError::Http(format!("{} answered HTTP {}", asset.name, resp.status())));
    }
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).or((asset.size > 0).then_some(asset.size));
    let limit = if asset.size > 0 { asset.size } else { MAX_ASSET_BYTES };
    let result = (|| -> Result<(u64, String)> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&part_path)?;
        let mut reader = resp.into_reader();
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done: u64 = 0;
        progress(Progress { downloaded: 0, total });
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > limit {
                return Err(UpdateError::TooLarge { name: asset.name.clone(), limit });
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            progress(Progress { downloaded: done, total });
        }
        file.sync_all()?;
        Ok((done, hex::encode(hasher.finalize())))
    })();
    let (done, actual) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_file(&part_path);
            return Err(e);
        }
    };
    if asset.size > 0 && done != asset.size {
        let _ = std::fs::remove_file(&part_path);
        return Err(UpdateError::SizeMismatch { name: asset.name.clone(), got: done, want: asset.size });
    }
    if actual != expected {
        let _ = std::fs::remove_file(&part_path);
        return Err(UpdateError::ChecksumMismatch { name: asset.name.clone(), expected: expected.to_string(), actual });
    }
    let _ = std::fs::remove_file(&final_path);
    std::fs::rename(&part_path, &final_path)?;
    Ok((final_path, actual))
}

// ---------------------------------------------------------------------------------------------------------------
// Release notes as plain text
// ---------------------------------------------------------------------------------------------------------------

/// Turn GitHub-flavoured Markdown release notes into readable plain text: headings lose their `#`, `**bold**`, `*italic*`
/// and `` `code` `` lose their marks, `- item` / `* item` become `• item` (nesting kept as indentation), `[text](url)` becomes
/// `text (url)`, images become their alt text, block quotes and code fences lose their markers, rules and HTML tags are dropped,
/// and runs of blank lines collapse. It is meant for display only and never executes or fetches anything.
pub fn plain_release_notes(md: &str) -> String {
    let mut out: Vec<String> = vec![];
    let mut in_fence = false;
    for raw in md.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push(format!("    {line}"));
            continue;
        }
        if t.is_empty() {
            out.push(String::new());
            continue;
        }
        // horizontal rules
        let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() >= 3
            && (compact.chars().all(|c| c == '-') || compact.chars().all(|c| c == '*') || compact.chars().all(|c| c == '_'))
        {
            continue;
        }
        let indent = line.len() - t.len();
        // headings
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            out.push(plain_inline(t[hashes..].trim().trim_end_matches('#').trim()));
            continue;
        }
        // block quotes
        let (t, quoted) = match t.strip_prefix('>') {
            Some(r) => (r.trim_start(), true),
            None => (t, false),
        };
        // bullets
        let bullet = ["- ", "* ", "+ "].iter().find_map(|m| t.strip_prefix(m));
        let text = if let Some(rest) = bullet {
            let level = indent / 2;
            format!("{}\u{2022} {}", "  ".repeat(level), plain_inline(rest.trim()))
        } else {
            let body = plain_inline(t);
            if quoted { format!("  {body}") } else { body }
        };
        out.push(text);
    }
    // collapse blank runs, trim the ends
    let mut res = String::new();
    let mut blanks = 0;
    for l in out {
        if l.trim().is_empty() {
            blanks += 1;
            continue;
        }
        if !res.is_empty() {
            res.push('\n');
            if blanks > 0 {
                res.push('\n');
            }
        }
        blanks = 0;
        res.push_str(&l);
    }
    res
}

/// Inline Markdown to text: links, images, code spans, bold, italic, autolinks, HTML tags.
fn plain_inline(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut o = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if i + 1 < c.len() && c[i + 1].is_ascii_punctuation() => {
                o.push(c[i + 1]);
                i += 2;
            }
            '`' => {
                let n = c[i..].iter().take_while(|x| **x == '`').count();
                let fence: String = "`".repeat(n);
                let rest: String = c[i + n..].iter().collect();
                match rest.find(&fence) {
                    Some(end) => {
                        o.push_str(rest[..end].trim());
                        i += n + rest[..end].chars().count() + n;
                    }
                    None => {
                        o.push_str(&fence);
                        i += n;
                    }
                }
            }
            '!' | '[' => {
                let img = c[i] == '!' && c.get(i + 1) == Some(&'[');
                let open = if img { i + 1 } else { i };
                if c[open] == '['
                    && let Some((text, url, next)) = md_link(&c, open)
                {
                    let text = plain_inline(&text);
                    if img {
                        o.push_str(&text);
                    } else if text.is_empty() || text == url {
                        o.push_str(&url);
                    } else {
                        o.push_str(&format!("{text} ({url})"));
                    }
                    i = next;
                } else {
                    o.push(c[i]);
                    i += 1;
                }
            }
            '*' | '_' => {
                let n = c[i..].iter().take_while(|x| **x == c[i]).count();
                let prev = if i > 0 { c[i - 1] } else { ' ' };
                let next = c.get(i + n).copied().unwrap_or(' ');
                // `**` and `__` are marks; a single `*` is a mark unless it stands alone between spaces; a single `_` is a mark
                // unless it is inside a word (snake_case)
                let mark = n >= 2
                    || (c[i] == '*' && !(prev.is_whitespace() && next.is_whitespace()))
                    || (c[i] == '_' && !(prev.is_alphanumeric() && next.is_alphanumeric()));
                if !mark {
                    o.extend(std::iter::repeat_n(c[i], n));
                }
                i += n;
            }
            '<' => match c[i..].iter().position(|x| *x == '>') {
                Some(end) if end > 1 && c[i + 1].is_ascii_alphabetic() || c.get(i + 1) == Some(&'/') => {
                    let end = c[i..].iter().position(|x| *x == '>').unwrap_or(0);
                    let inner: String = c[i + 1..i + end].iter().collect();
                    if inner.starts_with("http://") || inner.starts_with("https://") {
                        o.push_str(&inner);
                    } else if inner.trim_end_matches(['/', ' ']).eq_ignore_ascii_case("br") {
                        o.push(' ');
                    }
                    i += end + 1;
                }
                _ => {
                    o.push('<');
                    i += 1;
                }
            },
            ch => {
                o.push(ch);
                i += 1;
            }
        }
    }
    o.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `[text](url "title")` starting at `c[open] == '['`: (text, url, index after the closing paren).
fn md_link(c: &[char], open: usize) -> Option<(String, String, usize)> {
    let mut depth = 0;
    let mut close = None;
    for (k, ch) in c.iter().enumerate().skip(open) {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(k);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = c[close + 2..].iter().position(|x| *x == ')')? + close + 2;
    let inside: String = c[close + 2..end].iter().collect();
    let url = inside.split_whitespace().next().unwrap_or("").trim_matches(['<', '>']).to_string();
    Some((c[open + 1..close].iter().collect(), url, end + 1))
}

// ---------------------------------------------------------------------------------------------------------------
// Applying an update
// ---------------------------------------------------------------------------------------------------------------

/// What [`apply`] is going to do with a downloaded file. Pure, so it can be tested on any host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyPlan {
    /// Start an installer and exit this process (Windows).
    LaunchInstaller { program: String, args: Vec<String> },
    /// Ask the OS to open the file (macOS disk image / pkg).
    OpenFile { program: String, args: Vec<String> },
    /// Replace the running executable with the file (AppImage) or the binary inside it (tar.gz).
    ReplaceExecutable { source: PathBuf, from_tar_gz: bool },
    /// A distribution package. `command` is always shown; it is run through `pkexec` only if that exists.
    SystemPackage { command: Vec<String>, use_pkexec: bool },
    /// Nothing safe to automate.
    Manual { reason: String },
}

pub fn plan_apply(file: &Path, os: &str, pkexec_available: bool) -> ApplyPlan {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let path = file.to_string_lossy().into_owned();
    let ext = |e: &str| name.ends_with(&format!(".{e}"));
    match os {
        "windows" if ext("msi") => {
            ApplyPlan::LaunchInstaller { program: "msiexec".into(), args: vec!["/i".into(), path, "/passive".into()] }
        }
        "windows" if ext("exe") => ApplyPlan::LaunchInstaller { program: path, args: vec![] },
        "macos" if ext("dmg") || ext("pkg") => ApplyPlan::OpenFile { program: "open".into(), args: vec![path] },
        "linux" if ext("AppImage") => ApplyPlan::ReplaceExecutable { source: file.to_path_buf(), from_tar_gz: false },
        "linux" if ext("tar.gz") => ApplyPlan::ReplaceExecutable { source: file.to_path_buf(), from_tar_gz: true },
        "linux" if ext("deb") => {
            ApplyPlan::SystemPackage { command: vec!["apt-get".into(), "install".into(), "-y".into(), path], use_pkexec: pkexec_available }
        }
        "linux" if ext("rpm") => ApplyPlan::SystemPackage { command: vec!["rpm".into(), "-U".into(), path], use_pkexec: pkexec_available },
        _ => ApplyPlan::Manual { reason: format!("{name} cannot be installed automatically on {os}; unpack or open it yourself") },
    }
}

/// What happened in [`apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// An installer was started in the background. The caller should exit so the installer can replace the app.
    InstallerLaunched,
    /// The file was opened in the OS (macOS); the user finishes the install.
    Opened,
    /// The executable was replaced. Restart the app. `backup` is the previous version, kept for rollback.
    Replaced {
        target: PathBuf,
        backup: PathBuf,
    },
    /// The package command ran successfully via pkexec.
    PackageInstalled {
        command: String,
    },
    /// The package command was not run (no pkexec): run it yourself.
    PackageCommand {
        command: String,
    },
    Manual {
        reason: String,
    },
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file())
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '+' | ':' | '=')) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn detached(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        cmd.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    cmd
}

/// The file to replace when self-updating on Linux: `$APPIMAGE` when running from an AppImage, else the executable.
pub fn running_executable() -> Result<PathBuf> {
    if let Some(a) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(a);
        if p.is_file() {
            return Ok(p);
        }
    }
    Ok(std::env::current_exe()?)
}

/// Install a verified download for the current OS. Takes only [`VerifiedDownload::path`]: never pass an unverified file.
pub fn apply(downloaded: &Path) -> Result<ApplyOutcome> {
    let pkexec = find_in_path("pkexec");
    let plan = plan_apply(downloaded, std::env::consts::OS, pkexec.is_some());
    match plan {
        ApplyPlan::LaunchInstaller { program, args } => {
            let mut c = std::process::Command::new(&program);
            c.args(&args);
            detached(&mut c).spawn().map_err(|e| UpdateError::Apply(format!("could not start {program}: {e}")))?;
            Ok(ApplyOutcome::InstallerLaunched)
        }
        ApplyPlan::OpenFile { program, args } => {
            let st = std::process::Command::new(&program)
                .args(&args)
                .status()
                .map_err(|e| UpdateError::Apply(format!("could not run {program}: {e}")))?;
            if st.success() { Ok(ApplyOutcome::Opened) } else { Err(UpdateError::Apply(format!("{program} exited with {st}"))) }
        }
        ApplyPlan::ReplaceExecutable { source, from_tar_gz } => {
            let target = running_executable()?;
            let backup = replace_executable(&source, &target, from_tar_gz)?;
            Ok(ApplyOutcome::Replaced { target, backup })
        }
        ApplyPlan::SystemPackage { command, use_pkexec } => {
            let shown = format!(
                "{}{}",
                if use_pkexec { "pkexec " } else { "sudo " },
                command.iter().map(|s| shell_quote(s)).collect::<Vec<_>>().join(" ")
            );
            match (use_pkexec, pkexec) {
                (true, Some(px)) => {
                    let st = std::process::Command::new(px)
                        .args(&command)
                        .status()
                        .map_err(|e| UpdateError::Apply(format!("could not run pkexec: {e}")))?;
                    if st.success() {
                        Ok(ApplyOutcome::PackageInstalled { command: shown })
                    } else {
                        Err(UpdateError::Apply(format!("`{shown}` exited with {st}")))
                    }
                }
                _ => Ok(ApplyOutcome::PackageCommand { command: shown }),
            }
        }
        ApplyPlan::Manual { reason } => Ok(ApplyOutcome::Manual { reason }),
    }
}

/// `<exe>.old`: where the previous version is kept.
pub fn backup_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

fn extract_from_tar_gz(archive: &Path, bin_name: &str, out: &Path) -> Result<()> {
    let f = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    for entry in ar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let matches = entry.path()?.file_name().is_some_and(|n| n == bin_name);
        if matches {
            if entry.size() > MAX_ASSET_BYTES {
                return Err(UpdateError::Apply("binary in archive is too large".into()));
            }
            let mut dst = std::fs::File::create(out)?;
            std::io::copy(&mut entry, &mut dst)?;
            dst.sync_all()?;
            return Ok(());
        }
    }
    Err(UpdateError::Apply(format!("{bin_name} not found in {}", archive.display())))
}

/// Replace `exe` with `source` (or with the binary inside the tar.gz `source`) without ever leaving `exe` missing:
/// the new file is written next to it, the old one is kept as `<exe>.old` (hard link, else copy), and the new file
/// is renamed over `exe`. Returns the backup path.
pub fn replace_executable(source: &Path, exe: &Path, from_tar_gz: bool) -> Result<PathBuf> {
    let dir = exe.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let fname = exe.file_name().ok_or_else(|| UpdateError::Apply("executable has no file name".into()))?.to_string_lossy().into_owned();
    let tmp = dir.join(format!(".{fname}.new{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let prepared = (|| -> Result<()> {
        if from_tar_gz {
            let bin = if cfg!(windows) { "synofurb.exe" } else { "synofurb" };
            extract_from_tar_gz(source, bin, &tmp)?;
        } else {
            std::fs::copy(source, &tmp)?;
        }
        if std::fs::metadata(&tmp)?.len() == 0 {
            return Err(UpdateError::Apply("new executable is empty".into()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::File::open(&tmp)?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = prepared {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let backup = backup_path(exe);
    let _ = std::fs::remove_file(&backup);
    if std::fs::hard_link(exe, &backup).is_err()
        && let Err(e) = std::fs::copy(exe, &backup)
    {
        let _ = std::fs::remove_file(&tmp);
        return Err(UpdateError::Apply(format!("could not keep a backup of the current version: {e}")));
    }
    if let Err(e) = std::fs::rename(&tmp, exe) {
        let _ = std::fs::remove_file(&tmp);
        return Err(UpdateError::Apply(format!("could not replace {}: {e}", exe.display())));
    }
    Ok(backup)
}

/// Put the previous version back (`<exe>.old` over `exe`).
pub fn rollback(exe: &Path) -> Result<()> {
    let old = backup_path(exe);
    if !old.is_file() {
        return Err(UpdateError::Apply(format!("no backup at {}", old.display())));
    }
    std::fs::rename(&old, exe)?;
    Ok(())
}

/// Call once the updated app has started successfully: it removes `<exe>.old`. Until then the previous binary stays
/// available for [`rollback`]. Returns true if a backup was removed.
pub fn mark_start_ok(exe: &Path) -> bool {
    std::fs::remove_file(backup_path(exe)).is_ok()
}

// ---------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::test_http::{Reply, TempDir, serve};
    use std::sync::{Arc, Mutex};

    // --- fixtures --------------------------------------------------------------------------------------------

    fn assets_json(version: &str) -> String {
        let mut names: Vec<String> = vec![];
        for (os, arch, exts) in [
            ("windows", "x86_64", vec!["msi", "exe", "zip"]),
            ("windows", "aarch64", vec!["msi", "zip"]),
            ("macos", "x86_64", vec!["dmg", "pkg", "tar.gz"]),
            ("macos", "aarch64", vec!["dmg", "pkg", "tar.gz"]),
            ("linux", "x86_64", vec!["AppImage", "tar.gz", "deb", "rpm"]),
            ("linux", "aarch64", vec!["AppImage", "tar.gz", "deb", "rpm"]),
        ] {
            for e in exts {
                names.push(asset_name(version, os, arch, e));
            }
        }
        names.push(SUMS_ASSET.into());
        names.push(SIGNATURE_ASSET.into());
        names
            .iter()
            .map(|n| {
                format!(r#"{{"name":"{n}","size":1234,"state":"uploaded","browser_download_url":"https://github.com/andreaswiren/synofurb/releases/download/v{version}/{n}"}}"#)
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn release_json(tag: &str, prerelease: bool, draft: bool) -> String {
        let v = tag.trim_start_matches('v');
        format!(
            r#"{{"tag_name":"{tag}","name":"{tag}","draft":{draft},"prerelease":{prerelease},"published_at":"2026-10-30T12:00:00Z","html_url":"https://github.com/andreaswiren/synofurb/releases/tag/{tag}","body":"Changes\n- things","assets":[{}]}}"#,
            assets_json(v)
        )
    }

    fn list(rels: &[(&str, bool, bool)]) -> String {
        format!("[{}]", rels.iter().map(|(t, p, d)| release_json(t, *p, *d)).collect::<Vec<_>>().join(","))
    }

    fn check_local(base: &str, current: &str, channel: Channel, target: &Target) -> Result<UpdateInfo> {
        let prefs = ext_preference(target, None, false);
        check_inner(current, DEFAULT_REPO, channel, target, &prefs, base, HostPolicy::LOOPBACK_TEST, &ReleaseCache::in_memory())
    }

    fn parse_releases(json: &str) -> Vec<GhRelease> {
        serde_json::from_str(json).unwrap()
    }

    // --- versions --------------------------------------------------------------------------------------------

    #[test]
    fn version_parsing_and_ordering() {
        let v = |s: &str| parse_version(s).unwrap();
        assert_eq!(v("v0.2.0"), v("0.2.0"));
        assert_eq!(v("V0.2.0"), v("0.2.0"));
        assert!(is_newer(&v("v0.2.0"), &v("0.1.9")));
        assert!(is_newer(&v("0.10.0"), &v("0.9.0")), "numeric, not lexical");
        assert!(!is_newer(&v("0.1.0"), &v("0.1.0")));
        // pre-releases sort before the release and by identifier
        assert!(!is_newer(&v("0.2.0-rc.1"), &v("0.2.0")));
        assert!(is_newer(&v("0.2.0"), &v("0.2.0-rc.1")));
        assert!(is_newer(&v("0.2.0-rc.2"), &v("0.2.0-rc.1")));
        assert!(is_newer(&v("0.2.0-rc.10"), &v("0.2.0-rc.9")));
        assert!(is_newer(&v("0.2.0-rc.1"), &v("0.2.0-beta.5")));
        assert!(is_newer(&v("0.2.0-rc.1"), &v("0.1.0")));
        // build metadata never matters
        assert!(!is_newer(&v("0.2.0+build.9"), &v("0.2.0+build.1")));
        assert!(!is_newer(&v("0.2.0+build.1"), &v("0.2.0+build.9")));
        assert_eq!(v("1.0.0+abc").to_string(), "1.0.0");
        // junk
        for bad in ["", "v", "latest", "1.0", "1.0.0.0", "release-1", "vv1.0.0", "01.0.0"] {
            assert!(parse_version(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn channel_parsing() {
        assert_eq!("stable".parse::<Channel>().unwrap(), Channel::Stable);
        assert_eq!("Beta".parse::<Channel>().unwrap(), Channel::Prerelease);
        assert!("nightly".parse::<Channel>().is_err());
    }

    // --- release selection -----------------------------------------------------------------------------------

    #[test]
    fn stable_skips_prereleases_drafts_and_junk_tags_and_ignores_order() {
        let rels = parse_releases(&list(&[
            ("v0.3.0-rc.1", true, false),
            ("v0.2.0", false, false),
            ("v0.2.1", false, true),       // draft
            ("v0.2.5-rc.1", false, false), // not flagged on GitHub but the version says pre-release
            ("nightly", false, false),     // not a version
            ("v0.1.0", false, false),
        ]));
        let (v, r) = select_release(&rels, Channel::Stable).unwrap();
        assert_eq!((v.to_string().as_str(), r.tag_name.as_str()), ("0.2.0", "v0.2.0"));
        let (v, _) = select_release(&rels, Channel::Prerelease).unwrap();
        assert_eq!(v.to_string(), "0.3.0-rc.1");
        assert!(select_release(&parse_releases("[]"), Channel::Stable).is_none());
        let only_pre = parse_releases(&list(&[("v1.0.0-beta.1", true, false)]));
        assert!(select_release(&only_pre, Channel::Stable).is_none());
    }

    #[test]
    fn prerelease_channel_prefers_a_newer_stable_over_an_older_rc() {
        let rels = parse_releases(&list(&[("v0.2.0-rc.1", true, false), ("v0.2.0", false, false)]));
        assert_eq!(select_release(&rels, Channel::Prerelease).unwrap().0.to_string(), "0.2.0");
    }

    // --- assets ----------------------------------------------------------------------------------------------

    #[test]
    fn asset_selection_for_every_platform() {
        let rels = parse_releases(&list(&[("v0.2.0", false, false)]));
        let (v, rel) = select_release(&rels, Channel::Stable).unwrap();
        let cur = parse_version("0.1.0").unwrap();
        let cases: &[(&str, &str, Option<&str>, bool, &str)] = &[
            ("windows", "x86_64", None, false, "synofurb-0.2.0-windows-x86_64.msi"),
            ("windows", "aarch64", None, false, "synofurb-0.2.0-windows-aarch64.msi"),
            ("macos", "x86_64", None, false, "synofurb-0.2.0-macos-x86_64.dmg"),
            ("macos", "aarch64", None, false, "synofurb-0.2.0-macos-aarch64.dmg"),
            ("linux", "x86_64", Some("/home/u/bin/synofurb"), false, "synofurb-0.2.0-linux-x86_64.AppImage"),
            ("linux", "aarch64", None, false, "synofurb-0.2.0-linux-aarch64.AppImage"),
            ("linux", "x86_64", Some("/usr/bin/synofurb"), false, "synofurb-0.2.0-linux-x86_64.deb"),
            ("linux", "x86_64", Some("/opt/synofurb/synofurb"), false, "synofurb-0.2.0-linux-x86_64.deb"),
            ("linux", "x86_64", Some("/usr/local/bin/synofurb"), false, "synofurb-0.2.0-linux-x86_64.AppImage"),
            ("linux", "x86_64", Some("/usr/bin/synofurb"), true, "synofurb-0.2.0-linux-x86_64.AppImage"),
        ];
        for (os, arch, exe, appimage, want) in cases {
            let t = Target::new(os, arch);
            let prefs = ext_preference(&t, exe.map(Path::new), *appimage);
            let info = build_info(&cur, Channel::Stable, &t, &prefs, v.clone(), rel);
            assert_eq!(info.asset.as_ref().map(|a| a.name.as_str()), Some(*want), "{os}-{arch} {exe:?}");
            assert!(info.installable() && info.problems.is_empty(), "{:?}", info.problems);
            assert!(info.update_available);
            assert_eq!(info.latest, "0.2.0");
            assert_eq!(info.published_at.as_deref(), Some("2026-10-30T12:00:00Z"));
            assert!(info.notes.contains("things"));
            assert_eq!(info.asset.as_ref().unwrap().size, 1234);
        }
    }

    #[test]
    fn missing_or_mislabeled_assets_are_reported() {
        let cur = parse_version("0.1.0").unwrap();
        // no riscv build
        let rels = parse_releases(&list(&[("v0.2.0", false, false)]));
        let (v, rel) = select_release(&rels, Channel::Stable).unwrap();
        let t = Target::new("linux", "riscv64");
        let info = build_info(&cur, Channel::Stable, &t, &ext_preference(&t, None, false), v.clone(), rel);
        assert!(info.asset.is_none() && !info.installable());
        assert!(info.problems[0].contains("no build for this system"));

        // an asset named for another version must never be chosen
        let json = list(&[("v0.2.0", false, false)]).replace("synofurb-0.2.0-windows-x86_64.msi", "synofurb-0.1.0-windows-x86_64.msi");
        let rels = parse_releases(&json);
        let (v, rel) = select_release(&rels, Channel::Stable).unwrap();
        let t = Target::new("windows", "x86_64");
        let info = build_info(&cur, Channel::Stable, &t, &ext_preference(&t, None, false), v.clone(), rel);
        assert_eq!(
            info.asset.as_ref().map(|a| a.name.as_str()),
            Some("synofurb-0.2.0-windows-x86_64.exe"),
            "falls through to the next extension"
        );

        // no checksum / signature assets
        let json = list(&[("v0.2.0", false, false)]).replace("\"SHA256SUMS.minisig\"", "\"other.txt\"");
        let rels = parse_releases(&json);
        let (v, rel) = select_release(&rels, Channel::Stable).unwrap();
        let info = build_info(&cur, Channel::Stable, &t, &ext_preference(&t, None, false), v.clone(), rel);
        assert!(info.signature.is_none() && !info.installable());
        assert!(info.problems.iter().any(|p| p.contains("SHA256SUMS.minisig")));
    }

    #[test]
    fn up_to_date_and_ahead_of_release() {
        let rels = parse_releases(&list(&[("v0.2.0", false, false)]));
        let (v, rel) = select_release(&rels, Channel::Stable).unwrap();
        let t = Target::new("linux", "x86_64");
        for (cur, avail) in [("0.2.0", false), ("v0.2.0+local", false), ("0.3.0-dev", false), ("0.1.9", true), ("0.2.0-rc.1", true)] {
            let c = parse_version(cur).unwrap();
            assert_eq!(build_info(&c, Channel::Stable, &t, &["AppImage"], v.clone(), rel).update_available, avail, "{cur}");
        }
    }

    // --- network layer against a local server ----------------------------------------------------------------

    #[test]
    fn check_uses_etag_and_reports_the_release() {
        type Seen = Vec<(String, Option<String>, Option<String>)>;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let s2 = seen.clone();
        let body = list(&[("v0.1.0", false, false), ("v0.2.0", false, false)]);
        let srv = serve(move |path, h| {
            s2.lock().unwrap().push((path.to_string(), h.get("if-none-match").cloned(), h.get("user-agent").cloned()));
            if h.get("if-none-match").map(String::as_str) == Some("\"abc\"") {
                Reply::status(304)
            } else {
                Reply::ok(body.clone()).header("ETag", "\"abc\"")
            }
        });
        let cache = ReleaseCache::in_memory();
        let t = Target::new("windows", "x86_64");
        let prefs = ext_preference(&t, None, false);
        for _ in 0..2 {
            let info =
                check_inner("0.1.0", DEFAULT_REPO, Channel::Stable, &t, &prefs, &srv.base, HostPolicy::LOOPBACK_TEST, &cache).unwrap();
            assert_eq!(info.latest, "0.2.0");
            assert!(info.update_available);
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, "/repos/andreaswiren/synofurb/releases?per_page=30");
        assert_eq!(seen[0].1, None);
        assert_eq!(seen[1].1.as_deref(), Some("\"abc\""), "second request is conditional");
        assert!(seen[0].2.as_deref().is_some_and(|u| u.starts_with("synofurb/")), "User-Agent is set: {:?}", seen[0].2);
    }

    #[test]
    fn disk_cache_survives_a_new_process() {
        let tmp = TempDir::new("upd-cache");
        let body = list(&[("v0.2.0", false, false)]);
        let srv =
            serve(
                move |_, h| {
                    if h.contains_key("if-none-match") { Reply::status(304) } else { Reply::ok(body.clone()).header("ETag", "W/\"x\"") }
                },
            );
        let t = Target::new("linux", "x86_64");
        let prefs = ext_preference(&t, None, false);
        let run = || {
            check_inner(
                "0.1.0",
                DEFAULT_REPO,
                Channel::Stable,
                &t,
                &prefs,
                &srv.base,
                HostPolicy::LOOPBACK_TEST,
                &ReleaseCache::on_disk(tmp.path()),
            )
        };
        assert_eq!(run().unwrap().latest, "0.2.0");
        assert_eq!(run().unwrap().latest, "0.2.0", "second run is served from the disk copy after a 304");
    }

    #[test]
    fn rate_limit_and_errors() {
        let t = Target::new("linux", "x86_64");
        let srv = serve(|_, _| Reply::status(403).header("x-ratelimit-remaining", "0").header("x-ratelimit-reset", "4102444800"));
        let e = check_local(&srv.base, "0.1.0", Channel::Stable, &t).unwrap_err();
        assert!(matches!(e, UpdateError::RateLimited { reset_at: Some(4102444800), .. }), "{e:?}");
        assert!(e.to_string().contains("rate limit"));

        let srv = serve(|_, _| Reply::status(429).header("retry-after", "120"));
        let e = check_local(&srv.base, "0.1.0", Channel::Stable, &t).unwrap_err();
        assert!(matches!(e, UpdateError::RateLimited { retry_after: Some(120), .. }), "{e:?}");
        assert!(e.to_string().contains("2 minute"), "{e}");

        // a plain 403 is not a rate limit
        let srv = serve(|_, _| Reply::status(403));
        assert!(matches!(check_local(&srv.base, "0.1.0", Channel::Stable, &t).unwrap_err(), UpdateError::Http(_)));
        let srv = serve(|_, _| Reply::status(404));
        assert!(matches!(check_local(&srv.base, "0.1.0", Channel::Stable, &t).unwrap_err(), UpdateError::NotFound(_)));
        let srv = serve(|_, _| Reply::ok("<html>not json</html>"));
        assert!(matches!(check_local(&srv.base, "0.1.0", Channel::Stable, &t).unwrap_err(), UpdateError::Parse(_)));
        let srv = serve(|_, _| Reply::ok("[]"));
        assert!(matches!(check_local(&srv.base, "0.1.0", Channel::Stable, &t).unwrap_err(), UpdateError::NoRelease));
        // bad inputs fail before the network
        assert!(matches!(
            check_local("http://127.0.0.1:1", "not-a-version", Channel::Stable, &t).unwrap_err(),
            UpdateError::BadVersion(..)
        ));
        for repo in ["", "a", "a/b/c", "../x", "a/..", "a b/c", "a/b?x=1"] {
            let prefs = ext_preference(&t, None, false);
            let e = check_inner(
                "0.1.0",
                repo,
                Channel::Stable,
                &t,
                &prefs,
                "http://127.0.0.1:1",
                HostPolicy::LOOPBACK_TEST,
                &ReleaseCache::in_memory(),
            )
            .unwrap_err();
            assert!(matches!(e, UpdateError::BadRepo(_)), "{repo:?}: {e:?}");
        }
    }

    #[test]
    fn real_api_host_is_https_only() {
        let t = Target::new("linux", "x86_64");
        let prefs = ext_preference(&t, None, false);
        let e = check_inner(
            "0.1.0",
            DEFAULT_REPO,
            Channel::Stable,
            &t,
            &prefs,
            "http://api.github.com",
            HostPolicy::API,
            &ReleaseCache::in_memory(),
        )
        .unwrap_err();
        assert!(matches!(e, UpdateError::NotHttps(_)), "{e:?}");
        let e = check_inner(
            "0.1.0",
            DEFAULT_REPO,
            Channel::Stable,
            &t,
            &prefs,
            "https://example.com",
            HostPolicy::API,
            &ReleaseCache::in_memory(),
        )
        .unwrap_err();
        assert!(matches!(e, UpdateError::UnsupportedHost(_)), "{e:?}");
    }

    // --- URL policy ------------------------------------------------------------------------------------------

    #[test]
    fn download_host_policy() {
        let p = HostPolicy::DOWNLOAD;
        for ok in [
            "https://github.com/andreaswiren/synofurb/releases/download/v1/x",
            "https://GitHub.com/a/b",
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://release-assets.githubusercontent.com/x?y=1",
            "https://githubusercontent.com/x",
            "https://github.com:443/x",
        ] {
            assert!(p.check(ok).is_ok(), "{ok}");
        }
        for (bad, kind) in [
            ("http://github.com/x", "https"),
            ("ftp://github.com/x", "https"),
            ("//github.com/x", "url"),
            ("github.com/x", "url"),
            ("https://evil.example.com/x", "host"),
            ("https://github.com.evil.com/x", "host"),
            ("https://evilgithub.com/x", "host"),
            ("https://notgithubusercontent.com/x", "host"),
            ("https://githubusercontent.com.evil.com/x", "host"),
            ("https://github.com@evil.com/x", "url"),
            ("https://evil.com@github.com/x", "url"),
            ("https://github.com:8443/x", "url"),
            ("https://github.com./x", "host"),
            ("https://127.0.0.1/x", "host"),
            ("http://127.0.0.1:8000/x", "https"),
            ("https://github.com\\@evil.com/x", "url"),
            ("https://", "url"),
        ] {
            let e = p.check(bad).unwrap_err();
            let got = match e {
                UpdateError::NotHttps(_) => "https",
                UpdateError::UnsupportedHost(_) => "host",
                UpdateError::BadUrl(_) => "url",
                other => panic!("{bad}: {other:?}"),
            };
            assert_eq!(got, kind, "{bad}");
        }
    }

    // --- checksums and signatures ----------------------------------------------------------------------------

    #[test]
    fn sha256sums_parsing() {
        let h = "a".repeat(64);
        let h2 = "B".repeat(64);
        let text = format!(
            "# comment\n\n{h}  synofurb-1.0.0-linux-x86_64.AppImage\n{h2} *synofurb-1.0.0-windows-x86_64.msi\r\n{h}  ./other-file.zip\n"
        );
        let t = parse_sha256sums(&text).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t["synofurb-1.0.0-windows-x86_64.msi"], "b".repeat(64), "digests are normalised to lower case");
        assert_eq!(t["other-file.zip"], h);
        for bad in [
            "abc  file".to_string(),
            h.clone(),
            format!("{h}  ../etc/passwd"),
            format!("{h}  dir/file"),
            format!("{}g  file", "a".repeat(63)),
            format!("{h}  f\n{}  f", "c".repeat(64)),
        ] {
            assert!(parse_sha256sums(&bad).is_err(), "{bad:?}");
        }
        // the same digest twice is harmless
        assert!(parse_sha256sums(&format!("{h}  f\n{h}  f")).is_ok());
    }

    struct Keys {
        pk: String,
        sk: minisign::SecretKey,
        pkk: minisign::PublicKey,
    }
    fn keys() -> Keys {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Keys { pk: kp.pk.to_base64(), sk: kp.sk, pkk: kp.pk }
    }
    fn sign(k: &Keys, data: &[u8]) -> String {
        minisign::sign(Some(&k.pkk), &k.sk, std::io::Cursor::new(data), Some("trusted"), Some("test")).unwrap().to_string()
    }

    #[test]
    fn signature_verification() {
        let k = keys();
        let data = b"hello sums";
        let sig = sign(&k, data);
        verify_signature(data, &sig, &k.pk).unwrap();
        // the key may also be given as the whole .pub file
        let pub_file = format!("untrusted comment: minisign public key\n{}\n", k.pk);
        verify_signature(data, &sig, &pub_file).unwrap();
        // tampered data
        assert!(matches!(verify_signature(b"hello sumz", &sig, &k.pk).unwrap_err(), UpdateError::SignatureInvalid(_)));
        // another key
        let other = keys();
        assert!(matches!(verify_signature(data, &sig, &other.pk).unwrap_err(), UpdateError::SignatureInvalid(_)));
        // garbage
        assert!(matches!(verify_signature(data, "garbage", &k.pk).unwrap_err(), UpdateError::SignatureInvalid(_)));
        assert!(matches!(verify_signature(data, &sig, "RWnotakey").unwrap_err(), UpdateError::BadSigningKey(_)));
        // placeholder is never accepted
        assert!(matches!(verify_signature(data, &sig, UPDATE_PUBLIC_KEY_PLACEHOLDER).unwrap_err(), UpdateError::NoSigningKey));
        assert!(matches!(verify_signature(data, &sig, "  ").unwrap_err(), UpdateError::NoSigningKey));
    }

    #[test]
    fn shipped_key_is_still_the_placeholder_and_that_blocks_updates() {
        // If you are the maintainer and this fails because you set a real key: delete this test.
        assert!(signing_key_is_placeholder(UPDATE_PUBLIC_KEY), "UPDATE_PUBLIC_KEY was replaced; remove this test");
        let e = ensure_can_verify(UPDATE_PUBLIC_KEY).unwrap_err();
        assert!(e.to_string().contains("placeholder"), "{e}");
        assert!(e.to_string().contains("minisign -G"), "{e}");
    }

    // --- download_verified -----------------------------------------------------------------------------------

    struct Release {
        info: UpdateInfo,
        srv: crate::compat::test_http::TestServer,
        hits: Arc<Mutex<Vec<String>>>,
    }

    /// A release served from a local server. `served_payload` may differ from `signed_payload` to simulate tampering.
    fn make_release(
        k: &Keys,
        signed_payload: &[u8],
        served_payload: Vec<u8>,
        sums_override: Option<String>,
        sig_key: Option<&Keys>,
    ) -> Release {
        let name = "synofurb-0.2.0-linux-x86_64.AppImage";
        let sums = sums_override.unwrap_or_else(|| format!("{}  {name}\n", hex::encode(Sha256::digest(signed_payload))));
        let sig = sign(sig_key.unwrap_or(k), sums.as_bytes());
        let hits: Arc<Mutex<Vec<String>>> = Arc::default();
        let h2 = hits.clone();
        let sums2 = sums.clone();
        let served = served_payload.clone();
        let srv = serve(move |path, _| {
            h2.lock().unwrap().push(path.to_string());
            match path {
                "/SHA256SUMS" => Reply::ok(sums2.clone()),
                "/SHA256SUMS.minisig" => Reply::ok(sig.clone()),
                "/asset" => Reply::ok(served.clone()),
                "/redirect-evil" => Reply::status(302).header("Location", "https://evil.example.com/asset"),
                "/redirect-ok" => Reply::status(302).header("Location", "/asset"),
                _ => Reply::status(404),
            }
        });
        let a = |n: &str, p: &str, size: u64| AssetInfo { name: n.into(), url: format!("{}{p}", srv.base), size };
        let info = UpdateInfo {
            current: "0.1.0".into(),
            latest: "0.2.0".into(),
            tag: "v0.2.0".into(),
            update_available: true,
            channel: Channel::Stable,
            prerelease: false,
            published_at: None,
            notes: String::new(),
            html_url: None,
            asset: Some(a(name, "/asset", served_payload.len() as u64)),
            checksums: Some(a(SUMS_ASSET, "/SHA256SUMS", 100)),
            signature: Some(a(SIGNATURE_ASSET, "/SHA256SUMS.minisig", 100)),
            problems: vec![],
        };
        Release { info, srv, hits }
    }

    fn payload() -> Vec<u8> {
        (0..200_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn dl(rel: &Release, dir: &Path, key: &str) -> (Result<VerifiedDownload>, Vec<Progress>) {
        let mut seen = vec![];
        let r = download_verified_with(&rel.info, dir, key, HostPolicy::LOOPBACK_TEST, &mut |p| seen.push(p));
        (r, seen)
    }

    #[test]
    fn download_verified_happy_path() {
        let k = keys();
        let data = payload();
        let rel = make_release(&k, &data, data.clone(), None, None);
        let tmp = TempDir::new("upd-ok");
        let (r, progress) = dl(&rel, tmp.path(), &k.pk);
        let v = r.unwrap();
        assert_eq!(std::fs::read(&v.path).unwrap(), data);
        assert_eq!(v.sha256, hex::encode(Sha256::digest(&data)));
        assert_eq!(v.path.file_name().unwrap(), "synofurb-0.2.0-linux-x86_64.AppImage");
        assert!(v.signed && v.integrity == Integrity::Signed);
        assert!(!tmp.path().join("synofurb-0.2.0-linux-x86_64.AppImage.part").exists());
        assert_eq!(progress.first().unwrap().downloaded, 0);
        let last = progress.last().unwrap();
        assert_eq!((last.downloaded, last.total), (data.len() as u64, Some(data.len() as u64)));
        assert!(progress.windows(2).all(|w| w[0].downloaded <= w[1].downloaded), "monotonic");
        let _ = &rel.srv;
    }

    #[test]
    fn download_follows_same_policy_redirects_only() {
        let k = keys();
        let data = payload();
        let mut rel = make_release(&k, &data, data.clone(), None, None);
        let tmp = TempDir::new("upd-redir");
        // relative redirect is fine
        rel.info.asset.as_mut().unwrap().url = format!("{}/redirect-ok", rel.srv.base);
        assert!(dl(&rel, tmp.path(), &k.pk).0.is_ok());
        // redirect to a foreign host is refused even though the first URL was fine
        let tmp = TempDir::new("upd-redir2");
        rel.info.asset.as_mut().unwrap().url = format!("{}/redirect-evil", rel.srv.base);
        let (r, _) = dl(&rel, tmp.path(), &k.pk);
        // LOOPBACK_TEST allows any host, so use the production policy on the second hop instead
        assert!(r.is_err(), "evil.example.com is not reachable here, and in production it is refused by policy");
        let e = HostPolicy::DOWNLOAD.check("https://evil.example.com/asset").unwrap_err();
        assert!(matches!(e, UpdateError::UnsupportedHost(_)));
    }

    #[test]
    fn checksum_mismatch_is_rejected_and_nothing_is_left_behind() {
        let k = keys();
        let data = payload();
        let mut evil = data.clone();
        evil[1000] ^= 0xff;
        let rel = make_release(&k, &data, evil, None, None);
        let tmp = TempDir::new("upd-sum");
        let (r, _) = dl(&rel, tmp.path(), &k.pk);
        let e = r.unwrap_err();
        assert!(matches!(e, UpdateError::ChecksumMismatch { .. }), "{e:?}");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0, "no .part or final file may remain");
    }

    #[test]
    fn signature_mismatch_stops_before_the_asset_is_downloaded() {
        let k = keys();
        let attacker = keys();
        let data = payload();
        // SHA256SUMS signed with the wrong key
        let rel = make_release(&k, &data, data.clone(), None, Some(&attacker));
        let tmp = TempDir::new("upd-sig");
        let (r, _) = dl(&rel, tmp.path(), &k.pk);
        assert!(matches!(r.unwrap_err(), UpdateError::SignatureInvalid(_)));
        assert!(!rel.hits.lock().unwrap().iter().any(|p| p == "/asset"), "asset must not be fetched: {:?}", rel.hits.lock().unwrap());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);

        // a valid signature over *different* sums text than what is served
        let good = make_release(&k, &data, data.clone(), None, None);
        let mut tampered = good;
        let other_sums = format!("{}  synofurb-0.2.0-linux-x86_64.AppImage\n", "0".repeat(64));
        let rel2 = make_release(&k, &data, data.clone(), Some(other_sums.clone()), None);
        // serve rel2's (signed) signature over rel's sums: simplest is to sign other_sums but serve the real sums
        let sig_other = sign(&k, other_sums.as_bytes());
        let sums_real = format!("{}  synofurb-0.2.0-linux-x86_64.AppImage\n", hex::encode(Sha256::digest(&data)));
        let srv = serve(move |p, _| match p {
            "/SHA256SUMS" => Reply::ok(sums_real.clone()),
            "/SHA256SUMS.minisig" => Reply::ok(sig_other.clone()),
            _ => Reply::status(404),
        });
        tampered.info.checksums.as_mut().unwrap().url = format!("{}/SHA256SUMS", srv.base);
        tampered.info.signature.as_mut().unwrap().url = format!("{}/SHA256SUMS.minisig", srv.base);
        let tmp = TempDir::new("upd-sig2");
        assert!(matches!(dl(&tampered, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::SignatureInvalid(_)));
        drop(rel2);
    }

    #[test]
    fn placeholder_key_refuses_without_touching_the_network() {
        let k = keys();
        let data = payload();
        let rel = make_release(&k, &data, data.clone(), None, None);
        let tmp = TempDir::new("upd-ph");
        let (r, progress) = dl(&rel, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER);
        assert!(matches!(r.unwrap_err(), UpdateError::NoSigningKey));
        assert!(rel.hits.lock().unwrap().is_empty(), "no request may be made");
        assert!(progress.is_empty());
        // and the public entry point with the shipped constant behaves the same
        let mut called = false;
        let e = download_verified(&rel.info, tmp.path(), &mut |_| called = true).unwrap_err();
        assert!(matches!(e, UpdateError::NoSigningKey) && !called);
    }

    // ---- the opt-in unsigned path (placeholder key only)

    fn dl_unsigned(rel: &Release, dir: &Path, key: &str) -> (Result<VerifiedDownload>, Vec<Progress>) {
        let mut seen = vec![];
        let r =
            download_unchecked_signature_with(&rel.info, dir, key, UnsignedConsent::acknowledged(), HostPolicy::LOOPBACK_TEST, &mut |p| {
                seen.push(p)
            });
        (r, seen)
    }

    /// A release as the project publishes it today: SHA256SUMS, but no `.minisig`.
    fn unsigned_release(data: &[u8], served: Vec<u8>) -> (Keys, Release) {
        let k = keys();
        let mut rel = make_release(&k, data, served, None, None);
        rel.info.signature = None;
        (k, rel)
    }

    #[test]
    fn unsigned_path_works_only_with_the_placeholder_key_and_is_labelled() {
        let data = payload();
        let (_k, rel) = unsigned_release(&data, data.clone());
        let tmp = TempDir::new("upd-uns");
        let (r, progress) = dl_unsigned(&rel, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER);
        let v = r.unwrap();
        assert_eq!(std::fs::read(&v.path).unwrap(), data);
        assert!(!v.signed && v.integrity == Integrity::ChecksumOnly);
        assert_eq!(v.sha256, hex::encode(Sha256::digest(&data)));
        assert!(progress.len() > 2);
        let hits = rel.hits.lock().unwrap().clone();
        assert!(hits.contains(&"/SHA256SUMS".to_string()) && hits.contains(&"/asset".to_string()), "{hits:?}");
        assert!(!hits.iter().any(|h| h.contains("minisig")), "the signature is not needed or fetched: {hits:?}");
        assert!(!tmp.path().join("synofurb-0.2.0-linux-x86_64.AppImage.part").exists());
        // the same release is not installable signed, and says what it is
        assert!(!rel.info.installable());
        assert_eq!(rel.info.install_mode_for(UPDATE_PUBLIC_KEY_PLACEHOLDER), InstallMode::UnsignedWithConsent);
    }

    #[test]
    fn a_real_key_makes_the_unsigned_path_impossible() {
        let data = payload();
        let (k, rel) = unsigned_release(&data, data.clone());
        let tmp = TempDir::new("upd-uns-real");
        let (r, progress) = dl_unsigned(&rel, tmp.path(), &k.pk);
        assert!(matches!(r.unwrap_err(), UpdateError::UnsignedNotAllowed));
        assert!(rel.hits.lock().unwrap().is_empty(), "nothing is requested");
        assert!(progress.is_empty() && std::fs::read_dir(tmp.path()).unwrap().count() == 0);
        // and the release without a signature is blocked, never offered unsigned
        match rel.info.install_mode_for(&k.pk) {
            InstallMode::Blocked(why) => assert!(why.contains("SHA256SUMS.minisig") && why.contains("signed"), "{why}"),
            other => panic!("{other:?}"),
        }
        // a signed release with the real key is still signed
        let signed = make_release(&k, &data, data.clone(), None, None);
        assert_eq!(signed.info.install_mode_for(&k.pk), InstallMode::Signed);
        let v = dl(&signed, tmp.path(), &k.pk).0.unwrap();
        assert!(v.signed && v.integrity == Integrity::Signed);
        // the shipped build: the constant decides
        assert_eq!(unsigned_allowed(), signing_key_is_placeholder(UPDATE_PUBLIC_KEY));
    }

    #[test]
    fn unsigned_path_still_checks_the_checksum_and_leaves_nothing_behind() {
        let data = payload();
        let mut evil = data.clone();
        evil[500] ^= 1;
        let (_k, rel) = unsigned_release(&data, evil);
        let tmp = TempDir::new("upd-uns-bad");
        let e = dl_unsigned(&rel, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER).0.unwrap_err();
        assert!(matches!(e, UpdateError::ChecksumMismatch { .. }), "{e:?}");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0, "no .part and no final file");

        // no entry for the file, a malformed or hostile entry, a missing SHA256SUMS
        let k = keys();
        for (sums, want) in [
            (format!("{}  some-other-file.zip\n", "1".repeat(64)), "missing"),
            (format!("{}  ../../etc/passwd\n", "1".repeat(64)), "bad"),
            ("not a checksum line\n".to_string(), "bad"),
            (
                format!(
                    "{h}  synofurb-0.2.0-linux-x86_64.AppImage\n{g}  synofurb-0.2.0-linux-x86_64.AppImage\n",
                    h = "a".repeat(64),
                    g = "b".repeat(64)
                ),
                "bad",
            ),
        ] {
            let mut r = make_release(&k, &data, data.clone(), Some(sums), None);
            r.info.signature = None;
            let e = dl_unsigned(&r, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER).0.unwrap_err();
            match want {
                "missing" => assert!(matches!(e, UpdateError::ChecksumMissing(_)), "{e:?}"),
                _ => assert!(matches!(e, UpdateError::BadChecksumFile(_)), "{e:?}"),
            }
            assert!(!r.hits.lock().unwrap().iter().any(|h| h == "/asset"), "the asset is not fetched when SHA256SUMS is bad");
        }
        let mut r = make_release(&k, &data, data.clone(), None, None);
        r.info.signature = None;
        r.info.checksums = None;
        assert!(matches!(dl_unsigned(&r, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER).0.unwrap_err(), UpdateError::MissingAsset(SUMS_ASSET)));
        assert!(r.hits.lock().unwrap().is_empty());
        assert_eq!(r.info.install_mode_for(UPDATE_PUBLIC_KEY_PLACEHOLDER).name(), "blocked");
        // the file may not be larger than the release says
        let mut r = make_release(&k, &data, data.clone(), None, None);
        r.info.signature = None;
        r.info.asset.as_mut().unwrap().size = 1000;
        assert!(matches!(dl_unsigned(&r, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER).0.unwrap_err(), UpdateError::TooLarge { .. }));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn unsigned_path_keeps_the_host_policy_and_the_name_rules() {
        let data = payload();
        let (_k, rel) = unsigned_release(&data, data.clone());
        let tmp = TempDir::new("upd-uns-host");
        // production policy: plain http to the local server is refused before any request
        let r = download_unchecked_signature_with(
            &rel.info,
            tmp.path(),
            UPDATE_PUBLIC_KEY_PLACEHOLDER,
            UnsignedConsent::acknowledged(),
            HostPolicy::DOWNLOAD,
            &mut |_| {},
        );
        assert!(matches!(r.unwrap_err(), UpdateError::NotHttps(_)));
        assert!(rel.hits.lock().unwrap().is_empty());
        // other hosts, for the checksum file as well as for the asset
        for (which, url) in [("asset", "https://evil.example.com/a"), ("sums", "https://github.com.evil.com/SHA256SUMS")] {
            let (_k2, mut r) = unsigned_release(&data, data.clone());
            let a = if which == "asset" { r.info.asset.as_mut().unwrap() } else { r.info.checksums.as_mut().unwrap() };
            a.url = url.into();
            let e = download_unchecked_signature_with(
                &r.info,
                tmp.path(),
                UPDATE_PUBLIC_KEY_PLACEHOLDER,
                UnsignedConsent::acknowledged(),
                HostPolicy::DOWNLOAD,
                &mut |_| {},
            )
            .unwrap_err();
            assert!(matches!(e, UpdateError::NotHttps(_) | UpdateError::UnsupportedHost(_)), "{which}: {e:?}");
            assert!(r.hits.lock().unwrap().is_empty());
        }
        for bad in ["../evil", "a/b", ".hidden", ""] {
            let (_k3, mut r) = unsigned_release(&data, data.clone());
            r.info.asset.as_mut().unwrap().name = bad.into();
            let e = dl_unsigned(&r, tmp.path(), UPDATE_PUBLIC_KEY_PLACEHOLDER).0.unwrap_err();
            assert!(matches!(e, UpdateError::BadAssetName(_)), "{bad:?}: {e:?}");
        }
        // the public entry point with the shipped key and production policy never reaches the local server either
        let e = download_unchecked_signature(&rel.info, tmp.path(), UnsignedConsent::acknowledged(), &mut |_| {}).unwrap_err();
        assert!(matches!(e, UpdateError::NotHttps(_) | UpdateError::UnsignedNotAllowed), "{e:?}");
    }

    #[test]
    fn release_notes_become_plain_text() {
        let md = "## What's new\r\n\r\n**Read this first.** This build is *not* signed yet; see `SHA256SUMS`.\r\n\r\n- Fixed the `recover` command\r\n- Added [the guide](https://example.com/guide) and ![logo](x.png)\r\n  - nested **bold** item\r\n* star bullet with snake_case_name\r\n\r\n\r\n\r\n> quoted line\r\n---\r\n```sh\r\nsynofurb-cli self-update\r\n```\r\nSee <https://github.com/andreaswiren/synofurb/releases> and <b>html</b>.<br>Done \\*escaped\\*.";
        let t = plain_release_notes(md);
        let want = [
            "What's new",
            "",
            "Read this first. This build is not signed yet; see SHA256SUMS.",
            "",
            "\u{2022} Fixed the recover command",
            "\u{2022} Added the guide (https://example.com/guide) and logo",
            "  \u{2022} nested bold item",
            "\u{2022} star bullet with snake_case_name",
            "",
            "  quoted line",
            "    synofurb-cli self-update",
            "See https://github.com/andreaswiren/synofurb/releases and html. Done *escaped*.",
        ]
        .join("\n");
        assert_eq!(t, want);
        assert!(!t.contains("**") && !t.contains('`') && !t.contains("]("));
        assert_eq!(plain_release_notes(""), "");
        assert_eq!(plain_release_notes("a < b and 2 * 3 * 4"), "a < b and 2 * 3 * 4");
        assert_eq!(plain_release_notes("[https://x.org](https://x.org)"), "https://x.org");
    }

    #[test]
    fn consent_cannot_be_forged_and_the_warning_is_the_agreed_one() {
        // the only constructor is `acknowledged()`; the warning says what is and is not checked
        let _c = UnsignedConsent::acknowledged();
        assert!(UNSIGNED_WARNING.contains("SHA-256 listed in SHA256SUMS of the same GitHub release"));
        assert!(UNSIGNED_WARNING.contains("tampered release"));
        assert_eq!(InstallMode::Signed.name(), "signed");
        assert!(InstallMode::Blocked("x".into()).is_blocked());
    }

    #[test]
    fn missing_checksum_entry_or_assets() {
        let k = keys();
        let data = payload();
        let sums = format!("{}  some-other-file.zip\n", "1".repeat(64));
        let rel = make_release(&k, &data, data.clone(), Some(sums), None);
        let tmp = TempDir::new("upd-miss");
        assert!(matches!(dl(&rel, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::ChecksumMissing(_)));

        let mut rel = make_release(&k, &data, data.clone(), None, None);
        rel.info.signature = None;
        assert!(matches!(dl(&rel, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::MissingAsset(SIGNATURE_ASSET)));
        rel.info.checksums = None;
        assert!(matches!(dl(&rel, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::MissingAsset(SUMS_ASSET)));
        rel.info.asset = None;
        assert!(matches!(dl(&rel, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::NoAssetForTarget(_)));
    }

    #[test]
    fn wrong_host_and_unsafe_names_are_rejected_before_any_request() {
        let k = keys();
        let data = payload();
        let rel = make_release(&k, &data, data.clone(), None, None);
        let tmp = TempDir::new("upd-host");

        // production policy against the local server: plain http and a non-GitHub host
        let r = download_verified_with(&rel.info, tmp.path(), &k.pk, HostPolicy::DOWNLOAD, &mut |_| {});
        assert!(matches!(r.unwrap_err(), UpdateError::NotHttps(_)));
        assert!(rel.hits.lock().unwrap().is_empty());

        for (url, want_https) in
            [("https://evil.example.com/a", false), ("https://github.com.evil.com/a", false), ("http://github.com/a", true)]
        {
            let mut r2 = make_release(&k, &data, data.clone(), None, None);
            r2.info.asset.as_mut().unwrap().url = url.into();
            let e = download_verified_with(&r2.info, tmp.path(), &k.pk, HostPolicy::DOWNLOAD, &mut |_| {}).unwrap_err();
            // the checksum URLs point at the local server and fail first; either way nothing is fetched
            assert!(matches!(e, UpdateError::NotHttps(_) | UpdateError::UnsupportedHost(_)), "{url}: {e:?}");
            let _ = want_https;
            assert!(r2.hits.lock().unwrap().is_empty(), "{url}: no request may happen");
        }

        for bad in ["../evil", "a/b", ".hidden", "name with space", ""] {
            let mut r3 = make_release(&k, &data, data.clone(), None, None);
            r3.info.asset.as_mut().unwrap().name = bad.into();
            let e = dl(&r3, tmp.path(), &k.pk).0.unwrap_err();
            assert!(matches!(e, UpdateError::BadAssetName(_)), "{bad:?}: {e:?}");
        }
    }

    #[test]
    fn oversized_and_short_downloads_are_rejected() {
        let k = keys();
        let data = payload();
        // release says the asset is smaller than what the server delivers
        let mut rel = make_release(&k, &data, data.clone(), None, None);
        rel.info.asset.as_mut().unwrap().size = (data.len() / 2) as u64;
        let tmp = TempDir::new("upd-big");
        assert!(matches!(dl(&rel, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::TooLarge { .. }));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        // release says larger than delivered
        let mut rel = make_release(&k, &data, data.clone(), None, None);
        rel.info.asset.as_mut().unwrap().size = (data.len() * 2) as u64;
        assert!(matches!(dl(&rel, tmp.path(), &k.pk).0.unwrap_err(), UpdateError::SizeMismatch { .. }));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    // --- applying --------------------------------------------------------------------------------------------

    #[test]
    fn apply_plans() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            plan_apply(&p("/c/s-0.2.0-windows-x86_64.msi"), "windows", false),
            ApplyPlan::LaunchInstaller {
                program: "msiexec".into(),
                args: vec!["/i".into(), "/c/s-0.2.0-windows-x86_64.msi".into(), "/passive".into()]
            }
        );
        assert!(matches!(plan_apply(&p("/c/x.exe"), "windows", false), ApplyPlan::LaunchInstaller { args, .. } if args.is_empty()));
        assert!(matches!(plan_apply(&p("/c/x.zip"), "windows", false), ApplyPlan::Manual { .. }));
        assert_eq!(
            plan_apply(&p("/c/x.dmg"), "macos", false),
            ApplyPlan::OpenFile { program: "open".into(), args: vec!["/c/x.dmg".into()] }
        );
        assert!(matches!(plan_apply(&p("/c/x.pkg"), "macos", false), ApplyPlan::OpenFile { .. }));
        assert_eq!(
            plan_apply(&p("/c/x.AppImage"), "linux", true),
            ApplyPlan::ReplaceExecutable { source: p("/c/x.AppImage"), from_tar_gz: false }
        );
        assert_eq!(
            plan_apply(&p("/c/x.tar.gz"), "linux", true),
            ApplyPlan::ReplaceExecutable { source: p("/c/x.tar.gz"), from_tar_gz: true }
        );
        assert_eq!(
            plan_apply(&p("/c/x.deb"), "linux", true),
            ApplyPlan::SystemPackage {
                command: vec!["apt-get".into(), "install".into(), "-y".into(), "/c/x.deb".into()],
                use_pkexec: true
            }
        );
        assert!(matches!(plan_apply(&p("/c/x.deb"), "linux", false), ApplyPlan::SystemPackage { use_pkexec: false, .. }));
        assert!(matches!(plan_apply(&p("/c/x.rpm"), "linux", true), ApplyPlan::SystemPackage { command, .. } if command[0] == "rpm"));
        // wrong extension for the OS never runs anything
        assert!(matches!(plan_apply(&p("/c/x.msi"), "linux", true), ApplyPlan::Manual { .. }));
        assert!(matches!(plan_apply(&p("/c/x.deb"), "windows", true), ApplyPlan::Manual { .. }));
        assert!(matches!(plan_apply(&p("/c/x.AppImage"), "macos", true), ApplyPlan::Manual { .. }));
    }

    #[test]
    fn shell_quoting_of_printed_commands() {
        assert_eq!(shell_quote("/tmp/a-b_c.deb"), "/tmp/a-b_c.deb");
        assert_eq!(shell_quote("/tmp/it's here.deb"), "'/tmp/it'\\''s here.deb'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn atomic_replace_keeps_a_backup_and_leaves_no_temp_files() {
        let tmp = TempDir::new("upd-rep");
        let exe = tmp.path().join("synofurb");
        let src = tmp.path().join("new.AppImage");
        std::fs::write(&exe, b"OLD-BINARY").unwrap();
        std::fs::write(&src, b"NEW-BINARY").unwrap();

        let backup = replace_executable(&src, &exe, false).unwrap();
        assert_eq!(backup, backup_path(&exe));
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW-BINARY");
        assert_eq!(std::fs::read(&backup).unwrap(), b"OLD-BINARY");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let mut names: Vec<String> =
            std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["new.AppImage", "synofurb", "synofurb.old"], "no temp files left: {names:?}");

        // the backup is a separate inode: writing to the new file must not touch it
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(std::fs::metadata(&exe).unwrap().ino(), std::fs::metadata(&backup).unwrap().ino());
        }

        // a second update replaces the backup with the then-current version
        std::fs::write(&src, b"NEWER").unwrap();
        replace_executable(&src, &exe, false).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEWER");
        assert_eq!(std::fs::read(backup_path(&exe)).unwrap(), b"NEW-BINARY");

        // rollback restores the previous version; mark_start_ok removes the backup
        rollback(&exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW-BINARY");
        assert!(rollback(&exe).is_err(), "backup was consumed");
        replace_executable(&src, &exe, false).unwrap();
        assert!(mark_start_ok(&exe));
        assert!(!backup_path(&exe).exists());
        assert!(!mark_start_ok(&exe));
    }

    #[test]
    fn failed_replace_leaves_the_current_binary_untouched() {
        let tmp = TempDir::new("upd-fail");
        let exe = tmp.path().join("synofurb");
        std::fs::write(&exe, b"OLD").unwrap();
        // missing source
        assert!(replace_executable(&tmp.path().join("nope"), &exe, false).is_err());
        // empty source
        let empty = tmp.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert!(replace_executable(&empty, &exe, false).is_err());
        // not a gzip archive
        let junk = tmp.path().join("junk.tar.gz");
        std::fs::write(&junk, b"definitely not gzip").unwrap();
        assert!(replace_executable(&junk, &exe, true).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLD");
        assert!(!backup_path(&exe).exists(), "no backup is made for a failed update");
        let names: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!names.iter().any(|n| n.contains(".new")), "temp file cleaned up: {names:?}");
    }

    #[test]
    fn replace_from_tar_gz_extracts_only_the_named_binary() {
        let tmp = TempDir::new("upd-tar");
        let archive = tmp.path().join("synofurb-0.2.0-linux-x86_64.tar.gz");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            for (path, data) in [
                ("synofurb-0.2.0/README.md", &b"readme"[..]),
                ("synofurb-0.2.0/synofurb", &b"BINARY-FROM-TAR"[..]),
                ("evil/../../synofurb.sh", &b"x"[..]),
            ] {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                // set_path rejects ".."; write the raw name instead for the hostile entry
                if path.contains("..") {
                    h.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
                    h.set_cksum();
                    b.append(&h, data).unwrap();
                } else {
                    b.append_data(&mut h, path, data).unwrap();
                }
            }
            b.into_inner().unwrap().finish().unwrap();
        }
        let exe = tmp.path().join("bin").join("synofurb");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"OLD").unwrap();
        replace_executable(&archive, &exe, true).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"BINARY-FROM-TAR");
        assert_eq!(std::fs::read(backup_path(&exe)).unwrap(), b"OLD");
        assert!(!tmp.path().join("synofurb.sh").exists());
        assert!(!tmp.path().parent().unwrap().join("synofurb.sh").exists());

        // archive without the binary
        let archive2 = tmp.path().join("empty.tar.gz");
        {
            let f = std::fs::File::create(&archive2).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "other", &b"x"[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        assert!(replace_executable(&archive2, &exe, true).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"BINARY-FROM-TAR");
    }
}
