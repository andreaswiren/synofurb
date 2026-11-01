//! Synology model compatibility list.
//!
//! `compat/models.json` (embedded at build time) says, per Synology model, how well synofurb is known to work on it.
//! The app looks the detected model up here and combines the answer with the live platform detection
//! ([`crate::platform::classify`]) into a [`CompatVerdict`]. The list can be refreshed from the repository
//! ([`CompatDb::refresh`]); offline, the cached copy or the embedded one is used.
//!
//! Rules for the data itself are in `compat/README.md`; the statuses are defined by [`Status`].

use crate::model::{Arch, DeviceFacts};
use crate::platform::{self, Support};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The list shipped inside this build.
pub const EMBEDDED_JSON: &str = include_str!("../../../compat/models.json");
/// Where [`CompatDb::refresh`] fetches the current list from.
pub const DEFAULT_URL: &str = "https://raw.githubusercontent.com/andreaswiren/synofurb/main/compat/models.json";
/// Highest `schema_version` this build understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;
/// A downloaded list larger than this is rejected (the real file is a few hundred KiB).
pub const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024;

const CACHE_FILE: &str = "compat-models.json";
const ETAG_FILE: &str = "compat-models.etag";

// ---------------------------------------------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------------------------------------------

/// How well a model is known to work. Ordered from best to worst knowledge, not from "best" to "worst" outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Maintainer, or two independent reports with logs, on a current release.
    Verified,
    /// At least one report exists.
    Community,
    /// No report yet; expected to work in theory.
    Untested,
    /// Reports show something fails or only partly works.
    KnownIssues,
    /// The hardware cannot run Debian 13.
    Unsupported,
}

impl Status {
    pub const ALL: [Status; 5] = [Status::Verified, Status::Community, Status::Untested, Status::KnownIssues, Status::Unsupported];

    /// Human-readable name, e.g. "Known issues".
    pub fn label(self) -> &'static str {
        match self {
            Status::Verified => "Verified",
            Status::Community => "Community-reported",
            Status::Untested => "Untested",
            Status::KnownIssues => "Known issues",
            Status::Unsupported => "Unsupported",
        }
    }

    /// One or two sentences for a tooltip or help text.
    pub fn explain(self) -> &'static str {
        match self {
            Status::Verified => {
                "A maintainer, or two independent people, ran synofurb on this model and Debian installed and booted. Logs are on file."
            }
            Status::Community => {
                "At least one person has reported a result for this model. It has not been verified to the project's standard."
            }
            Status::Untested => {
                "Nobody has reported a run on this model yet. The hardware class should work in theory, but that is not a promise."
            }
            Status::KnownIssues => "Reports show that something fails or only partly works on this model. Read the notes before you start.",
            Status::Unsupported => {
                "This hardware cannot run Debian 13 (for example PowerPC, 32-bit x86 or ARMv5). synofurb will not install on it."
            }
        }
    }

    /// Short text for a badge. Deliberately words, not colours, so it works in a terminal and for screen readers.
    pub fn icon_word(self) -> &'static str {
        match self {
            Status::Verified => "VERIFIED",
            Status::Community => "COMMUNITY",
            Status::Untested => "UNTESTED",
            Status::KnownIssues => "ISSUES",
            Status::Unsupported => "UNSUPPORTED",
        }
    }
}

/// CPU architecture as a Debian architecture name (plus the dead ends the core [`Arch`] lumps together).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelArch {
    Amd64,
    Arm64,
    Armhf,
    Armel,
    Ppc,
    I386,
}

impl ModelArch {
    pub fn debian(self) -> &'static str {
        match self {
            ModelArch::Amd64 => "amd64",
            ModelArch::Arm64 => "arm64",
            ModelArch::Armhf => "armhf",
            ModelArch::Armel => "armel",
            ModelArch::Ppc => "ppc",
            ModelArch::I386 => "i386",
        }
    }
    /// The core [`Arch`] this corresponds to; the dead ends map to [`Arch::Unsupported`].
    pub fn core_arch(self) -> Arch {
        match self {
            ModelArch::Amd64 => Arch::X86_64,
            ModelArch::Arm64 => Arch::Arm64,
            ModelArch::Armhf => Arch::Armhf,
            ModelArch::Armel | ModelArch::Ppc | ModelArch::I386 => Arch::Unsupported,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Boot {
    Dom,
    Uboot,
    Unknown,
}

/// Filter bucket for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CpuVendor {
    Intel,
    Amd,
    Arm,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceResult {
    Success,
    Partial,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceKind {
    /// Somebody ran synofurb.
    #[default]
    Synofurb,
    /// Another tool booted Debian on this model; does not count towards `verified`.
    ThirdParty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(default)]
    pub date: Option<String>,
    pub reporter: String,
    #[serde(default)]
    pub kind: EvidenceKind,
    #[serde(default)]
    pub synofurb_version: Option<String>,
    #[serde(default)]
    pub dsm_version: Option<String>,
    pub result: EvidenceResult,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub log_url: Option<String>,
    #[serde(default)]
    pub maintainer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub model: String,
    pub series: String,
    #[serde(default)]
    pub year: Option<u16>,
    pub platform: String,
    #[serde(default)]
    pub cpu: Option<String>,
    pub vendor: String,
    pub cpu_vendor: CpuVendor,
    pub arch: ModelArch,
    pub boot: Boot,
    #[serde(default)]
    pub ram: Option<String>,
    #[serde(default)]
    pub ram_mb_stock: Option<u32>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub threads: Option<u32>,
    pub status: Status,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// On-disk / on-wire shape of `models.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatFile {
    pub schema_version: u32,
    pub updated: String,
    #[serde(default)]
    pub hardware_source: Option<String>,
    pub models: Vec<ModelEntry>,
}

// ---------------------------------------------------------------------------------------------------------------
// Normalisation and lookup
// ---------------------------------------------------------------------------------------------------------------

/// Canonical form of a model name used as lookup key: lower case, `plus` -> `+`, only `[a-z0-9+]` kept.
///
/// `"DS918+"`, `"ds918plus"`, `"DS 918 +"` and `"ds-918+"` all become `"ds918+"`.
pub fn normalize_model(s: &str) -> String {
    let lower = s.trim().to_ascii_lowercase().replace("plus", "+");
    lower.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '+').collect()
}

/// Model prefixes DSM omits in the `unique` value for most models (`synology_apollolake_918+`).
const PREFIXES: [&str; 11] = ["ds", "rs", "fs", "sa", "dva", "hd", "pas", "rc", "eds", "nvr", "cs"];

fn is_unique_form(s: &str) -> bool {
    s.trim().trim_matches('"').to_ascii_lowercase().starts_with("synology_")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbOrigin {
    Embedded,
    Cache,
}

/// The compatibility list plus a lookup index.
#[derive(Debug, Clone)]
pub struct CompatDb {
    file: CompatFile,
    index: HashMap<String, Vec<usize>>,
    origin: DbOrigin,
}

/// Result of [`CompatDb::refresh`]. Network trouble is an outcome, not an error: the caller keeps using whatever
/// list it already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// A new, valid list was downloaded and cached.
    Updated { models: usize, updated: String },
    /// The server answered 304: the cached list is current.
    NotModified,
    /// The server could not be reached (or answered with an error). Cache/embedded list stays in use.
    Offline { reason: String },
    /// The server answered, but the body failed validation (too big, corrupt JSON, wrong schema, ...). The cache
    /// was left untouched.
    Rejected { reason: String },
}

impl CompatDb {
    fn from_file(file: CompatFile, origin: DbOrigin) -> Result<CompatDb> {
        check_basic(&file).map_err(|e| anyhow!(e))?;
        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, m) in file.models.iter().enumerate() {
            index.entry(normalize_model(&m.model)).or_default().push(i);
        }
        Ok(CompatDb { file, index, origin })
    }

    /// Parse and validate a `models.json` body.
    pub fn from_json(json: &str, origin: DbOrigin) -> Result<CompatDb> {
        let file: CompatFile = serde_json::from_str(json).context("compat list is not valid JSON for this schema")?;
        CompatDb::from_file(file, origin)
    }

    /// The list compiled into this binary. Panics only if the committed `models.json` is broken, which the unit
    /// tests catch.
    pub fn embedded() -> CompatDb {
        CompatDb::from_json(EMBEDDED_JSON, DbOrigin::Embedded).expect("embedded compat/models.json is valid")
    }

    /// Load the cached copy written by [`CompatDb::refresh`].
    pub fn load_cached(dir: &Path) -> Result<CompatDb> {
        let path = dir.join(CACHE_FILE);
        let body = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        CompatDb::from_json(&body, DbOrigin::Cache)
    }

    /// The best list available offline: the cache if it is valid and at least as new as the embedded list,
    /// otherwise the embedded one (a newer app version may ship a newer list than an old cache).
    pub fn load_best(cache_dir: &Path) -> CompatDb {
        let embedded = CompatDb::embedded();
        match CompatDb::load_cached(cache_dir) {
            Ok(c) if c.file.updated >= embedded.file.updated => c,
            _ => embedded,
        }
    }

    pub fn origin(&self) -> DbOrigin {
        self.origin
    }
    /// Date (`YYYY-MM-DD`) of the list.
    pub fn updated(&self) -> &str {
        &self.file.updated
    }
    pub fn models(&self) -> &[ModelEntry] {
        &self.file.models
    }
    pub fn len(&self) -> usize {
        self.file.models.len()
    }
    pub fn is_empty(&self) -> bool {
        self.file.models.is_empty()
    }

    /// Number of models per status, in [`Status::ALL`] order.
    pub fn count_by_status(&self) -> Vec<(Status, usize)> {
        Status::ALL.iter().map(|s| (*s, self.file.models.iter().filter(|m| m.status == *s).count())).collect()
    }

    /// Find a model by marketing name or by DSM's `unique` value.
    ///
    /// Accepts `DS918+`, `ds918plus`, `DS 918 +`, `RS3618xs` and `synology_apollolake_918+`. DSM leaves the `DS`
    /// prefix off in many `unique` values; when that makes the name ambiguous (`214` is both a DS214 and an RS214)
    /// the platform in the `unique` value decides, and if that does not settle it the answer is `None` rather than a
    /// guess.
    pub fn lookup(&self, model_or_unique: &str) -> Option<&ModelEntry> {
        let (hint, frag) = if is_unique_form(model_or_unique) {
            let lower = model_or_unique.to_ascii_lowercase();
            (platform::platform_token(&lower).map(str::to_string), platform::model_from_unique(&lower)?)
        } else {
            (None, model_or_unique.to_string())
        };
        let key = normalize_model(&frag);
        if key.is_empty() {
            return None;
        }
        let pick = |idxs: Vec<usize>| -> Option<&ModelEntry> {
            let mut c: Vec<&ModelEntry> = idxs.into_iter().map(|i| &self.file.models[i]).collect();
            if c.len() > 1
                && let Some(h) = &hint
            {
                c.retain(|e| &e.platform == h);
            }
            if c.len() == 1 { Some(c[0]) } else { None }
        };
        if let Some(idxs) = self.index.get(&key) {
            return pick(idxs.clone());
        }
        if key.starts_with(|c: char| c.is_ascii_digit()) {
            let mut idxs = Vec::new();
            for p in PREFIXES {
                if let Some(v) = self.index.get(&format!("{p}{key}")) {
                    idxs.extend(v.iter().copied());
                }
            }
            return pick(idxs);
        }
        None
    }

    /// Combine the list entry with what was detected on the live device.
    pub fn status_for(&self, facts: &DeviceFacts) -> CompatVerdict<'_> {
        let live = platform::classify(&facts.unique, &facts.uname_m);
        let entry = self.lookup(&facts.model).or_else(|| self.lookup(&facts.unique));
        let mut details: Vec<String> = Vec::new();
        let name = if facts.model.trim().is_empty() { facts.unique.as_str() } else { facts.model.as_str() };

        let mut status = entry.map_or(Status::Untested, |e| e.status);
        let mut blocking = false;

        match entry {
            None => {
                details.push(format!(
                    "{} is not in the compatibility list (updated {}); it is treated as untested.",
                    if name.is_empty() { "This model" } else { name },
                    self.file.updated
                ));
            }
            Some(e) => {
                details.push(format!(
                    "List entry: {} ({}, {}, {}).",
                    e.model,
                    e.platform,
                    e.arch.debian(),
                    e.cpu.as_deref().unwrap_or("CPU not listed")
                ));
                if let Some(h) = platform::platform_token(&facts.unique)
                    && !h.eq_ignore_ascii_case(&e.platform)
                {
                    details.push(format!(
                        "The device reports platform '{h}' but the list has '{}' for {}; the live value is used for the support decision.",
                        e.platform, e.model
                    ));
                }
                let live_vendor = match facts.cpu_vendor {
                    crate::model::CpuVendor::Intel => Some(CpuVendor::Intel),
                    crate::model::CpuVendor::Amd => Some(CpuVendor::Amd),
                    crate::model::CpuVendor::Arm => Some(CpuVendor::Arm),
                    crate::model::CpuVendor::Other => None,
                };
                if live_vendor.is_some_and(|v| v != e.cpu_vendor) {
                    details.push(format!(
                        "The device reports a {} CPU but the list has {:?} for {}. Check that the model was detected correctly.",
                        facts.cpu_vendor.label(),
                        e.cpu_vendor,
                        e.model
                    ));
                }
                if e.arch.core_arch() != facts.arch && e.arch.core_arch() != Arch::Unsupported && facts.arch != Arch::Unsupported {
                    details.push(format!(
                        "The list says {} but the device reports {}. Check that the model was detected correctly.",
                        e.arch.debian(),
                        facts.arch.debian()
                    ));
                }
            }
        }

        details.push(format!("Detected platform: {} ({}); support level: {}.", live.platform, live.soc, live.support.label()));

        if live.support == Support::Unsupported {
            if status != Status::Unsupported {
                details.push(
                    "The list does not call this model unsupported, but the detected hardware cannot run Debian 13. Live detection wins."
                        .into(),
                );
            }
            status = Status::Unsupported;
        }
        if status == Status::Unsupported {
            blocking = true;
            if live.support != Support::Unsupported {
                details.push("The list marks this model unsupported although detection looks fine. Installation stays blocked; if you believe the entry is wrong, open an issue.".into());
            }
        }
        if live.support == Support::Experimental && status != Status::Unsupported {
            details.push(live.note.to_string());
        }

        if let Some(e) = entry {
            if !e.evidence.is_empty() {
                let ok = e.evidence.iter().filter(|x| x.result == EvidenceResult::Success).count();
                details.push(format!("{} report(s) on file, {} successful.", e.evidence.len(), ok));
                for ev in &e.evidence {
                    if ev.kind == EvidenceKind::ThirdParty {
                        details.push(format!("Reported by {} using their own tool, not synofurb.", ev.reporter));
                    }
                }
            }
            if let Some(n) = e.notes.as_deref().filter(|n| !n.is_empty()) {
                details.push(n.to_string());
            }
        }

        let headline = match (entry, status) {
            (None, Status::Unsupported) => format!("{name}: unsupported hardware"),
            (None, _) => format!("{name}: untested (model not in the list)"),
            (Some(e), s) => format!("{}: {}", e.model, s.label().to_lowercase()),
        };
        CompatVerdict { status, entry, headline, details, blocking }
    }

    // -----------------------------------------------------------------------------------------------------------
    // Refresh
    // -----------------------------------------------------------------------------------------------------------

    /// Fetch `url` (normally [`DEFAULT_URL`]) into `cache_dir`.
    ///
    /// Uses `ETag`/`If-None-Match`, caps the body at [`MAX_DOWNLOAD_BYTES`], validates the JSON before it replaces
    /// anything, and writes the cache atomically. Anything wrong with the network or the body is reported as an
    /// outcome and leaves the existing cache alone. `Err` is reserved for local problems (cannot create the cache
    /// directory, cannot write the file) and for a URL that is not https (plain http is accepted for loopback only,
    /// which the tests use).
    pub fn refresh(cache_dir: &Path, url: &str) -> Result<RefreshOutcome> {
        check_url(url)?;
        std::fs::create_dir_all(cache_dir).with_context(|| format!("creating {}", cache_dir.display()))?;
        let cache_path = cache_dir.join(CACHE_FILE);
        let etag_path = cache_dir.join(ETAG_FILE);

        // Only send the ETag if the cache it belongs to is still readable and valid.
        let etag = if CompatDb::load_cached(cache_dir).is_ok() {
            std::fs::read_to_string(&etag_path).ok().map(|s| s.trim().to_string())
        } else {
            None
        };

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirects(3)
            .user_agent(concat!("synofurb/", env!("CARGO_PKG_VERSION")))
            .build();
        let mut req = agent.get(url).set("Accept", "application/json");
        if let Some(e) = etag.as_deref().filter(|e| !e.is_empty()) {
            req = req.set("If-None-Match", e);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Ok(RefreshOutcome::Offline { reason: format!("server answered HTTP {code}") }),
            Err(e) => return Ok(RefreshOutcome::Offline { reason: e.to_string() }),
        };
        if resp.status() == 304 {
            return Ok(RefreshOutcome::NotModified);
        }
        if resp.status() != 200 {
            return Ok(RefreshOutcome::Offline { reason: format!("unexpected HTTP status {}", resp.status()) });
        }
        if let Some(len) = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok())
            && len > MAX_DOWNLOAD_BYTES
        {
            return Ok(RefreshOutcome::Rejected { reason: format!("response is {len} bytes, limit is {MAX_DOWNLOAD_BYTES}") });
        }
        let new_etag = resp.header("ETag").map(|s| s.trim().to_string());
        let mut body = Vec::new();
        if let Err(e) = resp.into_reader().take(MAX_DOWNLOAD_BYTES + 1).read_to_end(&mut body) {
            return Ok(RefreshOutcome::Offline { reason: format!("download interrupted: {e}") });
        }
        if body.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Ok(RefreshOutcome::Rejected { reason: format!("response exceeds {MAX_DOWNLOAD_BYTES} bytes") });
        }
        let text = match String::from_utf8(body) {
            Ok(t) => t,
            Err(_) => return Ok(RefreshOutcome::Rejected { reason: "response is not UTF-8".into() }),
        };
        let db = match CompatDb::from_json(&text, DbOrigin::Cache) {
            Ok(d) => d,
            Err(e) => return Ok(RefreshOutcome::Rejected { reason: format!("{e:#}") }),
        };
        write_atomic(&cache_path, text.as_bytes())?;
        match new_etag.filter(|e| !e.is_empty()) {
            Some(e) => write_atomic(&etag_path, e.as_bytes())?,
            None => {
                let _ = std::fs::remove_file(&etag_path);
            }
        }
        Ok(RefreshOutcome::Updated { models: db.len(), updated: db.file.updated.clone() })
    }
}

fn check_url(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host == "127.0.0.1" || host == "localhost" || host == "::1" {
            return Ok(());
        }
    }
    bail!("refusing to fetch the compatibility list over a non-https URL: {url}")
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let mut tmp: PathBuf = path.to_path_buf();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tmp.set_file_name(format!(".{name}.tmp{}", std::process::id()));
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Checks that make a downloaded list safe to use. The unit tests apply stricter, project-specific rules on top.
fn check_basic(f: &CompatFile) -> std::result::Result<(), String> {
    if f.schema_version == 0 || f.schema_version > SUPPORTED_SCHEMA_VERSION {
        return Err(format!("unsupported schema_version {} (this build understands up to {SUPPORTED_SCHEMA_VERSION})", f.schema_version));
    }
    if f.models.is_empty() {
        return Err("list has no models".into());
    }
    let mut seen = std::collections::HashSet::new();
    for m in &f.models {
        if m.model.trim().is_empty() || m.platform.trim().is_empty() {
            return Err("an entry has an empty model or platform".into());
        }
        if !seen.insert(m.model.clone()) {
            return Err(format!("duplicate model {}", m.model));
        }
        match m.status {
            Status::Community | Status::KnownIssues | Status::Verified if m.evidence.is_empty() => {
                return Err(format!("{}: status {} needs evidence", m.model, m.status.label()));
            }
            Status::Untested if !m.evidence.is_empty() => return Err(format!("{}: untested model has evidence", m.model)),
            _ => {}
        }
    }
    Ok(())
}

/// The answer shown to the user for one device.
#[derive(Debug, Clone)]
pub struct CompatVerdict<'a> {
    pub status: Status,
    /// The list entry, if the model is known.
    pub entry: Option<&'a ModelEntry>,
    /// One line, e.g. `DS918+: untested`.
    pub headline: String,
    /// Further lines: where the data came from, conflicts between list and live detection, notes.
    pub details: Vec<String>,
    /// True if the install must not proceed (hardware cannot run Debian 13).
    pub blocking: bool,
}

// ---------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------

/// A tiny HTTP server for tests (also used by the updater tests).
#[cfg(test)]
pub(crate) mod test_http {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub struct Reply {
        pub status: u16,
        pub headers: Vec<(&'static str, String)>,
        pub body: Vec<u8>,
    }
    impl Reply {
        pub fn ok(body: impl Into<Vec<u8>>) -> Reply {
            Reply { status: 200, headers: vec![], body: body.into() }
        }
        pub fn status(status: u16) -> Reply {
            Reply { status, headers: vec![], body: vec![] }
        }
        pub fn header(mut self, k: &'static str, v: impl Into<String>) -> Reply {
            self.headers.push((k, v.into()));
            self
        }
    }

    pub struct TestServer {
        pub base: String,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    /// `handler(path_with_query, request_headers) -> Reply`. Header names are lower-cased.
    pub fn serve<F>(handler: F) -> TestServer
    where
        F: Fn(&str, &std::collections::HashMap<String, String>) -> Reply + Send + 'static,
    {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().expect("ip addr").port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                let Ok(Some(req)) = server.recv_timeout(std::time::Duration::from_millis(50)) else { continue };
                let headers: std::collections::HashMap<String, String> =
                    req.headers().iter().map(|h| (h.field.to_string().to_ascii_lowercase(), h.value.to_string())).collect();
                let reply = handler(req.url(), &headers);
                let mut resp = tiny_http::Response::from_data(reply.body).with_status_code(reply.status);
                for (k, v) in reply.headers {
                    resp.add_header(tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap());
                }
                let _ = req.respond(resp);
            }
        });
        TestServer { base: format!("http://127.0.0.1:{port}"), stop, handle: Some(handle) }
    }

    /// A unique scratch directory, removed on drop.
    pub struct TempDir(pub std::path::PathBuf);
    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            use std::sync::atomic::AtomicUsize;
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!("synofurb-test-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_http::*;
    use super::*;
    use crate::model::{Firmware, Nic};
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    /// Collected from Synology's CPU page + the SynoCommunity wiki (see compat/source/synology-cpu-table.csv).
    const MIN_MODELS: usize = 272;

    #[test]
    fn normalisation_table() {
        let cases = [
            ("DS918+", "ds918+"),
            ("ds918plus", "ds918+"),
            ("DS918Plus", "ds918+"),
            ("  DS 918 + ", "ds918+"),
            ("ds-918+", "ds918+"),
            ("RS3618xs", "rs3618xs"),
            ("RS3618XS", "rs3618xs"),
            ("DS411+ II", "ds411+ii"),
            ("DS411+II", "ds411+ii"),
            ("DS-106e", "ds106e"),
            ("DS220j", "ds220j"),
            ("ds1821plus", "ds1821+"),
            ("RS2423RP+", "rs2423rp+"),
            ("", ""),
        ];
        for (input, want) in cases {
            assert_eq!(normalize_model(input), want, "input {input:?}");
        }
    }

    #[test]
    fn lookup_forms() {
        let db = CompatDb::embedded();
        for q in [
            "DS918+",
            "ds918+",
            "ds918plus",
            "DS 918 plus",
            "synology_apollolake_918+",
            "\"synology_apollolake_918+\"",
            "SYNOLOGY_APOLLOLAKE_918+",
            "918+",
        ] {
            assert_eq!(db.lookup(q).map(|e| e.model.as_str()), Some("DS918+"), "query {q:?}");
        }
        assert_eq!(db.lookup("RS3618xs").map(|e| e.platform.as_str()), Some("broadwell"));
        assert_eq!(db.lookup("synology_broadwell_rs3618xs").map(|e| e.model.as_str()), Some("RS3618xs"));
        assert_eq!(db.lookup("synology_rtd1296_ds118").map(|e| e.model.as_str()), Some("DS118"));
        assert_eq!(db.lookup("synology_armada38x_ds218j").map(|e| e.model.as_str()), Some("DS218j"));
        assert_eq!(db.lookup("synology_r1000_923+").map(|e| e.model.as_str()), Some("DS923+"));
        assert_eq!(db.lookup("DS1522plus").map(|e| e.model.as_str()), Some("DS1522+"));
        assert_eq!(db.lookup("DS3617xsII").map(|e| e.model.as_str()), Some("DS3617xsII"));
        // unknown and malformed
        assert!(db.lookup("DS9999+").is_none());
        assert!(db.lookup("").is_none());
        assert!(db.lookup("synology_").is_none());
        assert!(db.lookup("synology_apollolake_").is_none());
        assert!(db.lookup("plus").is_none());
    }

    #[test]
    fn ambiguous_prefixless_names_use_the_platform_hint() {
        let db = CompatDb::embedded();
        // DS214 (armadaxp) and RS214 (armada370) both exist.
        assert!(db.lookup("214").is_none(), "ambiguous without a hint");
        assert_eq!(db.lookup("synology_armada370_214").map(|e| e.model.as_str()), Some("RS214"));
        assert_eq!(db.lookup("synology_armadaxp_214").map(|e| e.model.as_str()), Some("DS214"));
        // a full name is never ambiguous
        assert_eq!(db.lookup("DS214").map(|e| e.model.as_str()), Some("DS214"));
    }

    /// Hardware facts whose only interesting fields are model/unique/uname.
    fn facts(model: &str, unique: &str, uname: &str, arch: Arch) -> DeviceFacts {
        DeviceFacts {
            model: model.into(),
            unique: unique.into(),
            dsm_version: "7.2".into(),
            serial: String::new(),
            hostname: "nas".into(),
            uname_m: uname.into(),
            kernel: "4.4".into(),
            arch,
            mem_kib: 2 * 1024 * 1024,
            cpu: String::new(),
            firmware: Firmware::Bios,
            kexec: true,
            dt_model: String::new(),
            cpu_vendor: crate::model::CpuVendor::Other,
            dom: None,
            cmdline: String::new(),
            is_root: true,
            nics: Vec::<Nic>::new(),
            disks: vec![],
            arrays: vec![],
            lvs: vec![],
            mounts: vec![],
        }
    }

    #[test]
    fn verdicts() {
        let db = CompatDb::embedded();

        let v = db.status_for(&facts("DS918+", "synology_apollolake_918+", "x86_64", Arch::X86_64));
        assert_eq!(v.status, Status::Untested);
        assert!(!v.blocking);
        assert_eq!(v.entry.map(|e| e.model.as_str()), Some("DS918+"));
        assert!(v.headline.contains("DS918+"));

        let v = db.status_for(&facts("DS923+", "synology_r1000_923+", "x86_64", Arch::X86_64));
        assert_eq!(v.status, Status::Community);
        assert!(!v.blocking);
        assert!(v.details.iter().any(|d| d.contains("not synofurb")), "third-party evidence is labelled: {:?}", v.details);

        // PowerPC: unsupported and blocking, from the list and from live detection
        let v = db.status_for(&facts("DS413", "synology_qoriq_413", "ppc", Arch::Unsupported));
        assert_eq!(v.status, Status::Unsupported);
        assert!(v.blocking);

        // unknown model on supported hardware
        let v = db.status_for(&facts("DS9999+", "synology_apollolake_9999+", "x86_64", Arch::X86_64));
        assert_eq!(v.status, Status::Untested);
        assert!(v.entry.is_none());
        assert!(!v.blocking);
        assert!(v.details.iter().any(|d| d.contains("not in the compatibility list")));

        // unknown model on dead hardware: live detection blocks
        let v = db.status_for(&facts("DS9998", "synology_qoriq_9998", "ppc", Arch::Unsupported));
        assert_eq!(v.status, Status::Unsupported);
        assert!(v.blocking);

        // found via unique when the model string is empty
        let v = db.status_for(&facts("", "synology_geminilake_920+", "x86_64", Arch::X86_64));
        assert_eq!(v.entry.map(|e| e.model.as_str()), Some("DS920+"));
    }

    #[test]
    fn list_and_live_platform_conflict_is_reported() {
        let db = CompatDb::embedded();
        // DS918+ is apollolake in the list; pretend the device says geminilake
        let v = db.status_for(&facts("DS918+", "synology_geminilake_918+", "x86_64", Arch::X86_64));
        assert!(v.details.iter().any(|d| d.contains("reports platform 'geminilake'")), "{:?}", v.details);
    }

    #[test]
    fn status_words() {
        for s in Status::ALL {
            assert!(!s.label().is_empty() && !s.explain().is_empty());
            assert!(s.icon_word().chars().all(|c| c.is_ascii_uppercase()));
        }
        assert_eq!(Status::Verified.label(), "Verified");
        assert_eq!(serde_json::to_string(&Status::KnownIssues).unwrap(), "\"known-issues\"");
    }

    /// Project rules for the committed data. `check_basic` guards what is downloaded; this guards what is committed.
    #[test]
    fn embedded_db_obeys_schema_rules() {
        let db = CompatDb::embedded();
        assert!(db.len() >= MIN_MODELS, "only {} models, expected at least {MIN_MODELS}", db.len());
        assert_eq!(db.origin(), DbOrigin::Embedded);

        let mut names = HashSet::new();
        let mut keys = HashSet::new();
        // The platform table in platform.rs calls alpine4k (Annapurna AL-212, a 32-bit Cortex-A15 SoC) arm64.
        // Synology's own table says ARMv7, which the list follows. Reported to the maintainers; do not "fix" the data.
        const KNOWN_PLATFORM_TABLE_DISAGREEMENTS: [&str; 1] = ["alpine4k"];

        for m in db.models() {
            let id = &m.model;
            assert!(names.insert(id.clone()), "duplicate model {id}");
            assert!(keys.insert(normalize_model(id)), "{id}: normalised name collides with another model");
            assert!(!m.model.trim().is_empty() && m.model == m.model.trim(), "{id}: bad model name");
            assert!(m.series.chars().all(|c| c.is_ascii_uppercase()) && !m.series.is_empty(), "{id}: series {:?}", m.series);
            assert!(id.to_ascii_uppercase().starts_with(&m.series), "{id}: series {} is not the model prefix", m.series);
            assert!(
                m.platform.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !m.platform.is_empty(),
                "{id}: platform {:?}",
                m.platform
            );
            assert!(!m.vendor.is_empty(), "{id}: vendor");
            if let Some(y) = m.year {
                assert!((2000..=2100).contains(&y), "{id}: year {y}");
            }
            if let Some(r) = m.ram_mb_stock {
                assert!(r >= 64, "{id}: ram");
            }
            assert!(matches!(m.source.as_deref(), Some("kb.synology.com") | Some("spksrc-wiki")), "{id}: source {:?}", m.source);
            // CPU vendor bucket follows vendor/arch
            let want = match (m.vendor.as_str(), m.arch) {
                ("Intel", _) => CpuVendor::Intel,
                ("AMD", _) => CpuVendor::Amd,
                (_, ModelArch::Arm64 | ModelArch::Armhf | ModelArch::Armel) => CpuVendor::Arm,
                _ => CpuVendor::Other,
            };
            assert_eq!(m.cpu_vendor, want, "{id}: cpu_vendor");
            // x86 vendors imply x86 archs
            if matches!(m.cpu_vendor, CpuVendor::Amd) {
                assert_eq!(m.arch, ModelArch::Amd64, "{id}: AMD must be amd64");
            }
            // boot method follows the architecture class
            if m.boot == Boot::Dom {
                assert_eq!(m.arch, ModelArch::Amd64, "{id}: DOM boot is x86 only");
            }
            if m.arch == ModelArch::Amd64 {
                assert!(m.boot != Boot::Uboot, "{id}: x86 does not use U-Boot");
            }

            // arch consistent with the platform table in platform.rs
            let info = platform::classify(&format!("synology_{}_x", m.platform), "");
            let known = info.soc != "unknown";
            if known && !KNOWN_PLATFORM_TABLE_DISAGREEMENTS.contains(&m.platform.as_str()) {
                if info.arch == Arch::Unsupported {
                    assert!(
                        matches!(m.arch, ModelArch::Armel | ModelArch::Ppc | ModelArch::I386 | ModelArch::Armhf),
                        "{id}: platform {} is a dead end but arch is {}",
                        m.platform,
                        m.arch.debian()
                    );
                    assert_eq!(m.status, Status::Unsupported, "{id}: platform {} is unsupported in platform.rs", m.platform);
                } else {
                    assert_eq!(m.arch.debian(), info.arch.debian(), "{id}: arch vs platform.rs for {}", m.platform);
                    assert_ne!(m.status, Status::Unsupported, "{id}: platform {} is supported in platform.rs", m.platform);
                }
            }
            // dead-end architectures can only be unsupported
            if matches!(m.arch, ModelArch::Armel | ModelArch::Ppc | ModelArch::I386) {
                assert_eq!(m.status, Status::Unsupported, "{id}: {} cannot be anything but unsupported", m.arch.debian());
            }
            if m.status == Status::Unsupported {
                assert!(m.notes.as_deref().is_some_and(|n| n.starts_with("Unsupported:")), "{id}: unsupported needs a reason in notes");
            }

            // status <-> evidence
            assert_ne!(m.status, Status::Verified, "{id}: nothing may be verified yet");
            match m.status {
                Status::Untested | Status::Unsupported => {
                    assert!(m.evidence.is_empty(), "{id}: {} must have no evidence", m.status.label())
                }
                Status::Community => assert!(!m.evidence.is_empty(), "{id}: community needs evidence"),
                Status::KnownIssues => assert!(
                    m.evidence.iter().any(|e| e.result != EvidenceResult::Success),
                    "{id}: known-issues needs a partial/fail report"
                ),
                Status::Verified => unreachable!(),
            }
            for e in &m.evidence {
                assert!(!e.reporter.trim().is_empty(), "{id}: reporter");
                if let Some(u) = &e.log_url {
                    assert!(u.starts_with("https://"), "{id}: log_url must be https");
                }
                if e.kind == EvidenceKind::Synofurb {
                    assert!(e.synofurb_version.is_some() && e.date.is_some(), "{id}: synofurb evidence needs version and date");
                }
                if let Some(d) = &e.date {
                    assert!(d.len() >= 7 && d.as_bytes()[4] == b'-', "{id}: date {d:?}");
                }
            }
        }

        // Only the DS923+ is community, and its evidence is a third-party report, labelled as such.
        let community: Vec<&str> = db.models().iter().filter(|m| m.status == Status::Community).map(|m| m.model.as_str()).collect();
        assert_eq!(community, ["DS923+"]);
        let ds923 = db.lookup("DS923+").unwrap();
        assert_eq!(ds923.evidence.len(), 1);
        assert_eq!(ds923.evidence[0].reporter, "rylos/synodeb project");
        assert_eq!(ds923.evidence[0].kind, EvidenceKind::ThirdParty);
        assert_eq!(ds923.evidence[0].result, EvidenceResult::Success);
        assert!(ds923.evidence[0].notes.contains("not by synofurb"));

        // Spot checks that the table is what Synology prints.
        let ds918 = db.lookup("DS918+").unwrap();
        assert_eq!(
            (ds918.platform.as_str(), ds918.arch, ds918.cpu_vendor, ds918.ram_mb_stock),
            ("apollolake", ModelArch::Amd64, CpuVendor::Intel, Some(4096))
        );
        assert_eq!(db.lookup("DS1621+").unwrap().cpu.as_deref(), Some("AMD Ryzen V1500B"));
        assert_eq!(db.lookup("DS218j").unwrap().arch, ModelArch::Armhf);
        assert_eq!(db.lookup("DS118").unwrap().arch, ModelArch::Arm64);
        assert_eq!(db.lookup("DS213+").unwrap().status, Status::Unsupported);
    }

    #[test]
    fn schema_file_matches_the_rust_enums() {
        let schema: serde_json::Value = serde_json::from_str(include_str!("../../../compat/schema.json")).expect("schema.json is JSON");
        let enum_of = |path: &[&str]| -> Vec<String> {
            let mut v = &schema["$defs"];
            for p in path {
                v = &v[*p];
            }
            v["enum"].as_array().unwrap_or_else(|| panic!("no enum at {path:?}")).iter().map(|x| x.as_str().unwrap().to_string()).collect()
        };
        let statuses: Vec<String> = Status::ALL.iter().map(|s| serde_json::to_value(s).unwrap().as_str().unwrap().to_string()).collect();
        assert_eq!(enum_of(&["model", "properties", "status"]), statuses);
        let archs: Vec<String> = [ModelArch::Amd64, ModelArch::Arm64, ModelArch::Armhf, ModelArch::Armel, ModelArch::Ppc, ModelArch::I386]
            .iter()
            .map(|a| a.debian().to_string())
            .collect();
        assert_eq!(enum_of(&["model", "properties", "arch"]), archs);
        assert_eq!(schema["properties"]["schema_version"]["const"], SUPPORTED_SCHEMA_VERSION);
    }

    #[test]
    fn counts_by_status_add_up() {
        let db = CompatDb::embedded();
        let total: usize = db.count_by_status().iter().map(|(_, n)| n).sum();
        assert_eq!(total, db.len());
    }

    // --- refresh ---------------------------------------------------------------------------------------------

    fn sample_json(updated: &str, status: &str) -> String {
        let evidence = if status == "community" {
            r#"[{"date":"2026-01-02","reporter":"someone","kind":"synofurb","synofurb_version":"0.1.0","dsm_version":"7.2","result":"success","notes":"ok","log_url":"https://example.com/log"}]"#
        } else {
            "[]"
        };
        format!(
            r#"{{"schema_version":1,"updated":"{updated}","models":[{{"model":"DS918+","series":"DS","platform":"apollolake","vendor":"Intel","cpu_vendor":"intel","arch":"amd64","boot":"dom","status":"{status}","evidence":{evidence}}}]}}"#
        )
    }

    #[test]
    fn refresh_downloads_then_uses_etag_then_falls_back() {
        let tmp = TempDir::new("compat");
        let body = Arc::new(Mutex::new(sample_json("2099-01-01", "community")));
        let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(vec![]));
        let (b2, s2) = (body.clone(), seen.clone());
        let srv = serve(move |_path, headers| {
            let inm = headers.get("if-none-match").cloned();
            s2.lock().unwrap().push(inm.clone());
            if inm.as_deref() == Some("\"v1\"") {
                Reply::status(304)
            } else {
                Reply::ok(b2.lock().unwrap().clone()).header("ETag", "\"v1\"")
            }
        });
        let url = format!("{}/compat/models.json", srv.base);

        // 1. first fetch
        let out = CompatDb::refresh(tmp.path(), &url).unwrap();
        assert_eq!(out, RefreshOutcome::Updated { models: 1, updated: "2099-01-01".into() });
        let cached = CompatDb::load_cached(tmp.path()).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached.origin(), DbOrigin::Cache);
        assert_eq!(cached.lookup("ds918plus").unwrap().status, Status::Community);

        // 2. second fetch sends the ETag and gets 304
        assert_eq!(CompatDb::refresh(tmp.path(), &url).unwrap(), RefreshOutcome::NotModified);
        assert_eq!(seen.lock().unwrap().as_slice(), [None, Some("\"v1\"".to_string())]);

        // 3. a newer cache wins over the embedded list; an old one does not
        assert_eq!(CompatDb::load_best(tmp.path()).len(), 1);
        std::fs::write(tmp.path().join(CACHE_FILE), sample_json("2000-01-01", "untested")).unwrap();
        assert!(CompatDb::load_best(tmp.path()).len() >= MIN_MODELS, "older cache must lose to the embedded list");

        // 4. corrupt body from the server is rejected and the cache is not touched
        *body.lock().unwrap() = "{ this is not json".into();
        let good = sample_json("2099-02-02", "untested");
        std::fs::write(tmp.path().join(CACHE_FILE), &good).unwrap();
        std::fs::remove_file(tmp.path().join(ETAG_FILE)).unwrap();
        let out = CompatDb::refresh(tmp.path(), &url).unwrap();
        assert!(matches!(out, RefreshOutcome::Rejected { .. }), "{out:?}");
        assert_eq!(std::fs::read_to_string(tmp.path().join(CACHE_FILE)).unwrap(), good);
        assert_eq!(CompatDb::load_cached(tmp.path()).unwrap().updated(), "2099-02-02");
    }

    #[test]
    fn refresh_rejects_bad_bodies() {
        let big = vec![b' '; (MAX_DOWNLOAD_BYTES as usize) + 10];
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty list", br#"{"schema_version":1,"updated":"2099-01-01","models":[]}"#.to_vec()),
            ("future schema", sample_json("2099-01-01", "untested").replace("\"schema_version\":1", "\"schema_version\":2").into_bytes()),
            ("community without evidence", sample_json("2099-01-01", "untested").replace("\"untested\"", "\"community\"").into_bytes()),
            ("wrong status word", sample_json("2099-01-01", "untested").replace("\"untested\"", "\"great\"").into_bytes()),
            ("html error page", b"<html>502 Bad Gateway</html>".to_vec()),
            ("oversize", big),
        ];
        for (name, body) in cases {
            let tmp = TempDir::new("compat-bad");
            let srv = serve(move |_, _| Reply::ok(body.clone()));
            let out = CompatDb::refresh(tmp.path(), &format!("{}/m.json", srv.base)).unwrap();
            assert!(matches!(out, RefreshOutcome::Rejected { .. }), "{name}: {out:?}");
            assert!(CompatDb::load_cached(tmp.path()).is_err(), "{name}: nothing may be cached");
        }
    }

    #[test]
    fn refresh_offline_and_http_errors_fall_back() {
        let tmp = TempDir::new("compat-off");
        // nothing listens here
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let out = CompatDb::refresh(tmp.path(), &format!("http://127.0.0.1:{port}/m.json")).unwrap();
        assert!(matches!(out, RefreshOutcome::Offline { .. }), "{out:?}");

        let srv = serve(|_, _| Reply::status(503));
        let out = CompatDb::refresh(tmp.path(), &format!("{}/m.json", srv.base)).unwrap();
        assert!(matches!(out, RefreshOutcome::Offline { .. }), "{out:?}");

        // with no cache the embedded list is what load_best returns
        assert!(CompatDb::load_cached(tmp.path()).is_err());
        assert_eq!(CompatDb::load_best(tmp.path()).origin(), DbOrigin::Embedded);
    }

    #[test]
    fn refresh_refuses_plain_http_to_remote_hosts() {
        let tmp = TempDir::new("compat-http");
        assert!(CompatDb::refresh(tmp.path(), "http://example.com/models.json").is_err());
        assert!(CompatDb::refresh(tmp.path(), "ftp://example.com/models.json").is_err());
        assert!(DEFAULT_URL.starts_with("https://"));
    }
}
