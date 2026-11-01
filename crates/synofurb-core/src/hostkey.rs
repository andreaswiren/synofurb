//! SSH host-key verification: what the client does with the key the server presents, before any password is sent.
//!
//! The SSH handshake (key exchange, server signature) completes before the client authenticates, and `russh` asks
//! [`HostKeyPolicy::check`] (through the client handler) at exactly that point. If the policy refuses, the connection is closed
//! and the authentication step is never reached: no password, no key signature and no command leaves this computer.
//!
//! Policies:
//! * [`HostKeyPolicy::Pinned`]: the presented SHA-256 fingerprint must equal the one the user gave (`--host-key SHA256:...`).
//! * [`HostKeyPolicy::Tofu`]: trust on first use against a [`KnownHosts`] file in the app data directory. A known unit must
//!   present the pinned key (a changed key is a hard error that names both fingerprints). An unknown unit is shown to the
//!   `confirm` callback together with its fingerprint (waited for at most `answer_timeout`, 180 s by default; a late answer pins nothing); `Some(callback)` returning `true` pins it, `false` aborts; with no callback
//!   the key is pinned and logged (the `ssh -o StrictHostKeyChecking=accept-new` behaviour).
//! * [`HostKeyPolicy::KeyAuthOnly`]: no host-key check, used only for the boot agent, whose host key is made up at every boot and
//!   which is reached with a per-unit client key (no password is ever sent to it).
//! * [`HostKeyPolicy::Insecure`]: accepts anything. Only the CLI flag `--insecure-host-key` selects it, and it says so loudly.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the user is asked when a unit shows a host key for the first time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    /// `SHA256:...` as `ssh-keygen -l` prints it.
    pub fingerprint: String,
}

/// Returns `true` to trust (and pin) the key. Called on a worker thread while the connection waits.
pub type HostKeyConfirm = Arc<dyn Fn(&HostKeyPrompt) -> bool + Send + Sync>;

#[derive(Clone)]
pub enum HostKeyPolicy {
    /// The server key must have exactly this fingerprint.
    Pinned(String),
    /// Trust on first use, remembered in `store`; `confirm` decides for an unknown unit (none = accept and pin).
    Tofu { store: KnownHosts, confirm: Option<HostKeyConfirm>, answer_timeout: Duration },
    /// Do not check (boot agent: random host key per boot, authenticated by our client key; no password is sent).
    KeyAuthOnly,
    /// Do not check anything. Explicit CLI flag only.
    Insecure,
}

impl std::fmt::Debug for HostKeyPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostKeyPolicy::Pinned(p) => write!(f, "Pinned({p})"),
            HostKeyPolicy::Tofu { store, confirm, answer_timeout } => {
                write!(f, "Tofu({}, confirm: {}, answer within {}s)", store.path.display(), confirm.is_some(), answer_timeout.as_secs())
            }
            HostKeyPolicy::KeyAuthOnly => write!(f, "KeyAuthOnly"),
            HostKeyPolicy::Insecure => write!(f, "Insecure"),
        }
    }
}

/// Why a host key was not accepted. Carried inside the `anyhow::Error` of a failed connect: use
/// `error.downcast_ref::<HostKeyError>()`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostKeyError {
    /// A pinned key is on record for this unit and the unit now presents another one.
    #[error(
        "the SSH host key of {host}:{port} CHANGED. Pinned: {pinned}. Presented now: {presented}. The connection was closed before anything was sent (no password, no command). If you did not just reinstall or replace this unit, someone may be intercepting the connection: do not continue. If you did, check the new fingerprint on the unit and forget the old one (`synofurb-cli host-key forget --host {host}`{port_arg})"
    )]
    Changed { host: String, port: u16, pinned: String, presented: String, port_arg: String },
    /// `--host-key` / `Pinned` was given and the unit presents a different key.
    #[error(
        "the SSH host key of {host}:{port} does not match the fingerprint you gave. Expected: {expected}. Presented: {presented}. The connection was closed before anything was sent"
    )]
    PinMismatch { host: String, port: u16, expected: String, presented: String },
    /// Nobody answered the "trust this key?" question in time. The connection was closed and nothing was pinned; an answer that comes
    /// later has no effect.
    #[error(
        "nobody answered the question about the SSH host key {fingerprint} of {host}:{port} within {seconds} seconds, so the connection was closed before any password was sent and nothing was pinned. Compare the fingerprint with the unit's, then connect again"
    )]
    AnswerTimeout { host: String, port: u16, fingerprint: String, seconds: u64 },
    /// A first-time key was shown and not trusted.
    #[error(
        "the SSH host key {fingerprint} of {host}:{port} was not trusted, so the connection was closed before any password was sent. Compare it with what the unit shows (`ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub` on the unit) and use --host-key {fingerprint} to trust it"
    )]
    NotTrusted { host: String, port: u16, fingerprint: String },
    /// The known-hosts file cannot be read or written.
    #[error("the known-hosts file {path} cannot be used: {reason}")]
    Store { path: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyDecision {
    /// Known good (matches the pin) or no check wanted.
    Accept,
    /// First contact, user (or accept-new) trusted it: it has been written to the store.
    AcceptedAndPinned,
}

/// Normalise a user-supplied fingerprint: `SHA256:abc...` or bare `abc...` (OpenSSH prints no `=` padding; it is dropped).
pub fn parse_fingerprint(s: &str) -> Result<String> {
    let t = s.trim();
    let body = t.strip_prefix("SHA256:").or_else(|| t.strip_prefix("sha256:")).unwrap_or(t).trim_end_matches('=');
    if body.len() != 43 || !body.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/') {
        bail!("'{s}' is not a SHA-256 host key fingerprint (expected SHA256: followed by 43 characters, as `ssh-keygen -l` prints it)");
    }
    Ok(format!("SHA256:{body}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownHost {
    pub fingerprint: String,
    /// RFC 3339.
    pub first_seen: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    hosts: BTreeMap<String, KnownHost>,
}

/// `host:port` -> pinned fingerprint, in a small JSON file (`known_hosts.json` in the app data directory by default).
/// Clones share the lock, so concurrent connections in one process do not lose entries.
#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

fn key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.trim().to_ascii_lowercase())
}

impl KnownHosts {
    pub fn at(path: impl Into<PathBuf>) -> KnownHosts {
        KnownHosts { path: path.into(), lock: Arc::new(Mutex::new(())) }
    }

    /// `<app data dir>/known_hosts.json`.
    pub fn default_path() -> PathBuf {
        crate::session::data_dir().join("known_hosts.json")
    }

    pub fn app_default() -> KnownHosts {
        KnownHosts::at(Self::default_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<KnownFile, HostKeyError> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) if t.trim().is_empty() => Ok(KnownFile::default()),
            Ok(t) => serde_json::from_str(&t).map_err(|e| self.err(format!("not valid JSON ({e}); move it away to start again"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(KnownFile::default()),
            Err(e) => Err(self.err(e.to_string())),
        }
    }

    fn err(&self, reason: String) -> HostKeyError {
        HostKeyError::Store { path: self.path.display().to_string(), reason }
    }

    fn save(&self, f: &KnownFile) -> Result<(), HostKeyError> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d).map_err(|e| self.err(e.to_string()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&KnownFile { version: 1, hosts: f.hosts.clone() }).map_err(|e| self.err(e.to_string()))?;
        std::fs::write(&tmp, text).map_err(|e| self.err(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| self.err(e.to_string()))
    }

    pub fn get(&self, host: &str, port: u16) -> Result<Option<KnownHost>, HostKeyError> {
        let _g = self.lock.lock().unwrap();
        Ok(self.load()?.hosts.get(&key(host, port)).cloned())
    }

    /// Record (or replace) the pinned key of a unit.
    pub fn pin(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), HostKeyError> {
        let _g = self.lock.lock().unwrap();
        let mut f = self.load()?;
        f.hosts.insert(
            key(host, port),
            KnownHost {
                fingerprint: fingerprint.to_string(),
                first_seen: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            },
        );
        self.save(&f)
    }

    /// Remove a pin. `true` when there was one.
    pub fn forget(&self, host: &str, port: u16) -> Result<bool, HostKeyError> {
        let _g = self.lock.lock().unwrap();
        let mut f = self.load()?;
        let had = f.hosts.remove(&key(host, port)).is_some();
        if had {
            self.save(&f)?;
        }
        Ok(had)
    }

    pub fn list(&self) -> Result<Vec<(String, KnownHost)>, HostKeyError> {
        let _g = self.lock.lock().unwrap();
        Ok(self.load()?.hosts.into_iter().collect())
    }
}

impl HostKeyPolicy {
    /// The default for code that does not choose: trust on first use in the app data directory, no question asked (the key is pinned
    /// and logged), a changed key is an error. Interactive front ends pass a policy with a `confirm` callback instead.
    pub fn app_default() -> HostKeyPolicy {
        HostKeyPolicy::accept_new(KnownHosts::app_default())
    }

    /// Ask-first variant of [`HostKeyPolicy::app_default`].
    pub fn tofu_with(store: KnownHosts, confirm: HostKeyConfirm) -> HostKeyPolicy {
        HostKeyPolicy::Tofu { store, confirm: Some(confirm), answer_timeout: ANSWER_TIMEOUT }
    }

    /// Trust on first use without a question: a first key is accepted and pinned.
    pub fn accept_new(store: KnownHosts) -> HostKeyPolicy {
        HostKeyPolicy::Tofu { store, confirm: None, answer_timeout: ANSWER_TIMEOUT }
    }

    /// [`HostKeyPolicy::tofu_with`] with another answer time (tests use a short one).
    pub fn tofu_with_timeout(store: KnownHosts, confirm: HostKeyConfirm, answer_timeout: Duration) -> HostKeyPolicy {
        HostKeyPolicy::Tofu { store, confirm: Some(confirm), answer_timeout }
    }

    /// Decide about a key presented by `host:port` **without writing anything**. [`HostKeyDecision::AcceptedAndPinned`] means "accepted; the
    /// caller must [`HostKeyPolicy::pin`] it once the connection is known to be alive" (the connection code pins after the handshake, so a
    /// connection that died while the person was thinking pins nothing).
    ///
    /// The confirm callback runs on its own thread and is waited for at most `answer_timeout`: no answer is [`HostKeyError::AnswerTimeout`],
    /// and an answer that arrives later is thrown away (it cannot pin, because only the caller of this function pins).
    pub fn decide(&self, host: &str, port: u16, presented: &str) -> Result<HostKeyDecision, HostKeyError> {
        match self {
            HostKeyPolicy::Insecure | HostKeyPolicy::KeyAuthOnly => Ok(HostKeyDecision::Accept),
            HostKeyPolicy::Pinned(want) => {
                if want == presented {
                    Ok(HostKeyDecision::Accept)
                } else {
                    Err(HostKeyError::PinMismatch {
                        host: host.to_string(),
                        port,
                        expected: want.clone(),
                        presented: presented.to_string(),
                    })
                }
            }
            HostKeyPolicy::Tofu { store, confirm, answer_timeout } => match store.get(host, port)? {
                Some(k) if k.fingerprint == presented => Ok(HostKeyDecision::Accept),
                Some(k) => Err(HostKeyError::Changed {
                    host: host.to_string(),
                    port,
                    pinned: k.fingerprint,
                    presented: presented.to_string(),
                    port_arg: if port == 22 { String::new() } else { format!(" --port {port}") },
                }),
                None => {
                    let trusted = match confirm {
                        None => true,
                        Some(c) => {
                            let (tx, rx) = std::sync::mpsc::channel();
                            let (c, prompt) =
                                (c.clone(), HostKeyPrompt { host: host.to_string(), port, fingerprint: presented.to_string() });
                            std::thread::spawn(move || {
                                let _ = tx.send(c(&prompt));
                            });
                            match rx.recv_timeout(*answer_timeout) {
                                Ok(t) => t,
                                Err(_) => {
                                    return Err(HostKeyError::AnswerTimeout {
                                        host: host.to_string(),
                                        port,
                                        fingerprint: presented.to_string(),
                                        seconds: answer_timeout.as_secs(),
                                    });
                                }
                            }
                        }
                    };
                    if !trusted {
                        return Err(HostKeyError::NotTrusted { host: host.to_string(), port, fingerprint: presented.to_string() });
                    }
                    Ok(HostKeyDecision::AcceptedAndPinned)
                }
            },
        }
    }

    /// Remember an accepted first-time key (a no-op for policies that keep no store).
    pub fn pin(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), HostKeyError> {
        match self {
            HostKeyPolicy::Tofu { store, .. } => store.pin(host, port, fingerprint),
            _ => Ok(()),
        }
    }

    /// [`HostKeyPolicy::decide`] and, for a newly accepted key, [`HostKeyPolicy::pin`] at once. For callers that have no connection to wait
    /// for; [`crate::remote::SshSession`] uses `decide` and pins after the handshake.
    pub fn check(&self, host: &str, port: u16, presented: &str) -> Result<HostKeyDecision, HostKeyError> {
        let d = self.decide(host, port, presented)?;
        if d == HostKeyDecision::AcceptedAndPinned {
            self.pin(host, port, presented)?;
        }
        Ok(d)
    }
}

/// [`crate::timing::HOST_KEY_ANSWER_TIMEOUT_SECS`] as a `Duration`.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(crate::timing::HOST_KEY_ANSWER_TIMEOUT_SECS);

static DEFAULT_POLICY: Mutex<Option<HostKeyPolicy>> = Mutex::new(None);

/// Policy used by [`crate::remote::SshSession::connect`] (the calls that do not pass one). A front end sets it once at start-up.
pub fn set_default_policy(p: HostKeyPolicy) {
    *DEFAULT_POLICY.lock().unwrap() = Some(p);
}

pub fn default_policy() -> HostKeyPolicy {
    DEFAULT_POLICY.lock().unwrap().clone().unwrap_or_else(HostKeyPolicy::app_default)
}

/// A finished install replaced the operating system, so the unit has a new host key. Replace the pin of `host:port` with the key the
/// app saw when it verified the new system (only when the default policy is trust-on-first-use; a `--host-key` pin or `--insecure-host-key`
/// is left alone). Returns whether a pin was written.
pub fn repin_after_install(host: &str, port: u16, fingerprint: &str) -> bool {
    match default_policy() {
        HostKeyPolicy::Tofu { store, .. } => store.pin(host, port, fingerprint).is_ok(),
        _ => false,
    }
}

/// Load a pin from the CLI's `--host-key` value: a fingerprint, or `@FILE` whose first non-comment line is one (`ssh-keygen -l` output
/// is accepted too: the `SHA256:...` word is used).
pub fn pin_from_arg(arg: &str) -> Result<String> {
    if let Some(file) = arg.strip_prefix('@') {
        let text = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
        if let Some(l) = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')) {
            let word = l.split_whitespace().find(|w| w.starts_with("SHA256:")).unwrap_or(l);
            return parse_fingerprint(word);
        }
        bail!("{file} holds no fingerprint");
    }
    parse_fingerprint(arg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP1: &str = "SHA256:abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
    const FP2: &str = "SHA256:ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ";

    fn store(tag: &str) -> KnownHosts {
        let p = std::env::temp_dir().join(format!("sf-kh-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        KnownHosts::at(p)
    }

    #[test]
    fn fingerprints_are_normalised_and_validated() {
        assert_eq!(parse_fingerprint(FP1).unwrap(), FP1);
        assert_eq!(parse_fingerprint(&FP1["SHA256:".len()..]).unwrap(), FP1);
        assert_eq!(parse_fingerprint(&format!(" {FP1}= ")).unwrap(), FP1);
        for bad in ["", "SHA256:short", "MD5:aa:bb", "SHA256:abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOP;&"] {
            assert!(parse_fingerprint(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pinned_policy_accepts_only_the_given_key() {
        let p = HostKeyPolicy::Pinned(FP1.into());
        assert_eq!(p.check("h", 22, FP1).unwrap(), HostKeyDecision::Accept);
        let e = p.check("h", 22, FP2).unwrap_err();
        assert!(matches!(e, HostKeyError::PinMismatch { .. }));
        let t = e.to_string();
        assert!(t.contains(FP1) && t.contains(FP2) && t.contains("before anything was sent"), "{t}");
    }

    #[test]
    fn tofu_pins_on_first_use_and_refuses_a_changed_key_with_both_fingerprints() {
        let s = store("tofu");
        let p = HostKeyPolicy::accept_new(s.clone());
        assert_eq!(p.check("192.168.1.20", 22, FP1).unwrap(), HostKeyDecision::AcceptedAndPinned);
        assert_eq!(s.get("192.168.1.20", 22).unwrap().unwrap().fingerprint, FP1);
        assert_eq!(p.check("192.168.1.20", 22, FP1).unwrap(), HostKeyDecision::Accept);
        // other port / other host are separate units
        assert_eq!(p.check("192.168.1.20", 2222, FP2).unwrap(), HostKeyDecision::AcceptedAndPinned);
        let e = p.check("192.168.1.20", 22, FP2).unwrap_err();
        let t = e.to_string();
        assert!(matches!(e, HostKeyError::Changed { .. }) && t.contains("CHANGED") && t.contains(FP1) && t.contains(FP2), "{t}");
        assert!(t.contains("host-key forget --host 192.168.1.20"), "{t}");
        assert_eq!(s.get("192.168.1.20", 22).unwrap().unwrap().fingerprint, FP1, "a changed key never overwrites the pin");
        // forgetting is the explicit way out
        assert!(s.forget("192.168.1.20", 22).unwrap() && !s.forget("192.168.1.20", 22).unwrap());
        assert_eq!(p.check("192.168.1.20", 22, FP2).unwrap(), HostKeyDecision::AcceptedAndPinned);
        let _ = std::fs::remove_file(s.path());
    }

    #[test]
    fn the_confirm_callback_decides_for_unknown_units_only() {
        let s = store("confirm");
        let asked = Arc::new(Mutex::new(vec![]));
        let a2 = asked.clone();
        let yes = HostKeyPolicy::tofu_with(
            s.clone(),
            Arc::new(move |p: &HostKeyPrompt| {
                a2.lock().unwrap().push(p.clone());
                true
            }),
        );
        assert_eq!(yes.check("nas", 22, FP1).unwrap(), HostKeyDecision::AcceptedAndPinned);
        assert_eq!(yes.check("NAS", 22, FP1).unwrap(), HostKeyDecision::Accept, "host names compare case-insensitively");
        assert_eq!(asked.lock().unwrap().len(), 1, "a known unit is not asked about again");
        assert_eq!(asked.lock().unwrap()[0], HostKeyPrompt { host: "nas".into(), port: 22, fingerprint: FP1.into() });
        // a refusal pins nothing
        let no = HostKeyPolicy::tofu_with(s.clone(), Arc::new(|_| false));
        let e = no.check("other", 22, FP2).unwrap_err();
        assert!(matches!(e, HostKeyError::NotTrusted { .. }) && e.to_string().contains(FP2));
        assert!(s.get("other", 22).unwrap().is_none());
        let _ = std::fs::remove_file(s.path());
    }

    #[test]
    fn a_finished_install_replaces_the_pin_but_only_under_trust_on_first_use() {
        let s = store("repin");
        set_default_policy(HostKeyPolicy::accept_new(s.clone()));
        s.pin("10.0.0.9", 22, FP1).unwrap();
        assert!(repin_after_install("10.0.0.9", 22, FP2));
        assert_eq!(s.get("10.0.0.9", 22).unwrap().unwrap().fingerprint, FP2);
        set_default_policy(HostKeyPolicy::Pinned(FP1.into()));
        assert!(!repin_after_install("10.0.0.9", 22, FP1), "an explicit --host-key is never overwritten");
        set_default_policy(HostKeyPolicy::app_default());
        let _ = std::fs::remove_file(s.path());
    }

    #[test]
    fn a_damaged_store_is_an_error_not_a_free_pass() {
        let s = store("bad");
        std::fs::write(s.path(), "{ not json").unwrap();
        let p = HostKeyPolicy::accept_new(s.clone());
        assert!(matches!(p.check("h", 22, FP1).unwrap_err(), HostKeyError::Store { .. }));
        let _ = std::fs::remove_file(s.path());
    }

    #[test]
    fn insecure_and_agent_policies_accept_anything_and_the_pin_argument_can_come_from_a_file() {
        assert_eq!(HostKeyPolicy::Insecure.check("h", 22, FP1).unwrap(), HostKeyDecision::Accept);
        assert_eq!(HostKeyPolicy::KeyAuthOnly.check("h", 22, FP2).unwrap(), HostKeyDecision::Accept);
        let f = std::env::temp_dir().join(format!("sf-pin-{}.txt", std::process::id()));
        std::fs::write(&f, format!("# from the unit\n256 {FP1} root@nas (ED25519)\n")).unwrap();
        assert_eq!(pin_from_arg(&format!("@{}", f.display())).unwrap(), FP1);
        assert_eq!(pin_from_arg(FP2).unwrap(), FP2);
        assert!(pin_from_arg("@/nonexistent/file").is_err());
        let _ = std::fs::remove_file(f);
    }
}
