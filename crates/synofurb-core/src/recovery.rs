//! Recovery engine for units that use the internal boot flash (DOM): read what the unit's boot state really is, act on it, and find out
//! whether DSM is back.
//!
//! Nothing here assumes what a previous run did. [`read_boot_state`] asks the unit (`synofurb-dom status --json`, and `sf-boot status`
//! on the boot agent); "could not read" is a state of its own ([`DefaultEntry::Unknown`], [`ReadOutcome`]) and is never reported as
//! "DSM is the default". The actions ([`enter_agent`], [`restore_flash`], [`remove_our_entries`], [`boot_dsm`]) refuse to run without
//! `confirmed = true`, return typed [`RecoveryError`]s, and describe only what they verified: [`boot_dsm`] with a reboot says
//! "reboot requested" and nothing more; use [`wait_for_dsm`] / [`probe_unit`] to learn that DSM answers.

use crate::logs::Logger;
use crate::remote::{ExecResult, Privileged, RemoteShell, SshSession};
use crate::session::{SshTarget, config_keys_dir};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::io::Read;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------- the unit's boot state

/// The entry the boot menu starts when nobody chooses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum DefaultEntry {
    /// One of DSM's own entries (`SYNOLOGY_1`, `SYNOLOGY_2`).
    Dsm(String),
    /// Our boot agent entry (`SYNOFURB_BOOT`): the unit boots the agent.
    Agent(String),
    /// Our Debian entries (`DEBIAN`, `DEBIAN_OLD`).
    Debian(String),
    /// Anything else (an EFI entry of ours, or an entry we do not know).
    Other(String),
    /// Not known. **Not** the same as DSM: nothing could be read, or the unit's answer did not say. The text says why.
    Unknown(String),
}

impl DefaultEntry {
    /// Classify a menu entry name.
    pub fn from_name(name: &str) -> DefaultEntry {
        let n = name.trim();
        if n.starts_with("SYNOLOGY_") {
            DefaultEntry::Dsm(n.to_string())
        } else if n == "SYNOFURB_BOOT" {
            DefaultEntry::Agent(n.to_string())
        } else if n == "DEBIAN" || n == "DEBIAN_OLD" {
            DefaultEntry::Debian(n.to_string())
        } else {
            DefaultEntry::Other(n.to_string())
        }
    }

    pub fn is_dsm(&self) -> bool {
        matches!(self, DefaultEntry::Dsm(_))
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, DefaultEntry::Unknown(_))
    }

    pub fn label(&self) -> String {
        match self {
            DefaultEntry::Dsm(n) => format!("DSM ({n})"),
            DefaultEntry::Agent(_) => "the boot agent".into(),
            DefaultEntry::Debian(n) => format!("Debian ({n})"),
            DefaultEntry::Other(n) => format!("{n} (not DSM)"),
            DefaultEntry::Unknown(why) => format!("not known ({why})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootEntry {
    pub index: Option<u32>,
    pub name: String,
    /// Added by synofurb (between its markers).
    pub ours: bool,
    pub dsm: bool,
}

/// The unit's own flash backup (`synofurb-dom` writes one before its first write).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlashBackup {
    pub exists: bool,
    pub path: Option<String>,
}

/// What the boot agent says about itself (`sf-boot status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentStatus {
    /// `armed` (watchdog counting), `claimed`, `disabled`, `starting`.
    pub state: String,
    pub watchdog_timeout_min: Option<u32>,
    pub watchdog_remaining_s: Option<i64>,
    /// `ok` (the agent reset the default to DSM), `pending`, `missing`, `failed`, `unknown`.
    pub dom_default: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunningSystem {
    Dsm,
    Debian,
    Agent,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateSource {
    /// `synofurb-dom status --json`: the real boot menu.
    DomTool,
    /// Only `sf-boot status` (the agent): no menu, only what the agent knows.
    AgentOnly,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", content = "detail", rename_all = "snake_case")]
pub enum ReadOutcome {
    /// The boot menu was read.
    Read,
    /// `synofurb-dom` is not on the unit: synofurb never installed the boot agent here, or the copy was removed. The boot menu was not read.
    ToolMissing,
    /// The tool ran and refused (no editable boot flash, not root, ...); the text is its answer.
    Refused(String),
    /// The unit did not answer or the answer could not be understood.
    Failed(String),
}

/// What the unit's boot flash looks like, read from the unit (never from this session's memory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnitBootState {
    pub read: ReadOutcome,
    pub source: StateSource,
    pub running: RunningSystem,
    pub default_entry: DefaultEntry,
    pub entries: Vec<BootEntry>,
    /// Names of the entries synofurb added.
    pub our_entries: Vec<String>,
    /// The agent's kernel and initrd are on the flash. `None` when the menu was not read.
    pub agent_present: Option<bool>,
    pub flash_backup: Option<FlashBackup>,
    pub agent: Option<AgentStatus>,
    pub notes: Vec<String>,
}

impl UnitBootState {
    /// A state that says only that nothing is known.
    pub fn unknown(read: ReadOutcome, why: &str) -> UnitBootState {
        UnitBootState {
            read,
            source: StateSource::Nothing,
            running: RunningSystem::Unknown,
            default_entry: DefaultEntry::Unknown(why.to_string()),
            entries: vec![],
            our_entries: vec![],
            agent_present: None,
            flash_backup: None,
            agent: None,
            notes: vec![],
        }
    }

    /// `Some(true)` only when the menu was read and its default is a DSM entry.
    pub fn default_is_dsm(&self) -> Option<bool> {
        if self.default_entry.is_unknown() { None } else { Some(self.default_entry.is_dsm()) }
    }

    /// `Some(true)`: the unit is in its original state as far as the boot flash goes (menu read, default DSM, none of our entries).
    /// `Some(false)`: something of ours is there or the default is not DSM. `None`: not known, so nothing may be claimed.
    pub fn is_pristine(&self) -> Option<bool> {
        if self.read != ReadOutcome::Read {
            return None;
        }
        Some(self.default_entry.is_dsm() && self.our_entries.is_empty())
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn num_of(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Parse `synofurb-dom status --json`. Lenient like `manage::parse_health`: unknown fields are ignored, a field of the wrong type is
/// skipped (and noted), only a document that is not a JSON object is an error.
pub fn parse_dom_status(text: &str) -> Result<UnitBootState, String> {
    let (a, b) = (text.find('{'), text.rfind('}'));
    let (Some(a), Some(b)) = (a, b) else { return Err("no JSON in the answer".into()) };
    let v: Value = serde_json::from_str(&text[a..=b]).map_err(|e| format!("not valid JSON ({e})"))?;
    if !v.is_object() {
        return Err("the answer is not a JSON object".into());
    }
    let mut notes = vec![];
    let mut entries = vec![];
    match v.get("entries") {
        Some(Value::Array(list)) => {
            for e in list {
                let Some(name) = str_of(e, "name") else {
                    notes.push("a menu entry without a name was skipped".to_string());
                    continue;
                };
                let ours = e
                    .get("ours")
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| name == "SYNOFURB_BOOT" || name == "DEBIAN" || name == "DEBIAN_OLD" || name.starts_with("EFI_"));
                let dsm = e.get("dsm").and_then(Value::as_bool).unwrap_or_else(|| name.starts_with("SYNOLOGY_"));
                entries.push(BootEntry { index: e.get("index").and_then(num_of), name: name.to_string(), ours, dsm });
            }
        }
        _ => notes.push("the answer has no list of menu entries".to_string()),
    }
    let default_entry = match v.get("default") {
        Some(d) => match str_of(d, "name").map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => DefaultEntry::from_name(n),
            None => match d.get("index").and_then(num_of).and_then(|i| entries.iter().find(|e| e.index == Some(i))) {
                Some(e) => DefaultEntry::from_name(&e.name),
                None => DefaultEntry::Unknown("the boot menu names no valid default entry".into()),
            },
        },
        None => DefaultEntry::Unknown("the answer has no default entry".into()),
    };
    let our_entries: Vec<String> = entries.iter().filter(|e| e.ours).map(|e| e.name.clone()).collect();
    let agent_present = match v.get("files") {
        Some(Value::Array(f)) => {
            let has = |n: &str| f.iter().any(|x| str_of(x, "name").is_some_and(|m| m.ends_with(n)));
            Some(has("agent.vmlinuz") && has("agent.initrd"))
        }
        _ => {
            notes.push("the answer lists no files of ours".to_string());
            None
        }
    };
    let flash_backup = v.get("backup").map(|b| FlashBackup {
        exists: b.get("exists").and_then(Value::as_bool).unwrap_or(false),
        path: str_of(b, "path").filter(|p| !p.is_empty()).map(str::to_string),
    });
    Ok(UnitBootState {
        read: ReadOutcome::Read,
        source: StateSource::DomTool,
        running: RunningSystem::Unknown,
        default_entry,
        entries,
        our_entries,
        agent_present,
        flash_backup,
        agent: None,
        notes,
    })
}

/// Parse `sf-boot status` (key=value lines).
pub fn parse_agent_status(text: &str) -> Option<AgentStatus> {
    let mut kv = std::collections::BTreeMap::new();
    for l in text.lines() {
        if let Some((k, v)) = l.split_once('=') {
            kv.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if kv.get("agent").map(String::as_str) != Some("synofurb-agent") {
        return None;
    }
    Some(AgentStatus {
        state: kv.get("state").cloned().unwrap_or_default(),
        watchdog_timeout_min: kv.get("watchdog_timeout_min").and_then(|s| s.parse().ok()),
        watchdog_remaining_s: kv.get("watchdog_remaining_s").and_then(|s| s.parse().ok()),
        dom_default: kv.get("dom_default").cloned().unwrap_or_else(|| "unknown".into()),
    })
}

/// Find `synofurb-dom` on the unit and run it with `args`. Exit 127 and "not installed" when there is none.
fn dom_tool_command(args: &str) -> String {
    format!(
        "T=$(command -v synofurb-dom 2>/dev/null); \
         if [ -z \"$T\" ]; then for c in /usr/local/bin/synofurb-dom /usr/bin/synofurb-dom /usr/local/sbin/synofurb-dom; do [ -f \"$c\" ] && T=$c && break; done; fi; \
         [ -n \"$T\" ] || {{ echo 'synofurb-dom: not installed'; exit 127; }}; /bin/sh \"$T\" {args} 2>&1"
    )
}

const DETECT_SYSTEM: &str = "if command -v sf-boot >/dev/null 2>&1; then echo agent; elif [ -f /etc/synoinfo.conf ] || [ -f /etc.defaults/synoinfo.conf ]; then echo dsm; elif [ -f /etc/debian_version ]; then echo debian; else echo unknown; fi";

fn classify_system(out: &str) -> RunningSystem {
    match out.trim() {
        "agent" => RunningSystem::Agent,
        "dsm" => RunningSystem::Dsm,
        "debian" => RunningSystem::Debian,
        _ => RunningSystem::Unknown,
    }
}

/// Read the unit's boot state over an open session. Never fails: a unit that does not answer gives a state whose [`ReadOutcome`] says
/// so and whose default entry is [`DefaultEntry::Unknown`].
pub fn read_boot_state(p: &mut Privileged<'_>) -> UnitBootState {
    let running = match p.sh.exec(&format!("/bin/sh -c '{DETECT_SYSTEM}'"), None) {
        Ok(r) if r.ok() => classify_system(&r.stdout),
        _ => RunningSystem::Unknown,
    };
    let agent = if running == RunningSystem::Agent {
        p.run("sf-boot status").ok().filter(ExecResult::ok).and_then(|r| parse_agent_status(&r.stdout))
    } else {
        None
    };
    let mut st = match p.run(&dom_tool_command("status --json")) {
        Err(e) => UnitBootState::unknown(ReadOutcome::Failed(format!("{e:#}")), "the unit did not answer"),
        Ok(r) if r.code == 127 => UnitBootState::unknown(ReadOutcome::ToolMissing, "synofurb-dom is not on the unit"),
        Ok(r) if !r.ok() => {
            let t = r.stdout.trim().to_string();
            UnitBootState::unknown(ReadOutcome::Refused(t), "the boot flash tool refused to report")
        }
        Ok(r) => match parse_dom_status(&r.stdout) {
            Ok(s) => s,
            Err(why) => UnitBootState::unknown(ReadOutcome::Failed(why.clone()), &why),
        },
    };
    st.running = running;
    if let Some(a) = agent {
        // without the menu the agent's own account of the default is the only evidence
        if st.source == StateSource::Nothing {
            st.source = StateSource::AgentOnly;
            st.default_entry = match a.dom_default.as_str() {
                "ok" => DefaultEntry::Dsm("reset by the agent".into()),
                "pending" => DefaultEntry::Unknown("the agent has not reset the default yet".into()),
                "failed" => DefaultEntry::Unknown("the agent could not reset the default".into()),
                "missing" => DefaultEntry::Unknown("the agent has no synofurb-dom".into()),
                other => DefaultEntry::Unknown(format!("agent reports '{other}'")),
            };
        }
        st.agent = Some(a);
    }
    st
}

/// Connect with `t`, find out whether the user is root, run `f`.
fn with_privileged<R>(t: &SshTarget, log: &Logger, f: impl FnOnce(&mut Privileged<'_>) -> R) -> Result<R> {
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, log)?;
    let r = with_privileged_on(&mut ssh, t.sudo_password.clone(), f);
    ssh.close();
    r
}

fn with_privileged_on<R>(sh: &mut dyn RemoteShell, sudo: Option<String>, f: impl FnOnce(&mut Privileged<'_>) -> R) -> Result<R> {
    let id = sh.exec("id -u", None)?;
    let mut p = Privileged { sh, is_root: id.stdout.trim() == "0", sudo_password: sudo };
    Ok(f(&mut p))
}

/// Connect to the unit and read its boot state (DSM, the installed Debian or the boot agent). An unreachable unit is an `Err`; a
/// reachable one whose boot flash cannot be read is an `Ok` state that says so.
pub fn read_unit_boot_state(t: &SshTarget, log: &Logger) -> Result<UnitBootState> {
    with_privileged(t, log, read_boot_state)
}

/// Flash backups on this computer that belong to `unit` (the serial number or host name `backup_dom` used), newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalBackup {
    pub path: PathBuf,
    pub bytes: u64,
    /// Hash from the `.img.sha256` file next to it, if there is one.
    pub recorded_sha256: Option<String>,
    pub modified: Option<String>,
}

pub fn find_local_backups(dir: &Path, unit: &str) -> Vec<LocalBackup> {
    let safe: String = unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    let prefix = format!("synofurb-dom-backup-{safe}-");
    let mut v: Vec<LocalBackup> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(&prefix) || !name.ends_with(".img") {
                return None;
            }
            let md = e.metadata().ok().filter(|m| m.is_file())?;
            let sum = std::fs::read_to_string(e.path().with_extension("img.sha256"))
                .ok()
                .and_then(|t| t.split_whitespace().next().map(str::to_ascii_lowercase))
                .filter(|h| h.len() == 64);
            let modified =
                md.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            Some(LocalBackup { path: e.path(), bytes: md.len(), recorded_sha256: sum, modified })
        })
        .collect();
    v.sort_by(|a, b| b.path.cmp(&a.path)); // the file name carries the timestamp
    v
}

// ---------------------------------------------------------------------------------------------- actions

/// Why a recovery action did not (fully) happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryError {
    /// The caller did not pass `confirmed = true`; nothing was done.
    #[error("{action} was not done: it changes the boot flash or reboots the unit and was not confirmed")]
    NotConfirmed { action: &'static str },
    /// The backup file cannot be used; nothing was sent to the unit.
    #[error("the backup file {path} cannot be used: {reason}")]
    BackupUnusable { path: String, reason: String },
    /// `synofurb-dom` is not on the unit.
    #[error(
        "synofurb-dom is not installed on the unit, so the boot flash cannot be changed from here. Use the manual recovery in docs/RECOVERY.md (the boot loader menu on a console, or the flash in another Linux computer)"
    )]
    ToolMissing,
    /// The tool ran and said no. Its output is included.
    #[error("synofurb-dom refused to {action}: {output}")]
    ToolRefused { action: &'static str, output: String },
    /// The file that arrived on the unit is not the file that was sent.
    #[error("the backup file did not arrive intact (sent {local}, the unit has {remote}); the boot flash was not touched")]
    UploadMismatch { local: String, remote: String },
    /// The action ran but the result could not be confirmed by reading the unit again. `flash_may_have_changed` is the honest summary.
    #[error("{what} could not be confirmed afterwards: {why}")]
    Unverified { what: &'static str, why: String, flash_may_have_changed: bool },
    /// The unit could not be reached or the connection broke.
    #[error("the unit could not be reached: {0}")]
    Transport(String),
}

impl RecoveryError {
    fn transport(e: impl std::fmt::Display) -> RecoveryError {
        RecoveryError::Transport(format!("{e:#}"))
    }
}

fn need(confirmed: bool, action: &'static str) -> Result<(), RecoveryError> {
    if confirmed { Ok(()) } else { Err(RecoveryError::NotConfirmed { action }) }
}

fn tool_result(r: ExecResult, action: &'static str) -> Result<String, RecoveryError> {
    match r.code {
        0 => Ok(r.stdout),
        127 => Err(RecoveryError::ToolMissing),
        _ => Err(RecoveryError::ToolRefused { action, output: format!("{} {}", r.stdout.trim(), r.stderr.trim()).trim().to_string() }),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentEntered {
    pub address: std::net::IpAddr,
    pub ssh_key: PathBuf,
    pub unit: String,
    /// The command that opens a shell on the agent.
    pub ssh_command: String,
}

/// Arm the boot agent for the next boot, reboot the unit and wait until the agent answers. The unit reboots: `confirmed` must be true.
/// If the agent does not answer in time the error is [`crate::session::SessionError::AgentTimeout`] and nothing is reset.
pub fn enter_agent(t: &SshTarget, cancel: &AtomicBool, confirmed: bool, log: &Logger) -> Result<AgentEntered> {
    need(confirmed, "booting into the agent")?;
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, log)?;
    let fp = crate::remote::RemoteShell::host_key_fingerprint(&ssh);
    let (facts, _) = crate::session::inspect_remote(&mut ssh, false, t.sudo_password.clone(), log)?;
    let unit = if facts.serial.is_empty() { t.host.clone() } else { facts.serial.clone() };
    let key = crate::session::agent_key_for(&unit, &config_keys_dir())?;
    let mut p = Privileged { sh: &mut ssh, is_root: facts.is_root, sudo_password: t.sudo_password.clone() };
    let r = p.run(&dom_tool_command("default agent --yes"))?;
    if !r.ok() {
        anyhow::bail!("could not arm the agent (is it installed? run `boot-manager install` first): {}", r.stdout.trim());
    }
    if cancel.load(Ordering::Relaxed) {
        let _ = crate::domboot::default_to_dsm(&mut ssh, facts.is_root, t.sudo_password.clone());
        anyhow::bail!("cancelled before the reboot; the default boot entry is DSM again");
    }
    let mut p = Privileged { sh: &mut ssh, is_root: facts.is_root, sudo_password: t.sudo_password.clone() };
    let _ = p.run("nohup /bin/sh -c 'sleep 2; sync; reboot' >/dev/null 2>&1 &");
    let ip: std::net::IpAddr = t.host.parse().context("host must be an IP address for this command")?;
    let (agent, ip) = crate::domboot::wait_for_agent(
        &[ip],
        &key.private_openssh,
        fp.as_deref(),
        crate::domboot::AgentWait::standard(crate::timing::BOOT_MANAGER_WATCHDOG_MIN),
        cancel,
        log,
    )?;
    drop(agent);
    let safe: String = unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    let ssh_key = config_keys_dir().join(format!("agent_{safe}_ed25519"));
    let ssh_command = format!("ssh -i {} root@{ip}", ssh_key.display());
    Ok(AgentEntered { address: ip, ssh_key, unit, ssh_command })
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreOutcome {
    pub backup: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    /// The `.img.sha256` file next to the backup agreed with the file. `false` = there was none to compare with.
    pub checked_against_sidecar: bool,
    /// `synofurb-dom restore` read the flash back and compared it; this is its verdict (exit code 0).
    pub readback_verified: bool,
    pub state_after: UnitBootState,
}

fn hash_file(p: &Path) -> std::io::Result<(String, u64)> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p)?;
    let (mut h, mut n, mut buf) = (Sha256::new(), 0u64, vec![0u8; 1 << 20]);
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64;
    }
    Ok((hex::encode(h.finalize()), n))
}

const MIN_FLASH_BYTES: u64 = 8 << 20;
const MAX_FLASH_BYTES: u64 = 4 << 30;

/// Check a backup file before anything is sent: a regular file of a plausible size whose hash agrees with its `.sha256` sidecar.
pub fn check_backup_file(path: &Path) -> Result<(String, u64, bool), RecoveryError> {
    let bad = |reason: String| RecoveryError::BackupUnusable { path: path.display().to_string(), reason };
    let md = std::fs::symlink_metadata(path).map_err(|e| bad(e.to_string()))?;
    if !md.is_file() {
        return Err(bad("it is not a regular file".into()));
    }
    if !(MIN_FLASH_BYTES..=MAX_FLASH_BYTES).contains(&md.len()) || md.len() % 512 != 0 {
        return Err(bad(format!("{} bytes is not the size of a boot flash image", md.len())));
    }
    let (sha, bytes) = hash_file(path).map_err(|e| bad(e.to_string()))?;
    let sidecar = path.with_extension("img.sha256");
    let mut checked = false;
    if let Ok(t) = std::fs::read_to_string(&sidecar) {
        let want = t.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if want != sha {
            return Err(bad(format!("its checksum differs from {} (the file is damaged or was changed)", sidecar.display())));
        }
        checked = true;
    }
    Ok((sha, bytes, checked))
}

/// Write a saved flash image back to the unit's boot flash through `synofurb-dom restore` on the unit. Only works while the unit is
/// reachable over SSH (DSM, the installed Debian or the agent); a flash that left the unit is restored on another computer (docs/RECOVERY.md).
/// Steps: check the file here, upload it to a private directory on the unit, compare hashes there, run the tool (which validates the
/// image, snapshots the current flash, writes, reads back and compares), read the unit's state again. Needs `confirmed`.
pub fn restore_flash(t: &SshTarget, backup: &Path, confirmed: bool, log: &Logger) -> Result<RestoreOutcome, RecoveryError> {
    need(confirmed, "restoring the boot flash")?;
    let checks = check_backup_file(backup)?;
    with_privileged(t, log, |p| restore_flash_on(p, backup, checks, log)).map_err(RecoveryError::transport)?
}

/// [`restore_flash`] on an open session (tests use a scripted one). The file has been checked.
pub fn restore_flash_on(
    p: &mut Privileged<'_>,
    backup: &Path,
    checks: (String, u64, bool),
    log: &Logger,
) -> Result<RestoreOutcome, RecoveryError> {
    let (sha, bytes, sidecar) = checks;
    let dir = format!("/tmp/.synofurb-restore-{}", crate::config::random_token(5));
    let remote = format!("{dir}/flash.img");
    let cleanup = |p: &mut Privileged<'_>| {
        let _ = p.run(&format!("rm -rf '{dir}'"));
    };
    p.run(&format!("mkdir -p '{dir}' && chmod 700 '{dir}'")).map_err(RecoveryError::transport)?;
    log.info(format!("uploading the saved boot flash image ({} MiB) to the unit", bytes / 1048576));
    let mut f = std::fs::File::open(backup).map_err(RecoveryError::transport)?;
    if let Err(e) = p.sh.upload(&remote, &mut f, bytes, 0o600, &mut |_| {}) {
        cleanup(p);
        return Err(RecoveryError::transport(e));
    }
    let remote_sha = p
        .run(&format!("sha256sum '{remote}' 2>/dev/null"))
        .ok()
        .and_then(|r| r.stdout.split_whitespace().next().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if remote_sha != sha {
        cleanup(p);
        return Err(RecoveryError::UploadMismatch {
            local: sha,
            remote: if remote_sha.is_empty() { "no checksum".into() } else { remote_sha },
        });
    }
    log.warn(
        "writing the saved image to the boot flash (synofurb-dom restore: validates, snapshots the current flash, writes, reads back)",
    );
    let r = p.run(&dom_tool_command(&format!("restore --yes '{remote}'")));
    cleanup(p);
    let out = match r {
        Ok(r) => tool_result(r, "restore the boot flash")?,
        Err(e) => return Err(RecoveryError::transport(e)),
    };
    log.info(out.trim());
    let state_after = read_boot_state(p);
    Ok(RestoreOutcome {
        backup: backup.to_path_buf(),
        bytes,
        sha256: sha,
        checked_against_sidecar: sidecar,
        readback_verified: true,
        state_after,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoveOutcome {
    pub output: String,
    pub state_after: UnitBootState,
}

/// Remove everything synofurb added to the boot menu and set the default to DSM (`synofurb-dom remove`). Afterwards the unit is read
/// again: success is only reported when the menu was read and shows no entry of ours and a DSM default. Needs `confirmed`.
pub fn remove_our_entries(t: &SshTarget, confirmed: bool, log: &Logger) -> Result<RemoveOutcome, RecoveryError> {
    need(confirmed, "removing synofurb's boot entries")?;
    with_privileged(t, log, |p| remove_our_entries_on(p, log)).map_err(RecoveryError::transport)?
}

pub fn remove_our_entries_on(p: &mut Privileged<'_>, log: &Logger) -> Result<RemoveOutcome, RecoveryError> {
    let out = tool_result(p.run(&dom_tool_command("remove --yes")).map_err(RecoveryError::transport)?, "remove the entries")?;
    log.info(out.trim());
    let state_after = read_boot_state(p);
    match (&state_after.read, state_after.our_entries.is_empty(), state_after.default_entry.is_dsm()) {
        (ReadOutcome::Read, true, true) => Ok(RemoveOutcome { output: out, state_after }),
        (ReadOutcome::Read, false, _) => Err(RecoveryError::Unverified {
            what: "removing the entries",
            why: format!("the boot menu still lists {}", state_after.our_entries.join(", ")),
            flash_may_have_changed: true,
        }),
        (ReadOutcome::Read, _, false) => Err(RecoveryError::Unverified {
            what: "removing the entries",
            why: format!("the default boot entry is {}", state_after.default_entry.label()),
            flash_may_have_changed: true,
        }),
        (other, _, _) => Err(RecoveryError::Unverified {
            what: "removing the entries",
            why: format!("the boot menu could not be read afterwards ({other:?})"),
            flash_may_have_changed: true,
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BootDsmOutcome {
    /// The default boot entry was read back and is DSM.
    pub default_is_dsm: bool,
    /// A reboot command was sent. **Only that**: it does not mean the unit rebooted or that DSM started. Use [`wait_for_dsm`].
    pub reboot_requested: bool,
    pub state_after: UnitBootState,
    /// One honest sentence for the user.
    pub message: String,
}

/// Set the default boot entry to DSM, read it back, and (only when `reboot` is true) ask the unit to reboot. Setting the default does
/// not reboot anything. With `reboot` the call needs `confirmed` and still reports only "reboot requested"; if the default could not be
/// verified as DSM no reboot is sent.
pub fn boot_dsm(t: &SshTarget, reboot: bool, confirmed: bool, log: &Logger) -> Result<BootDsmOutcome, RecoveryError> {
    if reboot {
        need(confirmed, "rebooting the unit into DSM")?;
    }
    with_privileged(t, log, |p| boot_dsm_on(p, reboot, log)).map_err(RecoveryError::transport)?
}

pub fn boot_dsm_on(p: &mut Privileged<'_>, reboot: bool, log: &Logger) -> Result<BootDsmOutcome, RecoveryError> {
    let out = tool_result(p.run(&dom_tool_command("default dsm --yes")).map_err(RecoveryError::transport)?, "set the default to DSM")?;
    log.info(out.trim());
    let state_after = read_boot_state(p);
    let default_is_dsm = state_after.read == ReadOutcome::Read && state_after.default_entry.is_dsm();
    if !default_is_dsm {
        return Err(RecoveryError::Unverified {
            what: "setting the default boot entry to DSM",
            why: format!("reading the boot menu afterwards says the default is {}", state_after.default_entry.label()),
            flash_may_have_changed: true,
        });
    }
    if !reboot {
        return Ok(BootDsmOutcome {
            default_is_dsm,
            reboot_requested: false,
            state_after,
            message: "The default boot entry is DSM. The unit was not rebooted: it keeps running what it runs now until you reboot it."
                .into(),
        });
    }
    let sent = p.run("nohup /bin/sh -c 'sleep 2; sync; reboot' >/dev/null 2>&1 &");
    log.warn("reboot requested; DSM is back only when it answers (see wait_for_dsm)");
    match sent {
        Ok(_) => Ok(BootDsmOutcome {
            default_is_dsm,
            reboot_requested: true,
            state_after,
            message: "Reboot requested. This only means the command was sent: DSM is back when it answers, and that is checked separately."
                .into(),
        }),
        Err(e) => Err(RecoveryError::transport(e)),
    }
}

// ---------------------------------------------------------------------------------------------- is DSM back?

/// What answers at the unit's address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProbeResult {
    /// SSH answers like DSM's OpenSSH and DSM's web port answers.
    DsmBack { ssh_banner: String, web_port: u16 },
    /// DSM's web port answers but SSH does not. Heuristic: the loader runs but DSM's operating system is not (a unit that lost its DSM
    /// partition shows the install page there), **or** DSM is still starting. Keep waiting a few minutes before you conclude.
    LoaderOnly { web_port: u16 },
    /// The boot agent answers (its SSH is dropbear): the unit is in the agent, not in DSM.
    AgentStillUp { ssh_banner: String },
    /// SSH answers (OpenSSH) but DSM's web port does not: some other system (the installed Debian, a DSM with other ports).
    SshOnly { ssh_banner: String },
    /// Nothing answers on the probed ports.
    Unreachable,
}

impl ProbeResult {
    pub fn is_dsm_back(&self) -> bool {
        matches!(self, ProbeResult::DsmBack { .. })
    }
}

#[derive(Debug, Clone)]
pub struct ProbeOptions {
    pub ssh_port: u16,
    /// DSM's web ports: 5000 (HTTP), 5001 (HTTPS).
    pub web_ports: Vec<u16>,
    pub connect_timeout: Duration,
    pub banner_timeout: Duration,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        ProbeOptions {
            ssh_port: 22,
            web_ports: vec![5000, 5001],
            connect_timeout: Duration::from_secs(2),
            banner_timeout: Duration::from_secs(3),
        }
    }
}

fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    (host, port).to_socket_addrs().ok()?.next()
}

fn port_open(host: &str, port: u16, to: Duration) -> bool {
    resolve(host, port).is_some_and(|a| TcpStream::connect_timeout(&a, to).is_ok())
}

/// The SSH identification line the server sends first (`SSH-2.0-OpenSSH_9.6`), if it sends one in time.
fn ssh_banner(host: &str, port: u16, o: &ProbeOptions) -> Option<String> {
    let mut s = TcpStream::connect_timeout(&resolve(host, port)?, o.connect_timeout).ok()?;
    s.set_read_timeout(Some(o.banner_timeout)).ok()?;
    let mut buf = [0u8; 255];
    let mut got = vec![];
    let deadline = Instant::now() + o.banner_timeout;
    while Instant::now() < deadline && !got.contains(&b'\n') && got.len() < 255 {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    let line = String::from_utf8_lossy(&got).lines().next()?.trim().to_string();
    line.starts_with("SSH-").then_some(line)
}

/// One look at the unit's address. Pure network reads with short timeouts; nothing is sent besides opening the connections.
pub fn probe_unit(host: &str, o: &ProbeOptions) -> ProbeResult {
    let web = o.web_ports.iter().copied().find(|p| port_open(host, *p, o.connect_timeout));
    match ssh_banner(host, o.ssh_port, o) {
        Some(b) if b.to_ascii_lowercase().contains("dropbear") => ProbeResult::AgentStillUp { ssh_banner: b },
        Some(b) => match web {
            Some(web_port) => ProbeResult::DsmBack { ssh_banner: b, web_port },
            None => ProbeResult::SshOnly { ssh_banner: b },
        },
        None => match web {
            Some(web_port) => ProbeResult::LoaderOnly { web_port },
            None => ProbeResult::Unreachable,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DsmWait {
    /// The last thing seen. `DsmBack` when the wait succeeded.
    pub last: ProbeResult,
    pub waited_s: u64,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Probe every few seconds until DSM answers, `timeout` passes or `cancel` is set. `on_state` is called after every probe with the result
/// and the time waited (drive a clock and the "waiting / DSM is back / DSM did not return / loader only" states from it).
pub fn wait_for_dsm(
    host: &str,
    timeout: Duration,
    cancel: &AtomicBool,
    opts: &ProbeOptions,
    poll: Duration,
    on_state: &mut dyn FnMut(&ProbeResult, Duration),
) -> DsmWait {
    let start = Instant::now();
    loop {
        let r = probe_unit(host, opts);
        on_state(&r, start.elapsed());
        if r.is_dsm_back() {
            return DsmWait { last: r, waited_s: start.elapsed().as_secs(), timed_out: false, cancelled: false };
        }
        if cancel.load(Ordering::Relaxed) {
            return DsmWait { last: r, waited_s: start.elapsed().as_secs(), timed_out: false, cancelled: true };
        }
        if start.elapsed() >= timeout {
            return DsmWait { last: r, waited_s: start.elapsed().as_secs(), timed_out: true, cancelled: false };
        }
        let until = (Instant::now() + poll).min(start + timeout);
        while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::mock::MockShell;
    use std::io::Write;

    const DOM_JSON: &str = r#"{"tool":"synofurb-dom","version":"1.0","device":"/dev/synoboot","device_bytes":134217728,
      "p1":{"dev":"/dev/synoboot1","bytes":20971520,"free_bytes":1,"fat_volume_id":"10EE-589C"},
      "p2":{"dev":"/dev/synoboot2","bytes":1,"free_bytes":1,"uuid":"45e5b07d"},
      "entries":[{"index":0,"name":"SYNOLOGY_1","ours":false,"dsm":true},{"index":1,"name":"SYNOLOGY_2","ours":false,"dsm":true},
                 {"index":2,"name":"SYNOFURB_BOOT","ours":true,"dsm":false}],
      "default":{"raw":"2","index":2,"name":"SYNOFURB_BOOT"},
      "files":[{"name":"agent.vmlinuz","size":1,"sha256":"a","installed":"d"},{"name":"agent.initrd","size":1,"sha256":"a","installed":"d"}],
      "efi_files":[],"backup":{"exists":true,"path":"/var/lib/synofurb/synofurb-dom-first-backup.img"}}"#;

    fn dsm_json() -> String {
        let mut v: Value = serde_json::from_str(DOM_JSON).unwrap();
        v["default"] = serde_json::json!({"raw":"1","index":1,"name":"SYNOLOGY_2"});
        v.to_string()
    }

    fn clean_json() -> String {
        // DSM default and none of our entries or files
        let mut v: Value = serde_json::from_str(&dsm_json()).unwrap();
        v["entries"].as_array_mut().unwrap().truncate(2);
        v["files"] = serde_json::json!([]);
        v.to_string()
    }

    #[test]
    fn dom_status_is_parsed_and_the_default_is_classified() {
        let s = parse_dom_status(DOM_JSON).unwrap();
        assert_eq!(s.read, ReadOutcome::Read);
        assert_eq!(s.default_entry, DefaultEntry::Agent("SYNOFURB_BOOT".into()));
        assert_eq!(s.our_entries, vec!["SYNOFURB_BOOT"]);
        assert_eq!(s.agent_present, Some(true));
        assert_eq!(s.flash_backup.as_ref().map(|b| b.exists), Some(true));
        assert_eq!(s.is_pristine(), Some(false));
        assert_eq!(s.default_is_dsm(), Some(false));
        let d = parse_dom_status(&dsm_json()).unwrap();
        assert!(d.default_entry.is_dsm() && d.default_entry.label().contains("SYNOLOGY_2"));
        assert_eq!(d.is_pristine(), Some(false), "our entry is still in the menu");
        let c = parse_dom_status(&clean_json()).unwrap();
        assert_eq!((c.is_pristine(), c.agent_present), (Some(true), Some(false)));
    }

    #[test]
    fn a_default_by_index_only_and_missing_fields_are_handled_leniently() {
        let t = r#"{"entries":[{"index":"0","name":"SYNOLOGY_1"},{"index":1,"name":"DEBIAN"},{"bogus":1}],"default":{"index":1},"future_field":[1,2]}"#;
        let s = parse_dom_status(&format!("noise\n{t}\nmore noise")).unwrap();
        assert_eq!(s.default_entry, DefaultEntry::Debian("DEBIAN".into()));
        assert_eq!(s.our_entries, vec!["DEBIAN"], "ownership falls back to the entry name");
        assert!(s.notes.iter().any(|n| n.contains("without a name")) && s.agent_present.is_none());
        // no usable default is UNKNOWN, never DSM
        for t in [
            r#"{"entries":[]}"#,
            r#"{"entries":[],"default":{}}"#,
            r#"{"entries":[{"index":0,"name":"SYNOLOGY_1"}],"default":{"index":7}}"#,
        ] {
            let s = parse_dom_status(t).unwrap();
            assert!(s.default_entry.is_unknown() && s.default_is_dsm().is_none(), "{t}");
        }
        assert!(parse_dom_status("not json").is_err() && parse_dom_status("[1]").is_err() && parse_dom_status("{broken").is_err());
    }

    #[test]
    fn agent_status_lines_are_parsed() {
        let t = "agent=synofurb-agent\nstate=armed\nwatchdog_timeout_min=30\nwatchdog_remaining_s=1500\ndom_default=ok\nkernel=6.12\n";
        let a = parse_agent_status(t).unwrap();
        assert_eq!(
            (a.state.as_str(), a.watchdog_timeout_min, a.watchdog_remaining_s, a.dom_default.as_str()),
            ("armed", Some(30), Some(1500), "ok")
        );
        assert!(parse_agent_status("state=armed\n").is_none());
    }

    fn priv_with(m: &mut MockShell) -> Privileged<'_> {
        Privileged { sh: m, is_root: true, sudo_password: None }
    }

    #[test]
    fn reading_the_state_distinguishes_unknown_from_dsm() {
        // 1. the menu was read
        let mut m = MockShell::default();
        m.push_ok("dsm\n");
        m.push_ok(&dsm_json());
        let s = read_boot_state(&mut priv_with(&mut m));
        assert_eq!((s.running, s.default_is_dsm()), (RunningSystem::Dsm, Some(true)));
        assert!(m.commands[1].contains("synofurb-dom") && m.commands[1].contains("status --json"));
        // 2. the tool is not on the unit: not DSM, UNKNOWN
        let mut m = MockShell::default();
        m.push_ok("dsm\n");
        m.results.push_back(ExecResult { code: 127, stdout: "synofurb-dom: not installed\n".into(), stderr: String::new() });
        let s = read_boot_state(&mut priv_with(&mut m));
        assert_eq!(s.read, ReadOutcome::ToolMissing);
        assert!(s.default_entry.is_unknown() && s.default_is_dsm().is_none() && s.is_pristine().is_none(), "{s:?}");
        // 3. the tool refuses
        let mut m = MockShell::default();
        m.push_ok("debian\n");
        m.results.push_back(ExecResult { code: 1, stdout: "no DOM found\n".into(), stderr: String::new() });
        let s = read_boot_state(&mut priv_with(&mut m));
        assert_eq!(s.read, ReadOutcome::Refused("no DOM found".into()));
        assert_eq!(s.running, RunningSystem::Debian);
        // 4. garbage
        let mut m = MockShell::default();
        m.push_ok("dsm\n");
        m.push_ok("<html>");
        assert!(matches!(read_boot_state(&mut priv_with(&mut m)).read, ReadOutcome::Failed(_)));
    }

    #[test]
    fn on_the_agent_its_own_status_fills_in_when_the_menu_cannot_be_read() {
        let mut m = MockShell::default();
        m.push_ok("agent\n");
        m.push_ok("agent=synofurb-agent\nstate=armed\nwatchdog_timeout_min=60\ndom_default=ok\n");
        m.results.push_back(ExecResult { code: 127, stdout: String::new(), stderr: String::new() });
        let s = read_boot_state(&mut priv_with(&mut m));
        assert_eq!((s.running, s.source), (RunningSystem::Agent, StateSource::AgentOnly));
        assert!(s.default_entry.is_dsm() && s.agent.as_ref().unwrap().watchdog_timeout_min == Some(60));
        assert_eq!(s.is_pristine(), None, "the menu itself was not read, so pristine is not claimed");
        // 'pending' / 'failed' mean the agent has not (or could not) reset the default: unknown
        for v in ["pending", "failed", "missing"] {
            let mut m = MockShell::default();
            m.push_ok("agent\n");
            m.push_ok(&format!("agent=synofurb-agent\nstate=armed\ndom_default={v}\n"));
            m.results.push_back(ExecResult { code: 127, stdout: String::new(), stderr: String::new() });
            assert!(read_boot_state(&mut priv_with(&mut m)).default_entry.is_unknown(), "{v}");
        }
    }

    #[test]
    fn actions_refuse_without_confirmation_and_do_not_touch_the_unit() {
        let t = SshTarget {
            host: "127.0.0.1".into(),
            port: 1,
            user: "root".into(),
            auth: crate::remote::Auth::Password("x".into()),
            sudo_password: None,
        };
        let log = Logger::memory(None);
        // each returns before connecting (port 1 would be a Transport error)
        assert!(matches!(remove_our_entries(&t, false, &log).unwrap_err(), RecoveryError::NotConfirmed { .. }));
        assert!(matches!(boot_dsm(&t, true, false, &log).unwrap_err(), RecoveryError::NotConfirmed { .. }));
        assert!(matches!(restore_flash(&t, Path::new("/nope.img"), false, &log).unwrap_err(), RecoveryError::NotConfirmed { .. }));
        let e = enter_agent(&t, &AtomicBool::new(false), false, &log).unwrap_err();
        assert!(matches!(e.downcast_ref::<RecoveryError>(), Some(RecoveryError::NotConfirmed { .. })));
        // setting the default without a reboot is not destructive and may run unconfirmed (here: fails at the connection)
        assert!(matches!(boot_dsm(&t, false, false, &log).unwrap_err(), RecoveryError::Transport(_)));
    }

    #[test]
    fn boot_dsm_verifies_the_default_before_it_reboots_and_only_says_requested() {
        // default set and verified, no reboot
        let mut m = MockShell::default();
        m.push_ok("default set to SYNOLOGY_2\n"); // default dsm
        m.push_ok("dsm\n");
        m.push_ok(&dsm_json());
        let o = boot_dsm_on(&mut priv_with(&mut m), false, &Logger::memory(None)).unwrap();
        assert!(o.default_is_dsm && !o.reboot_requested && o.message.contains("not rebooted"));
        assert!(!m.commands.iter().any(|c| c.contains("reboot")));
        // with the reboot: sent last, worded as a request
        let mut m = MockShell::default();
        m.push_ok("ok\n");
        m.push_ok("dsm\n");
        m.push_ok(&dsm_json());
        let o = boot_dsm_on(&mut priv_with(&mut m), true, &Logger::memory(None)).unwrap();
        assert!(o.reboot_requested && o.message.starts_with("Reboot requested") && o.message.contains("only means the command was sent"));
        assert!(!o.message.to_lowercase().contains("dsm is back.") && m.commands.last().unwrap().contains("reboot"));
        // the menu says the default is still the agent: error, and NO reboot command
        let mut m = MockShell::default();
        m.push_ok("ok\n");
        m.push_ok("dsm\n");
        m.push_ok(DOM_JSON);
        let e = boot_dsm_on(&mut priv_with(&mut m), true, &Logger::memory(None)).unwrap_err();
        assert!(matches!(e, RecoveryError::Unverified { flash_may_have_changed: true, .. }), "{e}");
        assert!(!m.commands.iter().any(|c| c.contains("sleep 2; sync; reboot")), "never reboot into a default that was not verified");
        // the tool is missing / refuses
        let mut m = MockShell::default();
        m.results.push_back(ExecResult { code: 127, stdout: String::new(), stderr: String::new() });
        assert_eq!(boot_dsm_on(&mut priv_with(&mut m), false, &Logger::memory(None)).unwrap_err(), RecoveryError::ToolMissing);
        let mut m = MockShell::default();
        m.results.push_back(ExecResult { code: 1, stdout: "no DOM\n".into(), stderr: String::new() });
        assert!(matches!(
            boot_dsm_on(&mut priv_with(&mut m), false, &Logger::memory(None)).unwrap_err(),
            RecoveryError::ToolRefused { .. }
        ));
    }

    #[test]
    fn remove_reports_success_only_when_the_menu_is_clean_afterwards() {
        let mut m = MockShell::default();
        m.push_ok("removed\n");
        m.push_ok("dsm\n");
        m.push_ok(&clean_json());
        let o = remove_our_entries_on(&mut priv_with(&mut m), &Logger::memory(None)).unwrap();
        assert_eq!(o.state_after.is_pristine(), Some(true));
        assert!(m.commands[0].contains("remove --yes"));
        // still listed
        let mut m = MockShell::default();
        m.push_ok("removed\n");
        m.push_ok("dsm\n");
        m.push_ok(&dsm_json());
        let e = remove_our_entries_on(&mut priv_with(&mut m), &Logger::memory(None)).unwrap_err();
        assert!(matches!(&e, RecoveryError::Unverified { why, .. } if why.contains("SYNOFURB_BOOT")), "{e}");
        // cannot read afterwards
        let mut m = MockShell::default();
        m.push_ok("removed\n");
        m.push_ok("dsm\n");
        m.results.push_back(ExecResult { code: 127, stdout: String::new(), stderr: String::new() });
        assert!(matches!(
            remove_our_entries_on(&mut priv_with(&mut m), &Logger::memory(None)).unwrap_err(),
            RecoveryError::Unverified { .. }
        ));
    }

    fn write_image(dir: &Path, name: &str, fill: u8, sidecar: bool) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        let data = vec![fill; 16 << 20];
        std::fs::write(&p, &data).unwrap();
        if sidecar {
            std::fs::write(p.with_extension("img.sha256"), format!("{}  {name}\n", crate::debian::sha256_hex(&data))).unwrap();
        }
        p
    }

    #[test]
    fn backup_files_are_checked_before_anything_is_sent() {
        let dir = std::env::temp_dir().join(format!("sf-rec-bk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = write_image(&dir, "synofurb-dom-backup-U1-20260101-000000.img", 7, true);
        let (sha, bytes, side) = check_backup_file(&good).unwrap();
        assert!(side && bytes == 16 << 20 && sha.len() == 64);
        let nosum = write_image(&dir, "synofurb-dom-backup-U1-20260102-000000.img", 8, false);
        assert!(!check_backup_file(&nosum).unwrap().2, "no sidecar: allowed, but not claimed as checked");
        // a damaged file
        let mut bad = std::fs::read(&good).unwrap();
        bad[100] ^= 1;
        std::fs::write(&good, bad).unwrap();
        assert!(
            matches!(check_backup_file(&good).unwrap_err(), RecoveryError::BackupUnusable { reason, .. } if reason.contains("checksum"))
        );
        // too small, not a file, missing
        let tiny = dir.join("tiny.img");
        std::fs::write(&tiny, b"x").unwrap();
        assert!(matches!(check_backup_file(&tiny).unwrap_err(), RecoveryError::BackupUnusable { .. }));
        assert!(check_backup_file(&dir).is_err() && check_backup_file(&dir.join("absent.img")).is_err());
        // listing: newest first, unit-specific
        write_image(&dir, "synofurb-dom-backup-OTHER-20260103-000000.img", 9, false);
        let l = find_local_backups(&dir, "U1");
        assert_eq!(l.len(), 2);
        assert!(l[0].path.ends_with("synofurb-dom-backup-U1-20260102-000000.img") && l[1].recorded_sha256.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_uploads_compares_hashes_runs_the_tool_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("sf-rec-rs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let img = write_image(&dir, "synofurb-dom-backup-U1-20260101-000000.img", 5, true);
        let checks = check_backup_file(&img).unwrap();
        let sha = checks.0.clone();
        let mut m = MockShell::default();
        m.push_ok(""); // mkdir
        m.push_ok(&format!("{sha}  /tmp/x/flash.img\n")); // sha256sum
        m.push_ok("restored and verified\n"); // restore
        m.push_ok(""); // rm
        m.push_ok("dsm\n");
        m.push_ok(&clean_json());
        let o = restore_flash_on(&mut priv_with(&mut m), &img, checks.clone(), &Logger::memory(None)).unwrap();
        assert!(o.checked_against_sidecar && o.readback_verified && o.state_after.is_pristine() == Some(true));
        assert_eq!(m.uploads.len(), 1);
        assert_eq!(m.uploads[0].2, 0o600);
        assert!(m.uploads[0].0.starts_with("/tmp/.synofurb-restore-") && m.uploads[0].1.len() == 16 << 20);
        assert!(m.commands.iter().any(|c| c.contains("restore --yes '/tmp/.synofurb-restore-")));
        assert!(m.commands.iter().any(|c| c.starts_with("rm -rf '/tmp/.synofurb-restore-")), "the uploaded image is removed");
        // the unit received something else: the tool is never run
        let mut m = MockShell::default();
        m.push_ok("");
        m.push_ok(&format!("{}  f\n", "0".repeat(64)));
        let e = restore_flash_on(&mut priv_with(&mut m), &img, checks.clone(), &Logger::memory(None)).unwrap_err();
        assert!(matches!(e, RecoveryError::UploadMismatch { .. }));
        assert!(!m.commands.iter().any(|c| c.contains("restore --yes")));
        // the tool refuses
        let mut m = MockShell::default();
        m.push_ok("");
        m.push_ok(&format!("{sha}  f\n"));
        m.results.push_back(ExecResult { code: 1, stdout: "image does not look like a DOM\n".into(), stderr: String::new() });
        let e = restore_flash_on(&mut priv_with(&mut m), &img, checks, &Logger::memory(None)).unwrap_err();
        assert!(matches!(&e, RecoveryError::ToolRefused { output, .. } if output.contains("does not look like")), "{e}");
        assert!(m.commands.iter().any(|c| c.starts_with("rm -rf ")), "cleaned up after a refusal too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_in_the_recovery_texts_promises_that_dsm_is_the_default_or_boots_normally() {
        for t in [
            RecoveryError::ToolMissing.to_string(),
            RecoveryError::NotConfirmed { action: "x" }.to_string(),
            crate::session::agent_timeout_message(10, 30),
        ] {
            let l = t.to_lowercase();
            for bad in ["boot dsm normally", "defaults to dsm", "dsm is the default", "dsm stays the default", "dsm as before"] {
                assert!(!l.contains(bad), "{bad}: {t}");
            }
        }
    }

    // ------------------------------------------------------------------------------------------ the probe

    fn serve_banner(banner: &'static [u8]) -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let _ = s.write_all(banner);
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        port
    }

    fn serve_open() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::sleep(Duration::from_millis(200));
                drop(s);
            }
        });
        port
    }

    fn closed_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn opts(ssh: u16, web: &[u16]) -> ProbeOptions {
        ProbeOptions {
            ssh_port: ssh,
            web_ports: web.to_vec(),
            connect_timeout: Duration::from_millis(500),
            banner_timeout: Duration::from_millis(800),
        }
    }

    #[test]
    fn the_probe_tells_dsm_loader_agent_other_and_nothing_apart() {
        let ssh_openssh = serve_banner(b"SSH-2.0-OpenSSH_9.6\r\n");
        let ssh_agent = serve_banner(b"SSH-2.0-dropbear_2022.83\r\n");
        let web = serve_open();
        let none = closed_port();
        let r = probe_unit("127.0.0.1", &opts(ssh_openssh, &[none, web]));
        assert_eq!(r, ProbeResult::DsmBack { ssh_banner: "SSH-2.0-OpenSSH_9.6".into(), web_port: web });
        assert!(r.is_dsm_back());
        assert_eq!(probe_unit("127.0.0.1", &opts(none, &[web])), ProbeResult::LoaderOnly { web_port: web });
        assert!(
            matches!(probe_unit("127.0.0.1", &opts(ssh_agent, &[web])), ProbeResult::AgentStillUp { .. }),
            "dropbear is the agent, even with a web port"
        );
        assert!(matches!(probe_unit("127.0.0.1", &opts(ssh_openssh, &[none])), ProbeResult::SshOnly { .. }));
        assert_eq!(probe_unit("127.0.0.1", &opts(none, &[none])), ProbeResult::Unreachable);
        // a port that accepts but sends no SSH banner is not SSH
        assert_eq!(probe_unit("127.0.0.1", &opts(serve_open(), &[none])), ProbeResult::Unreachable);
    }

    #[test]
    fn waiting_for_dsm_reports_states_and_stops_on_success_timeout_or_cancel() {
        let ssh = serve_banner(b"SSH-2.0-OpenSSH_9.6\r\n");
        let web = serve_open();
        let none = closed_port();
        let mut seen = vec![];
        let w = wait_for_dsm(
            "127.0.0.1",
            Duration::from_secs(5),
            &AtomicBool::new(false),
            &opts(ssh, &[web]),
            Duration::from_millis(50),
            &mut |r, _| seen.push(r.clone()),
        );
        assert!(w.last.is_dsm_back() && !w.timed_out && !w.cancelled && seen.len() == 1);
        let mut n = 0;
        let w = wait_for_dsm(
            "127.0.0.1",
            Duration::from_millis(300),
            &AtomicBool::new(false),
            &opts(none, &[none]),
            Duration::from_millis(50),
            &mut |_, _| n += 1,
        );
        assert!(w.timed_out && w.last == ProbeResult::Unreachable && n >= 2);
        let c = AtomicBool::new(true);
        let w = wait_for_dsm("127.0.0.1", Duration::from_secs(60), &c, &opts(none, &[none]), Duration::from_millis(50), &mut |_, _| {});
        assert!(w.cancelled && !w.timed_out);
    }
}
