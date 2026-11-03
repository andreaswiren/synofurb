//! Day-two management of a converted unit: basic health and updates of the management web app (`synofurb-mgmt`) that runs on the
//! appliance. This is the engine behind `synofurb-cli manage ...` and the desktop app's Manage screen.
//!
//! The NAS side is `apps/mgmt-plane/synofurb_mgmt.py`; the contract between the two is in `docs/MANAGE.md`:
//!
//! * `python3 /opt/synofurb-mgmt/synofurb_mgmt.py health --json` prints one JSON document ([`Health`], parsed leniently).
//! * `... update --from DIR --json` swaps in the files of `DIR` (a staged copy plus `MANIFEST.sha256`), prints one NDJSON progress line
//!   per step (`{"step","status","message"}`), exits non-zero on failure and rolls back by itself. `... update --rollback` goes back
//!   to the previous files.
//!
//! Everything here only reads the system or touches `/opt/synofurb-mgmt`, the systemd unit of that service and temporary files below
//! `/tmp`. It never touches disks, partitions, arrays or the boot flash. All shell access goes through [`RemoteShell`] /
//! [`Privileged`], so the flows are tested with a scripted fake NAS and no network. Secrets travel on stdin only (inherited from
//! [`Privileged`]); no command built here contains a password.

use crate::config::{random_token, sh_quote};
use crate::logs::{Event, Level, LogLine, Logger, Source};
use crate::remote::{ExecResult, Privileged, RemoteShell};
use anyhow::{Context, Result, anyhow, bail};
use serde::de::Deserializer;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Where the management plane lives on the appliance.
pub const MGMT_DIR: &str = "/opt/synofurb-mgmt";
pub const MGMT_SCRIPT: &str = "/opt/synofurb-mgmt/synofurb_mgmt.py";
/// The web UI listens here (HTTPS, self-signed).
pub const WEB_PORT: u16 = 8443;
/// Name of the checksum file inside a staged update directory (`sha256sum -c` format).
pub const MANIFEST_NAME: &str = "MANIFEST.sha256";
/// The `schema` the health document is expected to carry. Newer schemas are still parsed (fields are optional).
pub const HEALTH_SCHEMA: u32 = 1;

/// Files of the management plane that are shipped, in this order. Anything else in a directory is ignored.
/// `(name, required, executable)`
const MGMT_FILES: &[(&str, bool, bool)] = &[
    ("synofurb_mgmt.py", true, true),
    ("VERSION", true, false),
    ("README.md", true, false),
    ("synofurb-mgmt.service", false, false),
    // the add-on framework and the add-on definitions packed into one JSON file (tools/make-update-bundle.py writes the same pair);
    // optional so a bundle from before add-ons still verifies
    ("synofurb_addons.py", false, false),
    ("addons.pack.json", false, false),
    // certificates (lego) and the HAProxy front (docs/CERTIFICATES.md); optional like the add-on framework
    ("synofurb_edge.py", false, false),
];

const EMBEDDED: &[(&str, &str)] = &[
    ("synofurb_mgmt.py", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/synofurb_mgmt.py"))),
    ("VERSION", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/VERSION"))),
    ("README.md", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/README.md"))),
    ("synofurb-mgmt.service", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/synofurb-mgmt.service"))),
    ("synofurb_addons.py", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/synofurb_addons.py"))),
    ("addons.pack.json", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/addons.pack.json"))),
    ("synofurb_edge.py", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/synofurb_edge.py"))),
];

/// Per-file size limit when reading a bundle directory.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The URL of the appliance's web UI. IPv6 literals get brackets.
pub fn web_url(host: &str) -> String {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    if h.contains(':') { format!("https://[{h}]:{WEB_PORT}") } else { format!("https://{h}:{WEB_PORT}") }
}

// ---------------------------------------------------------------------------------------------- errors

/// Why a management call failed. The variants let the UI say something useful ("not installed" vs "too old" vs "unreachable").
#[derive(Debug, thiserror::Error)]
pub enum ManageError {
    #[error("the management plane is not installed on this unit ({MGMT_SCRIPT} is missing)")]
    NotInstalled,
    #[error(
        "the management plane on this unit is too old to report health{}; run `manage update` to bring it up to date",
        .installed_version.as_deref().map(|v| format!(" (version {v})")).unwrap_or_default()
    )]
    HealthUnsupported { installed_version: Option<String> },
    #[error("the management plane answered, but not with a health report: {0}")]
    BadOutput(String),
    #[error("the health command failed (exit {code}): {stderr}")]
    Command { code: u32, stderr: String },
    #[error("{0:#}")]
    Transport(anyhow::Error),
}

// ---------------------------------------------------------------------------------------------- lenient serde helpers

fn de_value<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
    Value::deserialize(d)
}

fn num_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64)),
        Value::String(s) => s.trim().parse::<u64>().ok().or_else(|| s.trim().parse::<f64>().ok().filter(|f| *f >= 0.0).map(|f| f as u64)),
        _ => None,
    }
}

fn num_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().trim_end_matches('%').parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

fn de_opt_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(num_u64(&de_value(d)?))
}
fn de_opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(num_f64(&de_value(d)?))
}
fn de_opt_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    Ok(num_u64(&de_value(d)?).map(|n| n.min(u32::MAX as u64) as u32))
}
fn de_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match de_value(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    })
}
fn de_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(bool_of(&de_value(d)?).unwrap_or(false))
}
fn de_opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(bool_of(&de_value(d)?))
}
fn de_f64_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    Ok(match de_value(d)? {
        Value::Array(a) => a.iter().filter_map(num_f64).collect(),
        _ => vec![],
    })
}
fn de_string_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match de_value(d)? {
        Value::Array(a) => a.into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Value::String(s) if !s.is_empty() => vec![s],
        _ => vec![],
    })
}

/// `true`/`false`, 0/1 and the words systemd and people use.
fn bool_of(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "active" | "running" | "up" | "on" | "enabled" => Some(true),
            "false" | "no" | "0" | "inactive" | "failed" | "dead" | "down" | "off" | "stopped" | "disabled" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------- health document

/// Overall verdict of a health report. Ordered by severity (`Unknown` is lowest so it never hides a known state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Overall {
    #[default]
    Unknown,
    Ok,
    Warn,
    Crit,
}

impl Overall {
    pub fn as_str(self) -> &'static str {
        match self {
            Overall::Unknown => "unknown",
            Overall::Ok => "ok",
            Overall::Warn => "warn",
            Overall::Crit => "crit",
        }
    }
    /// Short upper-case tag for badges ("OK", "WARN", "CRIT", "?").
    pub fn tag(self) -> &'static str {
        match self {
            Overall::Unknown => "?",
            Overall::Ok => "OK",
            Overall::Warn => "WARN",
            Overall::Crit => "CRIT",
        }
    }
    /// Lenient: case-insensitive, with a few synonyms. Anything else is `Unknown`.
    pub fn parse(s: &str) -> Overall {
        match s.trim().to_ascii_lowercase().as_str() {
            "ok" | "good" | "healthy" | "green" | "pass" | "passed" => Overall::Ok,
            "warn" | "warning" | "degraded" | "yellow" | "amber" => Overall::Warn,
            "crit" | "critical" | "error" | "fail" | "failed" | "red" => Overall::Crit,
            _ => Overall::Unknown,
        }
    }
}

impl Serialize for Overall {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Overall {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Overall::parse(&de_string(d)?))
    }
}

/// SMART self-assessment of one disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Smart {
    Passed,
    Failed,
    #[default]
    Unknown,
}

impl Smart {
    pub fn as_str(self) -> &'static str {
        match self {
            Smart::Passed => "passed",
            Smart::Failed => "failed",
            Smart::Unknown => "unknown",
        }
    }
}
impl Serialize for Smart {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Smart {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match de_string(d)?.trim().to_ascii_lowercase().as_str() {
            "passed" | "pass" | "ok" | "good" => Smart::Passed,
            "failed" | "fail" | "bad" | "failing" => Smart::Failed,
            _ => Smart::Unknown,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    /// Bytes.
    #[serde(default, deserialize_with = "de_opt_u64")]
    pub total: Option<u64>,
    /// Bytes available without swapping (MemAvailable).
    #[serde(default, deserialize_with = "de_opt_u64")]
    pub avail: Option<u64>,
}

impl Memory {
    /// Percent of memory that is NOT available.
    pub fn used_pct(&self) -> Option<f64> {
        match (self.total, self.avail) {
            (Some(t), Some(a)) if t > 0 => Some(((t.saturating_sub(a)) as f64 / t as f64 * 100.0).clamp(0.0, 100.0)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RootFs {
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub used_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArrayHealth {
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_string")]
    pub level: String,
    #[serde(default, deserialize_with = "de_string")]
    pub state: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub degraded: bool,
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub sync_pct: Option<f64>,
    /// `resync`, `recovery`, `reshape`, `check`, `repair` or empty.
    #[serde(default, deserialize_with = "de_string")]
    pub sync_action: String,
}

impl ArrayHealth {
    /// A rebuild-type operation is running (a scheduled `check` or `idle` is not one).
    pub fn rebuilding(&self) -> bool {
        matches!(self.sync_action.trim().to_ascii_lowercase().as_str(), "resync" | "recover" | "recovery" | "reshape" | "repair")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskHealth {
    #[serde(default, deserialize_with = "de_string")]
    pub dev: String,
    #[serde(default, deserialize_with = "de_string")]
    pub model: String,
    /// Bytes.
    #[serde(default, deserialize_with = "de_opt_u64")]
    pub size: Option<u64>,
    #[serde(default)]
    pub smart: Smart,
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub temp_c: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumeHealth {
    #[serde(default, deserialize_with = "de_string")]
    pub mount: String,
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub used_pct: Option<f64>,
    #[serde(default, deserialize_with = "de_string")]
    pub fstype: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceHealth {
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// `None` when the NAS did not say.
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BootInfo {
    #[serde(default, deserialize_with = "de_string")]
    pub default: String,
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub flash_present: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdatesInfo {
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub apt_pending: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_u32")]
    pub security_pending: Option<u32>,
}

/// The document printed by `synofurb_mgmt.py health --json`. Only `schema`, `version` and `overall` are required; everything else
/// is optional and a malformed section is dropped (with a note in `parse_notes`) rather than failing the whole report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub schema: u32,
    /// Version of the management plane running on the NAS.
    pub version: String,
    pub overall: Overall,
    #[serde(default, deserialize_with = "de_string")]
    pub hostname: String,
    #[serde(default, deserialize_with = "de_opt_u64")]
    pub uptime_s: Option<u64>,
    #[serde(default, deserialize_with = "de_f64_vec")]
    pub load: Vec<f64>,
    #[serde(default)]
    pub mem: Memory,
    #[serde(default, deserialize_with = "de_string")]
    pub os: String,
    #[serde(default, deserialize_with = "de_string")]
    pub kernel: String,
    #[serde(default)]
    pub root_fs: RootFs,
    #[serde(default)]
    pub arrays: Vec<ArrayHealth>,
    #[serde(default)]
    pub disks: Vec<DiskHealth>,
    #[serde(default)]
    pub volumes: Vec<VolumeHealth>,
    #[serde(default)]
    pub services: Vec<ServiceHealth>,
    #[serde(default)]
    pub boot: BootInfo,
    #[serde(default)]
    pub updates: UpdatesInfo,
    /// The NAS's own warning sentences.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub warnings: Vec<String>,
    /// What the parser had to skip (a section of the wrong type, an element that did not fit). Not part of the NAS document.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parse_notes: Vec<String>,
}

fn section<T: serde::de::DeserializeOwned + Default>(obj: &serde_json::Map<String, Value>, key: &str, notes: &mut Vec<String>) -> T {
    match obj.get(key) {
        None | Some(Value::Null) => T::default(),
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_else(|e| {
            notes.push(format!("ignored malformed \"{key}\": {e}"));
            T::default()
        }),
    }
}

fn list<T: serde::de::DeserializeOwned>(obj: &serde_json::Map<String, Value>, key: &str, notes: &mut Vec<String>) -> Vec<T> {
    match obj.get(key) {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a
            .iter()
            .enumerate()
            .filter_map(|(i, v)| match serde_json::from_value::<T>(v.clone()) {
                Ok(t) => Some(t),
                Err(e) => {
                    notes.push(format!("ignored \"{key}\"[{i}]: {e}"));
                    None
                }
            })
            .collect(),
        Some(_) => {
            notes.push(format!("ignored \"{key}\": not a list"));
            vec![]
        }
    }
}

/// Parse a health document. Required: a JSON object with `schema` (number >= 1), `version` (non-empty) and `overall`.
pub fn parse_health(text: &str) -> Result<Health, ManageError> {
    let t = text.trim();
    // tolerate noise before the document (a warning printed by Python, a login banner): take the last line that is a JSON object
    let v: Value = match serde_json::from_str(t) {
        Ok(v) => v,
        Err(first) => t
            .lines()
            .rev()
            .map(str::trim)
            .filter(|l| l.starts_with('{'))
            .find_map(|l| serde_json::from_str(l).ok())
            .ok_or_else(|| ManageError::BadOutput(format!("not valid JSON ({first})")))?,
    };
    let obj = v.as_object().ok_or_else(|| ManageError::BadOutput("the document is not a JSON object".into()))?;
    let schema = obj.get("schema").and_then(num_u64).filter(|s| *s >= 1);
    let Some(schema) = schema else {
        return Err(ManageError::BadOutput("missing \"schema\"".into()));
    };
    let version = obj.get("version").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let Some(version) = version else {
        return Err(ManageError::BadOutput("missing \"version\"".into()));
    };
    let Some(overall) = obj.get("overall").and_then(Value::as_str) else {
        return Err(ManageError::BadOutput("missing \"overall\"".into()));
    };
    let mut notes = vec![];
    if schema > HEALTH_SCHEMA as u64 {
        notes.push(format!("schema {schema} is newer than this program understands ({HEALTH_SCHEMA}); showing what it knows"));
    }
    let n = &mut notes;
    let h = Health {
        schema: schema.min(u32::MAX as u64) as u32,
        version: version.to_string(),
        overall: Overall::parse(overall),
        hostname: obj.get("hostname").and_then(Value::as_str).unwrap_or_default().to_string(),
        uptime_s: obj.get("uptime_s").and_then(num_u64),
        load: obj.get("load").and_then(Value::as_array).map(|a| a.iter().filter_map(num_f64).collect()).unwrap_or_default(),
        mem: section(obj, "mem", n),
        os: obj.get("os").and_then(Value::as_str).unwrap_or_default().to_string(),
        kernel: obj.get("kernel").and_then(Value::as_str).unwrap_or_default().to_string(),
        root_fs: section(obj, "root_fs", n),
        arrays: list(obj, "arrays", n),
        disks: list(obj, "disks", n),
        volumes: list(obj, "volumes", n),
        services: list(obj, "services", n),
        boot: section(obj, "boot", n),
        updates: section(obj, "updates", n),
        warnings: obj
            .get("warnings")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        parse_notes: vec![],
    };
    Ok(Health { parse_notes: notes, ..h })
}

/// Turn the result of `health --json` into a [`Health`] or a precise error. Pure, so it is tested without a NAS.
pub fn interpret_health_result(r: &ExecResult) -> Result<Health, ManageError> {
    let parse_error = match parse_health(&r.stdout) {
        Ok(h) => return Ok(h),
        Err(e) => e,
    };
    let err = r.stderr.to_ascii_lowercase();
    if r.code == 127 && err.contains("python3") {
        return Err(ManageError::Command { code: r.code, stderr: r.stderr.trim().to_string() });
    }
    if r.code == 127 || err.contains("no such file") || err.contains("can't open file") {
        return Err(ManageError::NotInstalled);
    }
    if looks_like_unknown_subcommand(r) {
        return Err(ManageError::HealthUnsupported { installed_version: None });
    }
    if r.stdout.trim().is_empty() {
        return Err(ManageError::Command { code: r.code, stderr: r.stderr.trim().to_string() });
    }
    Err(parse_error)
}

/// argparse (exit 2) complaining about a subcommand or flag it does not have: an older management plane.
fn looks_like_unknown_subcommand(r: &ExecResult) -> bool {
    let e = r.stderr.to_ascii_lowercase();
    r.code == 2
        && (e.contains("invalid choice") || e.contains("unrecognized arguments") || e.contains("usage:") || e.contains("unknown option"))
}

/// Ask the appliance for its health (`health --json`). Runs as root when it can (SMART needs it); a user without root and without a
/// sudo password gets the part of the report that needs no privileges (the management plane documents that `health` works that way).
pub fn fetch_health(p: &mut Privileged<'_>) -> Result<Health, ManageError> {
    let cmd = format!("python3 {MGMT_SCRIPT} health --json");
    let r = if p.is_root || p.sudo_password.is_some() { p.run(&cmd) } else { p.sh.exec(&cmd, None) }.map_err(ManageError::Transport)?;
    interpret_health_result(&r)
}

/// Changing the management plane needs root, either directly or through sudo.
fn need_root(p: &Privileged<'_>) -> Result<()> {
    if p.is_root || p.sudo_password.is_some() {
        Ok(())
    } else {
        bail!(
            "changing the management plane needs root: log in as root or give the sudo password (not root and no sudo password available)"
        )
    }
}

// ---------------------------------------------------------------------------------------------- summary for the UI

/// One line of the problem list. `level` is `Warn` or `Crit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bullet {
    pub level: Overall,
    pub text: String,
}

/// A terse, derived view of a [`Health`] for screens with little room: one tag, a handful of one-sentence bullets and counts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthSummary {
    /// The verdict shown to the user: the worse of what the NAS reported and what the numbers say.
    pub overall: Overall,
    /// What the NAS reported itself.
    pub reported: Overall,
    /// "Healthy", "Needs attention" or "Problem" (or "Unknown").
    pub headline: String,
    /// Problems, worst first, each one short sentence; at most [`MAX_BULLETS`] (the last may say "and N more").
    pub bullets: Vec<Bullet>,
    /// Number of problems found (can exceed `bullets.len()`).
    pub problems: usize,
    pub degraded_arrays: usize,
    pub rebuilding_arrays: usize,
    pub failed_disks: usize,
    pub services_down: usize,
    pub volumes_nearly_full: usize,
    pub security_updates: u32,
    pub pending_updates: u32,
}

/// Longest bullet list handed to a screen.
pub const MAX_BULLETS: usize = 5;
const MAX_BULLET_CHARS: usize = 96;
const WARN_FULL_PCT: f64 = 85.0;
const CRIT_FULL_PCT: f64 = 95.0;
const WARN_TEMP_C: f64 = 60.0;
const CRIT_TEMP_C: f64 = 70.0;
const WARN_MEM_USED_PCT: f64 = 95.0;

fn short(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= MAX_BULLET_CHARS {
        return s;
    }
    let cut: String = s.chars().take(MAX_BULLET_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

fn pct(p: f64) -> String {
    format!("{}%", p.round() as i64)
}

impl Health {
    /// Derive the UI summary. Rules (documented in docs/MANAGE.md): a degraded or inactive array, a failed SMART assessment or a
    /// file system at 95 % or more is `crit`; a service that is down, a rebuild in progress, a file system at 85 % or more, a hot disk,
    /// nearly no free memory or pending security updates is `warn`. The NAS's own `overall` can only raise the verdict, never lower it.
    pub fn summary(&self) -> HealthSummary {
        let mut found: Vec<Bullet> = vec![];
        macro_rules! add {
            ($level:expr, $text:expr) => {{
                let text: String = $text;
                found.push(Bullet { level: $level, text: short(&text) });
            }};
        }
        let (mut degraded, mut rebuilding, mut failed_disks, mut full) = (0usize, 0usize, 0usize, 0usize);

        for a in &self.arrays {
            let name = if a.name.is_empty() { "an array" } else { a.name.as_str() };
            let state = a.state.to_ascii_lowercase();
            let busy = if a.rebuilding() {
                a.sync_pct.map(|p| format!(", {} {}", a.sync_action.trim().to_ascii_lowercase(), pct(p))).unwrap_or_default()
            } else {
                String::new()
            };
            if a.degraded {
                degraded += 1;
                if a.rebuilding() {
                    rebuilding += 1;
                }
                add!(Overall::Crit, format!("{name} is degraded{busy}."));
            } else if state.contains("inactive") || state.contains("fail") {
                // same severity the NAS's own verdict gives an inactive array: not assembled, so not serving data
                add!(Overall::Warn, format!("{name} is {}.", a.state.trim()));
            } else if a.rebuilding() {
                rebuilding += 1;
                add!(Overall::Warn, format!("{name} is rebuilding{busy}."));
            }
        }
        for d in &self.disks {
            let name = if d.dev.is_empty() { "A disk" } else { d.dev.trim_start_matches("/dev/") };
            if d.smart == Smart::Failed {
                failed_disks += 1;
                add!(Overall::Crit, format!("{name} failed its SMART check; replace it."));
            } else if let Some(t) = d.temp_c.filter(|t| *t >= WARN_TEMP_C) {
                let lvl = if t >= CRIT_TEMP_C { Overall::Crit } else { Overall::Warn };
                add!(lvl, format!("{name} is hot ({} °C).", t.round() as i64));
            }
        }
        let down: Vec<&str> = self.services.iter().filter(|s| s.active == Some(false)).map(|s| s.name.as_str()).collect();
        match down.len() {
            0 => {}
            1 => add!(Overall::Warn, format!("Service {} is not running.", down[0])),
            n => {
                let shown = down.iter().take(3).copied().collect::<Vec<_>>().join(", ");
                let more = if n > 3 { format!(" and {} more", n - 3) } else { String::new() };
                add!(Overall::Warn, format!("{n} services are not running: {shown}{more}."));
            }
        }
        let mut fs: Vec<(&str, f64)> = self.volumes.iter().filter_map(|v| v.used_pct.map(|p| (v.mount.as_str(), p))).collect();
        if let Some(p) = self.root_fs.used_pct {
            fs.push(("The system disk", p));
        }
        for (name, p) in fs {
            if p >= WARN_FULL_PCT {
                full += 1;
                let lvl = if p >= CRIT_FULL_PCT { Overall::Crit } else { Overall::Warn };
                let name = if name.is_empty() { "A volume" } else { name };
                add!(lvl, format!("{name} is {} full.", pct(p)));
            }
        }
        if let Some(u) = self.mem.used_pct().filter(|u| *u >= WARN_MEM_USED_PCT) {
            add!(Overall::Warn, format!("Memory is almost full ({} free).", pct(100.0 - u)));
        }
        let security = self.updates.security_pending.unwrap_or(0);
        if security > 0 {
            add!(Overall::Warn, format!("{security} security update{} waiting.", if security == 1 { " is" } else { "s are" }));
        }

        let structural = found.iter().map(|b| b.level).max().unwrap_or(Overall::Ok);
        // The NAS saw something these numbers do not explain: show its own sentences, they are the only explanation we have.
        if self.overall > structural {
            for w in self.warnings.iter().filter(|w| !w.trim().is_empty()) {
                add!(self.overall, w.clone());
            }
            if self.warnings.iter().all(|w| w.trim().is_empty()) {
                add!(self.overall, format!("The NAS reports status \"{}\".", self.overall.as_str()));
            }
        } else if self.overall == Overall::Unknown {
            add!(Overall::Warn, "The NAS sent a status this program does not know.".to_string());
        }
        let overall = found.iter().map(|b| b.level).max().unwrap_or(Overall::Ok);

        // worst first, original order within a level
        found.sort_by_key(|b| std::cmp::Reverse(b.level));
        let problems = found.len();
        if problems > MAX_BULLETS {
            let rest = problems - (MAX_BULLETS - 1);
            let level = found[MAX_BULLETS - 1..].iter().map(|b| b.level).max().unwrap_or(Overall::Warn);
            found.truncate(MAX_BULLETS - 1);
            found.push(Bullet { level, text: format!("And {rest} more.") });
        }
        HealthSummary {
            overall,
            reported: self.overall,
            headline: match overall {
                Overall::Ok => "Healthy",
                Overall::Warn => "Needs attention",
                Overall::Crit => "Problem",
                Overall::Unknown => "Unknown",
            }
            .to_string(),
            bullets: found,
            problems,
            degraded_arrays: degraded,
            rebuilding_arrays: rebuilding,
            failed_disks,
            services_down: down.len(),
            volumes_nearly_full: full,
            security_updates: security,
            pending_updates: self.updates.apt_pending.unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------------------------- installed version / compare

/// What is on the appliance, read without needing the management plane to be healthy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct InstalledInfo {
    /// `synofurb_mgmt.py` exists.
    pub present: bool,
    /// From `VERSION`, else from the script's built-in default.
    pub version: Option<String>,
    /// SHA-256 of the installed script.
    pub script_sha256: Option<String>,
}

const READ_INSTALLED: &str = r#"S=/opt/synofurb-mgmt/synofurb_mgmt.py
if [ -f "$S" ]; then
  echo "present=1"
  v=$(head -n 1 /opt/synofurb-mgmt/VERSION 2>/dev/null | tr -d '[:space:]')
  [ -n "$v" ] || v=$(sed -n 's/^DEFAULT_VERSION = "\([^"]*\)".*/\1/p' "$S" 2>/dev/null | head -n 1)
  echo "version=$v"
  echo "sha256=$(sha256sum "$S" 2>/dev/null | cut -d' ' -f1)"
else
  echo "present=0"
fi"#;

pub fn parse_installed(stdout: &str) -> InstalledInfo {
    let mut i = InstalledInfo::default();
    for l in stdout.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "present" => i.present = v == "1",
            "version" if valid_version_text(v) => i.version = Some(v.to_string()),
            "sha256" if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) => i.script_sha256 = Some(v.to_ascii_lowercase()),
            _ => {}
        }
    }
    i
}

/// Read the installed version and script hash (no privileges needed).
pub fn read_installed(sh: &mut dyn RemoteShell) -> Result<InstalledInfo> {
    let r = sh.exec(&format!("/bin/sh -c {}", sh_quote(READ_INSTALLED)), None)?;
    if !r.ok() && r.stdout.trim().is_empty() {
        bail!("could not read the installed management plane (exit {}): {}", r.code, r.stderr.trim());
    }
    Ok(parse_installed(&r.stdout))
}

fn valid_version_text(v: &str) -> bool {
    !v.is_empty() && v.len() <= 64 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
}

/// How the version on the NAS relates to the one shipped with this program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionRelation {
    Same,
    /// The NAS is older: an update is available.
    NasOlder,
    /// The NAS is newer than this program's copy.
    NasNewer,
    /// One of the versions could not be understood and they are not equal as text.
    Unknown,
}

fn parse_semver(v: &str) -> Option<semver::Version> {
    let t = v.trim().trim_start_matches(['v', 'V']);
    let mut parsed = semver::Version::parse(t).ok().or_else(|| {
        // "0.2" or "0.2-dev" -> 0.2.0 / 0.2.0-dev
        let at = t.find(['-', '+']).unwrap_or(t.len());
        let (core, rest) = t.split_at(at);
        if core.split('.').count() == 2 { semver::Version::parse(&format!("{core}.0{rest}")).ok() } else { None }
    })?;
    parsed.build = semver::BuildMetadata::EMPTY; // build metadata does not take part in precedence
    Some(parsed)
}

/// Compare the NAS's version with the bundled one (semantic versioning; `v` prefixes and two-part versions are accepted).
pub fn compare(nas_version: &str, bundled: &str) -> VersionRelation {
    if nas_version.trim() == bundled.trim() {
        return VersionRelation::Same;
    }
    match (parse_semver(nas_version), parse_semver(bundled)) {
        (Some(n), Some(b)) => match n.cmp(&b) {
            std::cmp::Ordering::Equal => VersionRelation::Same,
            std::cmp::Ordering::Less => VersionRelation::NasOlder,
            std::cmp::Ordering::Greater => VersionRelation::NasNewer,
        },
        _ => VersionRelation::Unknown,
    }
}

// ---------------------------------------------------------------------------------------------- the files we ship

/// `dir` is an offline bundle (has `manifest.json`), or the `mgmt/` folder of one (its parent has `manifest.json`): the bundle's root.
fn offline_bundle_root(dir: &Path) -> Option<PathBuf> {
    if dir.join(crate::offline::MANIFEST_FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    let parent = dir.parent()?;
    (dir.file_name().is_some_and(|n| n == crate::offline::MGMT_SUBDIR) && parent.join(crate::offline::MANIFEST_FILE).is_file())
        .then(|| parent.to_path_buf())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    pub name: String,
    pub data: Vec<u8>,
    pub executable: bool,
}

impl BundleFile {
    pub fn sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.data))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "path", rename_all = "snake_case")]
pub enum BundleSource {
    /// The copy compiled into this program.
    Embedded,
    /// A directory given by the user (a staged copy, or an offline bundle's `mgmt/` folder).
    Dir(PathBuf),
}

impl BundleSource {
    pub fn label(&self) -> String {
        match self {
            BundleSource::Embedded => "the copy built into this program".to_string(),
            BundleSource::Dir(p) => format!("{}", p.display()),
        }
    }
}

/// A complete set of management-plane files to put on a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MgmtBundle {
    pub version: String,
    pub source: BundleSource,
    pub files: Vec<BundleFile>,
}

/// The management plane version shipped inside this program (`apps/mgmt-plane/VERSION` at build time).
pub fn bundled_mgmt_version() -> &'static str {
    EMBEDDED.iter().find(|(n, _)| *n == "VERSION").map(|(_, c)| c.trim()).unwrap_or("")
}

impl MgmtBundle {
    /// The copy built into this program (the same files `payload.rs` ships to a freshly installed unit, plus `VERSION`).
    pub fn embedded() -> MgmtBundle {
        let files = EMBEDDED
            .iter()
            .map(|(name, content)| BundleFile {
                name: name.to_string(),
                data: content.as_bytes().to_vec(),
                executable: MGMT_FILES.iter().any(|(n, _, x)| n == name && *x),
            })
            .collect();
        MgmtBundle { version: bundled_mgmt_version().to_string(), source: BundleSource::Embedded, files }
    }

    /// Load a management plane from a directory: either the files themselves (`synofurb_mgmt.py`, `synofurb-mgmt.service`, `VERSION`,
    /// optional `README.md`) or an offline bundle / release folder that keeps them in a `mgmt/` subfolder. For a real offline bundle
    /// (`manifest.json` next to `mgmt/`), pass either the bundle or its `mgmt/` folder. Symbolic links are refused.
    /// A `MANIFEST.sha256` in that folder is checked; a mismatch is an error. Files outside the list above are ignored.
    pub fn from_dir(dir: &Path) -> Result<MgmtBundle> {
        // An offline bundle (or its `mgmt/` folder): the engine also checks every file against the bundle's `manifest.json`, so a
        // pushed update is exactly what `bundle create` put there.
        if let Some(bundle_root) = offline_bundle_root(dir) {
            let b = crate::offline::Bundle::open(&bundle_root)
                .with_context(|| format!("{} looks like an offline bundle but cannot be opened", bundle_root.display()))?;
            return match b.mgmt()? {
                Some(m) => Ok(m),
                None => bail!(
                    "the offline bundle {} has no mgmt/ folder (it was built by an older synofurb). Run `synofurb bundle create` again with the same --out directory to add it, or point --from-bundle at a folder made by apps/mgmt-plane/tools/make-update-bundle.py",
                    bundle_root.display()
                ),
            };
        }
        MgmtBundle::from_plain_dir(dir)
    }

    /// [`MgmtBundle::from_dir`] without the offline-bundle detection (the bundle code itself reads its `mgmt/` folder this way).
    pub(crate) fn from_plain_dir(dir: &Path) -> Result<MgmtBundle> {
        let root = if dir.join("synofurb_mgmt.py").exists() || !dir.join("mgmt").is_dir() { dir.to_path_buf() } else { dir.join("mgmt") };
        if !root.is_dir() {
            bail!("{} is not a directory", dir.display());
        }
        let mut files = vec![];
        for (name, required, executable) in MGMT_FILES {
            let p = root.join(name);
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_file() => {
                    if m.len() > MAX_FILE_BYTES {
                        bail!("{} is larger than {} MiB", p.display(), MAX_FILE_BYTES / 1024 / 1024);
                    }
                    files.push(BundleFile {
                        name: name.to_string(),
                        data: std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?,
                        executable: *executable,
                    });
                }
                Ok(_) => bail!("{} is not a regular file (links are not accepted)", p.display()),
                Err(_) if *required => bail!("{} is missing {name}: not a management plane folder", root.display()),
                Err(_) => {}
            }
        }
        let version = String::from_utf8_lossy(&files.iter().find(|f| f.name == "VERSION").map(|f| f.data.clone()).unwrap_or_default())
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !valid_version_text(&version) {
            bail!("{} has an invalid VERSION file ({version:?})", root.display());
        }
        let b = MgmtBundle { version, source: BundleSource::Dir(root.clone()), files };
        let manifest = root.join(MANIFEST_NAME);
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest).with_context(|| format!("reading {}", manifest.display()))?;
            b.check_manifest(&text).with_context(|| format!("{} does not match its files", manifest.display()))?;
        }
        Ok(b)
    }

    /// Verify `sha256sum`-style text against the files in this bundle: every listed file that we ship must match, and every file we
    /// ship must be listed.
    pub fn check_manifest(&self, text: &str) -> Result<()> {
        let mut listed = std::collections::BTreeMap::new();
        for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (h, n) = l.split_once(char::is_whitespace).ok_or_else(|| anyhow!("malformed line {l:?}"))?;
            listed.insert(n.trim().trim_start_matches('*').to_string(), h.to_ascii_lowercase());
        }
        for f in &self.files {
            match listed.get(&f.name) {
                None => bail!("{} is not listed", f.name),
                Some(h) if *h != f.sha256() => bail!("{} has a different checksum", f.name),
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// `MANIFEST.sha256` for the staged directory (the `sha256sum` format, sorted by name).
    pub fn manifest(&self) -> String {
        let mut v: Vec<&BundleFile> = self.files.iter().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v.iter().map(|f| format!("{}  {}\n", f.sha256(), f.name)).collect()
    }

    pub fn script_sha256(&self) -> Option<String> {
        self.files.iter().find(|f| f.name == "synofurb_mgmt.py").map(BundleFile::sha256)
    }

    /// The staged directory as a tar.gz: the files plus `MANIFEST.sha256`. Deterministic (fixed times and owners).
    pub fn tarball(&self) -> Result<Vec<u8>> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        let mut tar = tar::Builder::new(gz);
        let mut add = |name: &str, data: &[u8], mode: u32| -> Result<()> {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_mtime(0);
            h.set_uid(0);
            h.set_gid(0);
            tar.append_data(&mut h, name, data)?;
            Ok(())
        };
        let mut files: Vec<&BundleFile> = self.files.iter().collect();
        files.sort_by(|a, b| a.name.cmp(&b.name));
        for f in files {
            add(&f.name, &f.data, if f.executable { 0o755 } else { 0o644 })?;
        }
        add(MANIFEST_NAME, self.manifest().as_bytes(), 0o644)?;
        Ok(tar.into_inner()?.finish()?)
    }
}

// ---------------------------------------------------------------------------------------------- planning

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateDecision {
    /// Same version, same files: nothing to do.
    UpToDate,
    /// The NAS is older.
    Upgrade,
    /// Same version number but the script differs from this program's copy (a development build).
    SameVersionChanged,
    /// Same files, `--force`.
    Reinstall,
    /// The NAS is newer and `--force` was given.
    Downgrade,
    /// The NAS is newer; refused without `--force`.
    DowngradeNeedsForce,
    /// Nothing usable is installed (or its version is unreadable): put this copy on the unit.
    Install,
}

impl UpdateDecision {
    /// Does this decision change the unit?
    pub fn applies(self) -> bool {
        !matches!(self, UpdateDecision::UpToDate | UpdateDecision::DowngradeNeedsForce)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedFile {
    pub name: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdatePlan {
    pub decision: UpdateDecision,
    pub nas_version: Option<String>,
    pub new_version: String,
    pub source: BundleSource,
    pub files: Vec<PlannedFile>,
    /// The web UI restarts (it is unavailable for a few seconds) when the update is applied.
    pub restarts_web_ui: bool,
    /// One sentence for a confirmation dialog or the CLI.
    pub message: String,
}

/// Decide what an update would do. Pure.
pub fn plan_update(installed: &InstalledInfo, bundle: &MgmtBundle, force: bool) -> UpdatePlan {
    let nas = installed.version.clone().filter(|_| installed.present);
    let same_files = match (&installed.script_sha256, bundle.script_sha256()) {
        (Some(a), Some(b)) => Some(*a == b),
        _ => None,
    };
    let relation = nas.as_deref().map(|n| compare(n, &bundle.version));
    let decision = match (installed.present, relation) {
        (false, _) | (true, None) | (true, Some(VersionRelation::Unknown)) => {
            // unreadable version: same bytes means nothing to do, otherwise install this copy
            if installed.present && same_files == Some(true) && !force { UpdateDecision::UpToDate } else { UpdateDecision::Install }
        }
        (true, Some(VersionRelation::NasOlder)) => UpdateDecision::Upgrade,
        (true, Some(VersionRelation::NasNewer)) => {
            if force {
                UpdateDecision::Downgrade
            } else {
                UpdateDecision::DowngradeNeedsForce
            }
        }
        (true, Some(VersionRelation::Same)) => match (same_files, force) {
            (Some(false), _) => UpdateDecision::SameVersionChanged,
            (_, true) => UpdateDecision::Reinstall,
            _ => UpdateDecision::UpToDate,
        },
    };
    let from = nas.as_deref().unwrap_or("unknown");
    let to = &bundle.version;
    let message = match decision {
        UpdateDecision::UpToDate => format!("The management plane is up to date ({to})."),
        UpdateDecision::Upgrade => format!("Update the management plane from {from} to {to}."),
        UpdateDecision::SameVersionChanged => {
            format!("Version {to} is installed, but its files differ from this program's copy; replace them.")
        }
        UpdateDecision::Reinstall => format!("Reinstall the management plane files of version {to}."),
        UpdateDecision::Downgrade => format!("Downgrade the management plane from {from} to {to}."),
        UpdateDecision::DowngradeNeedsForce => format!("The unit has {from}, which is newer than {to}; use --force to go back."),
        UpdateDecision::Install => format!("Install the management plane {to} (installed: {from})."),
    };
    UpdatePlan {
        decision,
        nas_version: nas,
        new_version: bundle.version.clone(),
        source: bundle.source.clone(),
        files: bundle.files.iter().map(|f| PlannedFile { name: f.name.clone(), bytes: f.data.len(), sha256: f.sha256() }).collect(),
        restarts_web_ui: decision.applies(),
        message,
    }
}

// ---------------------------------------------------------------------------------------------- events

/// Status word of a step reported by the NAS-side updater.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Running,
    Ok,
    Warn,
    Failed,
    Skipped,
    /// A rollback step (running, done or otherwise).
    Rollback,
    #[default]
    Info,
}

impl StepStatus {
    pub fn parse(s: &str) -> StepStatus {
        let s = s.trim().to_ascii_lowercase();
        if s.contains("rollback") || s.contains("rolled") || s.contains("restor") {
            return StepStatus::Rollback;
        }
        match s.as_str() {
            "run" | "running" | "start" | "started" | "begin" | "progress" | "in_progress" | "doing" => StepStatus::Running,
            "ok" | "done" | "success" | "succeeded" | "pass" | "passed" | "finished" | "complete" | "completed" => StepStatus::Ok,
            "warn" | "warning" => StepStatus::Warn,
            "fail" | "failed" | "error" | "err" | "fatal" => StepStatus::Failed,
            "skip" | "skipped" => StepStatus::Skipped,
            _ => StepStatus::Info,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Running => "running",
            StepStatus::Ok => "ok",
            StepStatus::Warn => "warn",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
            StepStatus::Rollback => "rollback",
            StepStatus::Info => "info",
        }
    }
}

/// One progress line of the NAS-side updater: `{"step":"install","status":"ok","message":"..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteStep {
    pub step: String,
    pub status: StepStatus,
    /// The status word exactly as the NAS wrote it.
    pub raw_status: String,
    pub message: String,
}

impl RemoteStep {
    fn is_rollback(&self) -> bool {
        self.status == StepStatus::Rollback || self.step.to_ascii_lowercase().contains("rollback")
    }
    fn level(&self) -> Level {
        match self.status {
            StepStatus::Failed => Level::Error,
            StepStatus::Warn | StepStatus::Rollback => Level::Warn,
            _ => Level::Info,
        }
    }
    /// "install: ok - copied 3 files"
    pub fn line(&self) -> String {
        let mut s = format!("{}: {}", self.step, self.raw_status);
        if !self.message.is_empty() {
            s.push_str(" - ");
            s.push_str(&self.message);
        }
        s
    }
}

/// Parse one stdout line of the NAS updater. `None` for blank lines; plain text becomes a [`ManageEvent::Log`].
pub fn parse_remote_line(line: &str) -> Option<ManageEvent> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if t.starts_with('{')
        && let Ok(Value::Object(o)) = serde_json::from_str::<Value>(t)
    {
        let text = |keys: &[&str]| keys.iter().find_map(|k| o.get(*k).and_then(Value::as_str)).unwrap_or("").trim().to_string();
        let (step, status, message) = (text(&["step", "name"]), text(&["status", "state"]), text(&["message", "msg"]));
        if step.is_empty() && status.is_empty() && message.is_empty() {
            return Some(ManageEvent::Log { level: Level::Debug, msg: t.to_string() });
        }
        let raw_status = if status.is_empty() { "info".to_string() } else { status };
        return Some(ManageEvent::Remote(RemoteStep {
            step: if step.is_empty() { "update".to_string() } else { step },
            status: StepStatus::parse(&raw_status),
            raw_status,
            message,
        }));
    }
    Some(ManageEvent::Log { level: Level::Info, msg: t.to_string() })
}

/// Progress of a management action. Same vocabulary as [`Event`] (steps, progress fractions, log lines) plus the NAS's own step lines;
/// [`ManageEvent::emit`] feeds a [`Logger`] so the CLI's progress display, the log file and `--log-format json` work unchanged.
#[derive(Debug, Clone, PartialEq)]
pub enum ManageEvent {
    /// Stage `n` of `total` on this computer's side ("Uploading the files").
    Step {
        n: u32,
        total: u32,
        name: String,
    },
    /// Upload progress, 0.0..=1.0.
    Progress {
        what: String,
        fraction: f32,
    },
    /// A step line printed by the updater on the NAS.
    Remote(RemoteStep),
    Log {
        level: Level,
        msg: String,
    },
}

impl ManageEvent {
    /// The core event this maps to (`Remote` and `Log` become log lines). The timestamp is set here.
    pub fn to_core(&self) -> Event {
        match self {
            ManageEvent::Step { n, total, name } => Event::Step { n: *n, total: *total, name: name.clone() },
            ManageEvent::Progress { what, fraction } => Event::Progress { what: what.clone(), fraction: *fraction },
            ManageEvent::Remote(r) => Event::Log(LogLine {
                ts: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
                level: r.level(),
                source: Source::App,
                msg: format!("NAS {}", r.line()),
            }),
            ManageEvent::Log { level, msg } => Event::Log(LogLine {
                ts: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
                level: *level,
                source: Source::App,
                msg: msg.clone(),
            }),
        }
    }

    /// Send to a logger: steps and progress as events, everything else as (secret-redacted, file-logged) lines.
    pub fn emit(&self, log: &Logger) {
        match self {
            ManageEvent::Step { .. } | ManageEvent::Progress { .. } => log.emit(self.to_core()),
            ManageEvent::Remote(r) => log.log(r.level(), Source::App, &format!("NAS {}", r.line())),
            ManageEvent::Log { level, msg } => log.log(*level, Source::App, msg),
        }
    }

    /// The NDJSON form. `step`, `progress` and `log` have exactly the shape the CLI's `--log-format json` uses; the NAS's own lines are
    /// `{"event":"nas_step","step","status","message"}`.
    pub fn to_json(&self) -> Value {
        match self {
            ManageEvent::Step { n, total, name } => json!({"event": "step", "n": n, "total": total, "name": name}),
            ManageEvent::Progress { what, fraction } => {
                json!({"event": "progress", "what": what, "fraction": (*fraction as f64 * 1000.0).round() / 1000.0})
            }
            ManageEvent::Remote(r) => {
                json!({"event": "nas_step", "step": r.step, "status": r.status.as_str(), "raw_status": r.raw_status, "message": r.message})
            }
            ManageEvent::Log { level, msg } => {
                let l = match level {
                    Level::Debug => "debug",
                    Level::Info => "info",
                    Level::Warn => "warn",
                    Level::Error => "error",
                };
                json!({"event": "log", "level": l, "source": "app", "msg": msg})
            }
        }
    }
}

/// Splits a byte stream into lines as they arrive and turns each into a [`ManageEvent`] (live, not after the command ends).
struct LineSink<'a> {
    buf: Vec<u8>,
    emit: &'a mut dyn FnMut(ManageEvent),
    steps: Vec<RemoteStep>,
}

impl<'a> LineSink<'a> {
    fn new(emit: &'a mut dyn FnMut(ManageEvent)) -> Self {
        LineSink { buf: vec![], emit, steps: vec![] }
    }
    fn line(&mut self, bytes: &[u8]) {
        if let Some(ev) = parse_remote_line(&String::from_utf8_lossy(bytes)) {
            if let ManageEvent::Remote(r) = &ev {
                self.steps.push(r.clone());
            }
            (self.emit)(ev);
        }
    }
    fn finish(&mut self) {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            self.line(&rest);
        }
    }
}

impl Write for LineSink<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            self.line(&line);
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------- update / rollback

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateOutcome {
    Updated,
    UpToDate,
    RolledBack,
    /// Nothing was done on purpose (a downgrade without `--force`, or cancelled before anything changed).
    Refused,
    Failed,
}

/// What state the unit is in after a failed update, judged from a second health call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackState {
    /// The update succeeded or never started; there was nothing to restore.
    NotApplicable,
    /// The unit answers and runs the version it had before: the old files are in place.
    Restored,
    /// The unit answers but runs another version than before the update: do not assume the old state.
    Incomplete,
    /// The health check could not be completed after the failure: look at the unit.
    Unverified,
}

/// What was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Update,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMethod {
    /// The updater of the management plane on the unit (`update --from DIR`).
    Unit,
    /// A built-in swap script, used when the installed copy is too old to have an updater.
    Bootstrap,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub operation: Operation,
    pub outcome: UpdateOutcome,
    pub method: Option<UpdateMethod>,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub plan: Option<UpdatePlan>,
    pub rollback: RollbackState,
    /// The NAS reported rollback steps itself.
    pub rollback_reported: bool,
    pub remote_steps: Vec<RemoteStep>,
    /// One sentence on what went wrong.
    pub failure: Option<String>,
    pub health_after: Option<Health>,
    pub summary_after: Option<HealthSummary>,
}

impl UpdateReport {
    fn new(operation: Operation, outcome: UpdateOutcome) -> Self {
        UpdateReport {
            operation,
            outcome,
            method: None,
            from_version: None,
            to_version: None,
            plan: None,
            rollback: RollbackState::NotApplicable,
            rollback_reported: false,
            remote_steps: vec![],
            failure: None,
            health_after: None,
            summary_after: None,
        }
    }
    pub fn ok(&self) -> bool {
        matches!(self.outcome, UpdateOutcome::Updated | UpdateOutcome::UpToDate | UpdateOutcome::RolledBack)
    }
    /// A sentence for the end of a run.
    pub fn headline(&self) -> String {
        let v = |o: &Option<String>| o.clone().unwrap_or_else(|| "unknown".into());
        match self.outcome {
            UpdateOutcome::Updated => format!("Management plane updated: {} -> {}.", v(&self.from_version), v(&self.to_version)),
            UpdateOutcome::UpToDate => format!("Already up to date ({}).", v(&self.to_version)),
            UpdateOutcome::RolledBack => format!("Rolled back to version {}.", v(&self.to_version)),
            UpdateOutcome::Refused => self.failure.clone().unwrap_or_else(|| "Nothing was changed.".into()),
            UpdateOutcome::Failed => {
                let why = self.failure.clone().unwrap_or_else(|| "the update failed".into());
                let state = match self.rollback {
                    RollbackState::Restored => format!("The unit is back on version {}.", v(&self.from_version)),
                    RollbackState::Incomplete => "The unit runs a different version than before the update; check it.".to_string(),
                    RollbackState::Unverified => "The health check after the failure did not complete; check the unit.".to_string(),
                    RollbackState::NotApplicable => "Nothing was changed on the unit.".to_string(),
                };
                let what = if self.operation == Operation::Rollback { "Rollback" } else { "Update" };
                format!("{what} failed: {why} {state}")
            }
        }
    }
}

pub struct UpdateOptions {
    /// Allow a reinstall of identical files and a downgrade.
    pub force: bool,
    /// Health checks after the update (the web service needs a moment).
    pub verify_attempts: u32,
    pub verify_delay: Duration,
    /// Checked before anything is changed; setting it later has no effect on a running update.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        UpdateOptions { force: false, verify_attempts: 4, verify_delay: Duration::from_secs(3), cancel: None }
    }
}

impl UpdateOptions {
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

const STEPS_TOTAL: u32 = 5;

fn step(emit: &mut dyn FnMut(ManageEvent), n: u32, name: &str) {
    emit(ManageEvent::Step { n, total: STEPS_TOTAL, name: name.to_string() });
}
fn note(emit: &mut dyn FnMut(ManageEvent), level: Level, msg: impl Into<String>) {
    emit(ManageEvent::Log { level, msg: msg.into() });
}

/// Health check after a change: a few tries, because the service needs a moment after a restart. Also requires that the management
/// service itself is not reported as stopped.
fn verify_health(p: &mut Privileged<'_>, opts: &UpdateOptions) -> Result<Health, ManageError> {
    let attempts = opts.verify_attempts.max(1);
    let mut last = None;
    for i in 0..attempts {
        if i > 0 && !opts.verify_delay.is_zero() {
            std::thread::sleep(opts.verify_delay);
        }
        match fetch_health(p) {
            Ok(h) => {
                let stopped = h.services.iter().any(|s| s.name.contains("synofurb-mgmt") && s.active == Some(false));
                if !stopped {
                    return Ok(h);
                }
                last = Some(ManageError::BadOutput("the synofurb-mgmt service is not running".into()));
            }
            // an unsupported/missing report will not fix itself
            Err(e @ (ManageError::NotInstalled | ManageError::HealthUnsupported { .. })) => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or(ManageError::BadOutput("no answer".into())))
}

fn last_failure(steps: &[RemoteStep], stderr: &str, code: u32) -> String {
    let failed: Vec<&RemoteStep> = steps.iter().filter(|s| s.status == StepStatus::Failed).collect();
    // The first real failure explains it; the closing "result" line only repeats it, and a failed rollback is the reason only when
    // nothing else failed (or when the rollback is what was asked for).
    let first = failed
        .iter()
        .find(|s| s.step != "result" && !s.is_rollback())
        .or_else(|| failed.iter().find(|s| s.step != "result"))
        .or_else(|| failed.last());
    if let Some(s) = first {
        return if s.message.is_empty() { format!("step \"{}\" failed", s.step) } else { short(&format!("{} ({})", s.message, s.step)) };
    }
    if let Some(l) = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()) {
        return short(l);
    }
    format!("the updater exited with status {code}")
}

fn stage_script(tar: &str, stage: &str, sha: &str) -> String {
    r#"set -eu
umask 077
T=@TAR@
S=@STAGE@
trap 'rm -f "$T"' EXIT
if [ "$(sha256sum "$T" | cut -d' ' -f1)" != '@SHA@' ]; then echo "the uploaded archive does not match its checksum" >&2; exit 3; fi
rm -rf "$S"
mkdir -m 700 "$S"
tar -xzf "$T" -C "$S" --no-same-owner
cd "$S"
sha256sum -c MANIFEST.sha256 >/dev/null
"#
    .replace("@TAR@", &sh_quote(tar))
    .replace("@STAGE@", &sh_quote(stage))
    .replace("@SHA@", sha)
}

/// Swap script for units whose management plane predates `update` (or is broken). It does what the unit's own updater does - verify,
/// keep the old install as `/opt/synofurb-mgmt.prev` (with the old unit file as `unit.backup`, so a later `update --rollback` works),
/// rename the new files into place, restart, check, restore on failure - and speaks the same NDJSON. `root` is empty on a real unit
/// (a prefix in tests, where systemctl is skipped). Messages are static ASCII: they are printed into JSON by `printf`.
pub fn bootstrap_script(root: &str, stage: &str) -> String {
    r#"set -u
ROOT=@ROOT@
STAGE=@STAGE@
OPT="$ROOT/opt/synofurb-mgmt"
NEW="$OPT.new"
PREV="$OPT.prev"
UNIT="$ROOT/etc/systemd/system/synofurb-mgmt.service"
say() { printf '{"step":"%s","status":"%s","message":"%s"}\n' "$1" "$2" "$3"; }
fail() { say "$1" fail "$2"; exit 1; }
svc() { [ -n "$ROOT" ] && return 0; command -v systemctl >/dev/null 2>&1 || return 0; systemctl "$@"; }
up() { [ -n "$ROOT" ] && return 0; sleep 3; systemctl is-active --quiet synofurb-mgmt.service && python3 "$OPT/synofurb_mgmt.py" health --json >/dev/null 2>&1; }
restore() {
  say rollback start "restoring the previous files"
  rm -rf "$OPT.failed"
  if [ -d "$PREV" ]; then
    [ -d "$OPT" ] && mv "$OPT" "$OPT.failed"
    if mv "$PREV" "$OPT"; then
      if [ -f "$OPT/unit.backup" ]; then cp "$OPT/unit.backup" "$UNIT"; rm -f "$OPT/unit.backup"; fi
      svc daemon-reload; svc restart synofurb-mgmt.service
      rm -rf "$OPT.failed"
      say rollback ok "previous files restored"
      return 0
    fi
  fi
  say rollback fail "could not restore the previous files"
  return 1
}
abort() {
  if restore; then say result fail "$1; rolled back"; exit 3; fi
  say result fail "$1; the rollback failed"; exit 4
}
say verify start "checking the staged files"
[ -f "$STAGE/MANIFEST.sha256" ] || fail verify "MANIFEST.sha256 is missing"
(cd "$STAGE" && sha256sum -c MANIFEST.sha256 >/dev/null 2>&1) || fail verify "a staged file does not match MANIFEST.sha256"
for f in synofurb_mgmt.py VERSION README.md; do [ -f "$STAGE/$f" ] || fail verify "$f is missing from the update"; done
say verify ok "staged files match their checksums"
say stage start "keeping the current files for rollback"
mkdir -p "$ROOT/opt" || fail stage "cannot write below /opt"
rm -rf "$PREV" "$NEW"
if [ -d "$OPT" ]; then
  cp -a "$OPT" "$PREV" || fail stage "could not copy the current files"
  if [ -f "$UNIT" ]; then cp -p "$UNIT" "$PREV/unit.backup"; fi
fi
mkdir -p "$NEW" || fail stage "cannot create a staging folder"
for f in synofurb_mgmt.py VERSION README.md synofurb-mgmt.service synofurb_addons.py addons.pack.json synofurb_edge.py; do
  if [ -f "$STAGE/$f" ]; then cp "$STAGE/$f" "$NEW/$f" || fail stage "could not copy $f"; fi
done
chmod 0755 "$NEW/synofurb_mgmt.py"; chmod 0644 "$NEW/VERSION" "$NEW/README.md"
[ -f "$NEW/synofurb-mgmt.service" ] && chmod 0644 "$NEW/synofurb-mgmt.service"
[ -f "$NEW/synofurb_addons.py" ] && chmod 0644 "$NEW/synofurb_addons.py"
[ -f "$NEW/addons.pack.json" ] && chmod 0644 "$NEW/addons.pack.json"
[ -f "$NEW/synofurb_edge.py" ] && chmod 0644 "$NEW/synofurb_edge.py"
say stage ok "files staged"
say swap start "putting the new files in place"
[ -d "$OPT" ] && mv "$OPT" "$OPT.old"
if ! mv "$NEW" "$OPT"; then
  [ -d "$OPT.old" ] && mv "$OPT.old" "$OPT"
  fail swap "could not move the new files into place"
fi
rm -rf "$OPT.old"
if [ -f "$OPT/synofurb-mgmt.service" ]; then
  mkdir -p "$(dirname "$UNIT")"
  cp "$OPT/synofurb-mgmt.service" "$UNIT.new" && mv -f "$UNIT.new" "$UNIT"
fi
say swap ok "files in place"
say restart start "restarting the management service"
svc daemon-reload
svc enable synofurb-mgmt.service >/dev/null 2>&1
if ! svc restart synofurb-mgmt.service; then abort "the service did not start"; fi
if ! up; then abort "the new version did not answer"; fi
say restart ok "the service answers"
say result ok "updated"
"#
    .replace("@ROOT@", &sh_quote(root))
    .replace("@STAGE@", &sh_quote(stage))
}

/// Upload `script` to a private temp file and run it, streaming stdout to `sink`. Returns (exit code, stderr).
fn run_script_streaming(p: &mut Privileged<'_>, script: &str, sink: &mut LineSink<'_>) -> Result<(u32, String)> {
    let name = format!("/tmp/.synofurb-{}.sh", random_token(6));
    let mut cur = std::io::Cursor::new(script.as_bytes().to_vec());
    p.sh.upload(&name, &mut cur, script.len() as u64, 0o700, &mut |_| {})?;
    let r = p.run_to(&format!("/bin/sh '{name}'"), sink);
    sink.finish();
    let _ = p.sh.exec(&format!("rm -f '{name}'"), None);
    r
}

fn run_unit_updater(p: &mut Privileged<'_>, args: &str, sink: &mut LineSink<'_>) -> Result<(u32, String)> {
    let r = p.run_to(&format!("python3 {MGMT_SCRIPT} update {args} --json"), sink);
    sink.finish();
    r
}

fn cleanup(p: &mut Privileged<'_>, stage: &str, tar: &str) {
    let _ = p.run(&format!("rm -rf {} {}", sh_quote(stage), sh_quote(tar)));
}

/// Put a management plane on the unit: stage the files, run the update there (which rolls back by itself on failure), verify with a
/// second health call and report what state the unit is in. `emit` receives progress as it happens.
///
/// Returns `Err` only when the unit could not be reached or driven at all; a failed update is an `Ok` report with
/// [`UpdateOutcome::Failed`] so the caller can show the rollback state.
pub fn push_update(
    p: &mut Privileged<'_>,
    bundle: &MgmtBundle,
    opts: &UpdateOptions,
    emit: &mut dyn FnMut(ManageEvent),
) -> Result<UpdateReport> {
    need_root(p)?;
    step(emit, 1, "Reading the installed management plane");
    let installed = read_installed(p.sh)?;
    let before = fetch_health(p);
    if let Err(ManageError::Transport(e)) = before {
        return Err(e.context("reading the unit's health"));
    }
    // The unit's own updater is used only when the installed copy demonstrably works (it answered `health`); anything else - missing,
    // too old, broken - is repaired by the built-in swap.
    let health_supported = before.is_ok();
    let plan = plan_update(&installed, bundle, opts.force);
    let mut report = UpdateReport::new(Operation::Update, UpdateOutcome::Failed);
    report.from_version = installed.version.clone().or_else(|| before.as_ref().ok().map(|h| h.version.clone()));
    report.to_version = Some(bundle.version.clone());
    report.plan = Some(plan.clone());
    note(emit, Level::Info, plan.message.clone());
    match plan.decision {
        UpdateDecision::UpToDate => {
            report.outcome = UpdateOutcome::UpToDate;
            report.health_after = before.ok();
            report.summary_after = report.health_after.as_ref().map(Health::summary);
            return Ok(report);
        }
        UpdateDecision::DowngradeNeedsForce => {
            report.outcome = UpdateOutcome::Refused;
            report.failure = Some(plan.message);
            return Ok(report);
        }
        _ => {}
    }
    if opts.cancelled() {
        report.outcome = UpdateOutcome::Refused;
        report.failure = Some("Cancelled before anything was changed.".into());
        return Ok(report);
    }

    // ------------------------------------------------------------------ stage
    step(emit, 2, "Uploading the update files");
    let tok = random_token(6);
    let (stage, tar_path) = (format!("/tmp/.synofurb-mgmt-stage-{tok}"), format!("/tmp/.synofurb-mgmt-{tok}.tar.gz"));
    let tar = bundle.tarball()?;
    let sha = hex::encode(Sha256::digest(&tar));
    {
        let size = tar.len() as u64;
        let mut cur = std::io::Cursor::new(&tar[..]);
        let mut sent = |n: u64| {
            emit(ManageEvent::Progress {
                what: "update files".into(),
                fraction: if size == 0 { 1.0 } else { (n as f32 / size as f32).min(1.0) },
            })
        };
        if let Err(e) = p.sh.upload(&tar_path, &mut cur, size, 0o600, &mut sent) {
            cleanup(p, &stage, &tar_path);
            return Err(e.context("uploading the update files"));
        }
    }
    let staged = p.run_script(&stage_script(&tar_path, &stage, &sha));
    match staged {
        Ok(r) if r.ok() => note(emit, Level::Info, format!("Staged {} files, checksums verified on the unit.", bundle.files.len())),
        Ok(r) => {
            cleanup(p, &stage, &tar_path);
            bail!("staging the update on the unit failed (exit {}): {}", r.code, short(r.stderr.trim()));
        }
        Err(e) => {
            cleanup(p, &stage, &tar_path);
            return Err(e.context("staging the update on the unit"));
        }
    }
    if opts.cancelled() {
        cleanup(p, &stage, &tar_path);
        report.outcome = UpdateOutcome::Refused;
        report.failure = Some("Cancelled before anything was changed.".into());
        return Ok(report);
    }

    // ------------------------------------------------------------------ apply
    step(emit, 3, "Applying the update (the web UI restarts)");
    let mut sink = LineSink::new(emit);
    let mut method = if health_supported { UpdateMethod::Unit } else { UpdateMethod::Bootstrap };
    let applied = if method == UpdateMethod::Unit {
        let force = if plan.decision == UpdateDecision::Downgrade { " --force" } else { "" };
        run_unit_updater(p, &format!("--from {}{force}", sh_quote(&stage)), &mut sink)
    } else {
        (sink.emit)(ManageEvent::Log {
            level: Level::Info,
            msg: "The installed management plane is too old to update itself; using the built-in swap.".into(),
        });
        run_script_streaming(p, &bootstrap_script("", &stage), &mut sink)
    };
    let mut applied = applied;
    if method == UpdateMethod::Unit
        && let Ok((code, err)) = &applied
        && *code != 0
        && sink.steps.is_empty()
        && looks_like_unknown_subcommand(&ExecResult { code: *code, stdout: String::new(), stderr: err.clone() })
    {
        // health exists but `update` does not (an in-between version): nothing was changed, use the built-in swap
        (sink.emit)(ManageEvent::Log { level: Level::Info, msg: "The unit has no updater; using the built-in swap.".into() });
        method = UpdateMethod::Bootstrap;
        applied = run_script_streaming(p, &bootstrap_script("", &stage), &mut sink);
    }
    let steps = std::mem::take(&mut sink.steps);
    drop(sink);
    report.method = Some(method);
    report.rollback_reported = steps.iter().any(RemoteStep::is_rollback);
    report.remote_steps = steps;
    let (code, stderr) = match applied {
        Ok(x) => x,
        Err(e) => {
            // the connection died while the unit was working: find out what state it is in
            cleanup(p, &stage, &tar_path);
            report.failure = Some(format!("lost the connection while updating: {e:#}"));
            finish_failed(p, opts, &mut report, emit);
            return Ok(report);
        }
    };
    for l in stderr.lines().map(str::trim).filter(|l| !l.is_empty()) {
        note(emit, if code == 0 { Level::Debug } else { Level::Warn }, format!("NAS stderr: {l}"));
    }

    // ------------------------------------------------------------------ verify
    step(emit, 4, "Checking the unit afterwards");
    let after = if code == 0 { verify_health(p, opts) } else { Err(ManageError::BadOutput(String::new())) };
    step(emit, 5, "Cleaning up");
    cleanup(p, &stage, &tar_path);

    if code != 0 {
        report.failure = Some(last_failure(&report.remote_steps, &stderr, code));
        finish_failed(p, opts, &mut report, emit);
        // exit 2 = the unit refused the files before changing anything: there was nothing to restore
        if code == 2 && report.rollback == RollbackState::Restored {
            report.rollback = RollbackState::NotApplicable;
        }
        return Ok(report);
    }
    match after {
        Ok(h) if compare(&h.version, &bundle.version) == VersionRelation::Same => {
            report.outcome = UpdateOutcome::Updated;
            report.summary_after = Some(h.summary());
            report.health_after = Some(h);
        }
        other => {
            let why = match other {
                Ok(h) => format!("the unit reports version {} after the update, expected {}", h.version, bundle.version),
                Err(e) => format!("the unit did not pass the health check after the update ({e})"),
            };
            note(emit, Level::Error, why.clone());
            report.failure = Some(why);
            // the unit said it succeeded but does not look right: ask it to go back
            if method == UpdateMethod::Unit {
                note(emit, Level::Warn, "Asking the unit to roll back.");
                let mut sink = LineSink::new(emit);
                let rb = run_unit_updater(p, "--rollback", &mut sink);
                let steps = std::mem::take(&mut sink.steps);
                drop(sink);
                report.rollback_reported |= steps.iter().any(RemoteStep::is_rollback);
                report.remote_steps.extend(steps);
                if let Err(e) = rb {
                    note(emit, Level::Error, format!("The rollback request failed: {e:#}"));
                }
            }
            finish_failed(p, opts, &mut report, emit);
        }
    }
    Ok(report)
}

/// After a failure: look at the unit again and decide what the rollback state is.
fn finish_failed(p: &mut Privileged<'_>, opts: &UpdateOptions, report: &mut UpdateReport, emit: &mut dyn FnMut(ManageEvent)) {
    report.outcome = UpdateOutcome::Failed;
    match verify_health(p, opts) {
        Ok(h) => {
            report.rollback = match &report.from_version {
                Some(from) if compare(&h.version, from) == VersionRelation::Same => RollbackState::Restored,
                Some(_) => RollbackState::Incomplete,
                // nothing to compare with; it answers, which is the best we can say
                None => RollbackState::Unverified,
            };
            report.summary_after = Some(h.summary());
            report.health_after = Some(h);
        }
        Err(e) => {
            note(emit, Level::Error, format!("The unit does not answer the health check: {e}"));
            report.rollback = RollbackState::Unverified;
        }
    }
}

/// Go back to the previous management plane (`update --rollback` on the unit), then verify.
pub fn push_rollback(p: &mut Privileged<'_>, opts: &UpdateOptions, emit: &mut dyn FnMut(ManageEvent)) -> Result<UpdateReport> {
    need_root(p)?;
    step(emit, 1, "Reading the installed management plane");
    let installed = read_installed(p.sh)?;
    if !installed.present {
        bail!("{}", ManageError::NotInstalled);
    }
    let before = fetch_health(p);
    if let Err(e @ (ManageError::HealthUnsupported { .. } | ManageError::NotInstalled)) = &before {
        bail!("this unit cannot roll back remotely: {e}");
    }
    let mut report = UpdateReport::new(Operation::Rollback, UpdateOutcome::Failed);
    report.from_version = installed.version.clone();
    report.method = Some(UpdateMethod::Unit);
    step(emit, 2, "Rolling back");
    let mut sink = LineSink::new(emit);
    let applied = run_unit_updater(p, "--rollback", &mut sink);
    let steps = std::mem::take(&mut sink.steps);
    drop(sink);
    report.rollback_reported = true;
    report.remote_steps = steps;
    let (code, stderr) = applied?;
    step(emit, 3, "Checking the unit afterwards");
    let after = verify_health(p, opts);
    if code != 0 {
        report.failure = Some(last_failure(&report.remote_steps, &stderr, code));
        report.outcome = UpdateOutcome::Failed;
        report.rollback = RollbackState::Unverified;
        report.health_after = after.ok();
        report.summary_after = report.health_after.as_ref().map(Health::summary);
        return Ok(report);
    }
    match after {
        Ok(h) => {
            report.outcome = UpdateOutcome::RolledBack;
            report.to_version = Some(h.version.clone());
            report.summary_after = Some(h.summary());
            report.health_after = Some(h);
        }
        Err(e) => {
            report.failure = Some(format!("the rollback finished but the unit does not pass the health check ({e})"));
            report.rollback = RollbackState::Unverified;
        }
    }
    Ok(report)
}

// ---------------------------------------------------------------------------------------------- one-call status for the UI

/// Everything a screen needs to show the state of a unit's management plane: installed version, health (when available), and what an
/// update would do with this program's copy.
#[derive(Debug, Clone, Serialize)]
pub struct ManageStatus {
    pub installed: InstalledInfo,
    pub bundled_version: String,
    pub relation: Option<VersionRelation>,
    pub plan: UpdatePlan,
    pub health: Option<Health>,
    pub summary: Option<HealthSummary>,
    /// Why there is no health report (not installed, too old, ...).
    pub health_error: Option<String>,
}

/// Read installed version and health in one go and compare with `bundle`. Never fails because of the management plane's own
/// state (that is reported in the result); only a dead connection is an `Err`.
pub fn status(p: &mut Privileged<'_>, bundle: &MgmtBundle) -> Result<ManageStatus> {
    let installed = read_installed(p.sh)?;
    let (health, health_error) = if installed.present {
        match fetch_health(p) {
            Ok(h) => (Some(h), None),
            Err(ManageError::Transport(e)) => return Err(e),
            Err(e) => (None, Some(e.to_string())),
        }
    } else {
        (None, Some(ManageError::NotInstalled.to_string()))
    };
    let installed =
        InstalledInfo { version: installed.version.clone().or_else(|| health.as_ref().map(|h| h.version.clone())), ..installed };
    let relation = installed.version.as_deref().filter(|_| installed.present).map(|v| compare(v, &bundle.version));
    Ok(ManageStatus {
        plan: plan_update(&installed, bundle, false),
        relation,
        bundled_version: bundle.version.clone(),
        summary: health.as_ref().map(Health::summary),
        installed,
        health,
        health_error,
    })
}

#[cfg(test)]
mod tests;
