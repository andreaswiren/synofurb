//! Background work. Everything slow runs on a plain `std::thread` and reports back through one unbounded channel;
//! the UI side (`App::run_receiver`) awaits that channel on GPUI's foreground executor and calls `notify`. The UI
//! thread never blocks. The core stays the only place that knows how to scan, inspect, plan and install.

use futures::channel::mpsc::UnboundedSender;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;
use synofurb_core::compat::{CompatDb, RefreshOutcome};
use synofurb_core::discovery::{self, Found, Inventory, ScanOptions, mdns, probe, ssdp, syno_udp};
use synofurb_core::logs::{Event, Logger};
use synofurb_core::model::DeviceFacts;
use synofurb_core::remote::{RemoteShell, SshSession};
use synofurb_core::session::{self, InstallJob, Outcome, SshTarget};
use synofurb_core::updater::{self, Channel, Progress, ReleaseCache, UpdateInfo, VerifiedDownload};

pub type Tx = UnboundedSender<Msg>;

/// Everything a worker can tell the UI.
pub enum Msg {
    /// A discovery method finished (`Ok(n)` results) or failed.
    ScanMethod {
        name: &'static str,
        result: Result<usize, String>,
    },
    /// Merged inventory so far (live results).
    ScanPartial(Inventory),
    ScanDone(Inventory),
    /// The unit's `health --json` (read-only), or why there is none.
    ManageHealth(Result<Box<synofurb_core::manage::Health>, String>),
    Inspected(Result<(Box<DeviceFacts>, String), String>),
    Ev(Event),
    /// What kind of failure the install ended with (typed, from the engine), sent just before `Finished(Err)`.
    /// First contact with a unit: its SSH host key must be trusted before anything is sent. Answer the question; a dropped one
    /// refuses.
    HostKeyAsk(Box<crate::ui::trust::TrustAsk>),
    /// A pinned unit now presents another key (the engine already refused to continue).
    HostKeyChanged {
        host: String,
        port: u16,
        pinned: String,
        presented: String,
    },
    FailureKind {
        /// `SessionError::AgentTimeout`: the agent never answered, so the unit's default boot entry is not confirmed.
        agent_timeout: bool,
        /// `SessionError::FlashBackupFailed`: nothing was written to the boot flash.
        flash_untouched: bool,
    },
    Finished(Result<Outcome, String>),
    /// The unit's real boot state, read over SSH when Recovery opens.
    UnitState(Result<Box<synofurb_core::recovery::UnitBootState>, String>),
    /// One look at the unit's DSM address while waiting for DSM, with the seconds waited.
    DsmProbe {
        last: synofurb_core::recovery::ProbeResult,
        secs: u64,
    },
    DsmWaitDone(Box<synofurb_core::recovery::DsmWait>),
    /// A recovery action finished (the engine verified it by reading the unit back, or says why not).
    RecoveryDone(Box<RecOutcome>),
    BootManagerDone(Result<(), String>),
    /// Progress (0..=100) of a management-plane update or rollback.
    MgmtProgress(u8),
    /// A management-plane update or rollback finished (the engine's own report), or could not be driven at all.
    MgmtDone(Result<MgmtDone, String>),
    CompatRefreshed(Box<CompatRefresh>),
    UpdateChecked(Result<Box<UpdateInfo>, String>),
    UpdateProgress(Progress),
    UpdateDownloaded(Result<Box<VerifiedDownload>, String>),
}

/// What `push_update` / `push_rollback` report, reduced to what the UI shows.
pub struct MgmtDone {
    pub outcome: synofurb_core::manage::UpdateOutcome,
    pub headline: String,
}

pub struct CompatRefresh {
    pub summary: String,
    pub ok: bool,
    pub db: CompatDb,
}

fn send(tx: &Tx, m: Msg) {
    let _ = tx.unbounded_send(m);
}

/// A connection failed because a pinned unit now shows another host key: tell the UI (it opens the danger dialog).
fn note_changed_key(tx: &Tx, e: &anyhow::Error) {
    if let Some((host, port, pinned, presented)) = crate::ui::trust::changed_key(e) {
        send(tx, Msg::HostKeyChanged { host, port, pinned, presented });
    }
}

// ------------------------------------------------------------------------------------------------ discovery

/// Run every discovery method on its own thread and stream merged results. (The core's `discovery::scan`
/// only returns at the end; this is the same orchestration with a partial-result hook. See docs/GUI.md.)
pub fn spawn_scan(tx: Tx, opts: ScanOptions, known: Inventory) {
    std::thread::spawn(move || {
        let (ftx, frx) = channel::<Found>();
        let t = opts.timeout;
        let mut handles = vec![];
        macro_rules! method {
            ($name:literal, $f:expr) => {{
                let (ftx, tx) = (ftx.clone(), tx.clone());
                handles.push(std::thread::spawn(move || {
                    let r = $f(&ftx).map_err(|e| format!("{e:#}"));
                    send(&tx, Msg::ScanMethod { name: $name, result: r });
                }));
            }};
        }
        method!("Synology Assistant broadcast", move |s: &Sender<Found>| syno_udp::scan(t, s));
        method!("Bonjour/mDNS", move |s: &Sender<Found>| mdns::scan(t, s));
        method!("UPnP/SSDP", move |s: &Sender<Found>| ssdp::scan(t, s));
        if opts.sweep {
            method!("Subnet sweep (DSM ports)", |s: &Sender<Found>| probe::sweep(s));
        }
        drop(ftx);
        let mut inv = known;
        let now = chrono_like_now();
        for mut f in frx {
            f.last_seen = now.clone();
            if f.mac.is_none()
                && let Some(ip) = f.primary_ip()
            {
                f.mac = discovery::arp_lookup(ip);
            }
            inv.merge(f);
            send(&tx, Msg::ScanPartial(inv.clone()));
        }
        for h in handles {
            let _ = h.join();
        }
        for d in &mut inv.devices {
            if d.mac.is_none()
                && let Some(ip) = d.primary_ip()
            {
                d.mac = discovery::arp_lookup(ip);
                if d.mac.as_deref().is_some_and(discovery::is_synology_mac) {
                    d.synology = true;
                    d.sources.push("oui".into());
                }
            }
        }
        send(&tx, Msg::ScanDone(inv));
    });
}

/// `YYYY-MM-DD HH:MM:SS` in UTC without pulling in a date crate (the UI only shows it as "last seen").
fn chrono_like_now() -> String {
    let secs = super::settings::now_unix();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil-from-days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, (rem % 3600) / 60, rem % 60)
}

// ------------------------------------------------------------------------------------------------ inspect / recover / boot manager

pub fn spawn_inspect(tx: Tx, t: SshTarget) {
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let r = (|| -> anyhow::Result<_> {
            let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log)?;
            let r = session::inspect_remote(&mut ssh, false, t.sudo_password.clone(), &log);
            ssh.close();
            r
        })();
        if let Err(e) = &r {
            note_changed_key(&tx, e);
        }
        send(&tx, Msg::Inspected(r.map(|(f, raw)| (Box::new(f), raw)).map_err(|e| format!("{e:#}"))));
    });
}

/// Probe the unit's DSM address every few seconds until DSM answers or `DSM_WAIT_MIN` minutes pass.
pub fn spawn_wait_dsm(tx: Tx, host: String, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let w = synofurb_core::recovery::wait_for_dsm(
            &host,
            Duration::from_secs(synofurb_core::timing::DSM_WAIT_MIN as u64 * 60),
            &cancel,
            &synofurb_core::recovery::ProbeOptions::default(),
            Duration::from_secs(5),
            &mut |r, d| send(&tx2, Msg::DsmProbe { last: r.clone(), secs: d.as_secs() }),
        );
        send(&tx, Msg::DsmWaitDone(Box::new(w)));
    });
}

/// What a recovery action asks for.
#[derive(Clone, Debug)]
pub enum RecAction {
    /// Set the default boot entry to DSM; `reboot` also asks the unit to reboot (the user chose it).
    BootDsm { reboot: bool },
    /// Write a saved flash image back through `synofurb-dom restore` on the unit.
    Restore { backup: PathBuf, unit: synofurb_core::domboot::UnitIdentity },
    /// Take our entries out of the boot menu.
    Remove,
    /// Arm the boot agent for one boot, reboot and wait for it.
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecLevel {
    /// Done and confirmed by reading the unit back.
    Done,
    /// A note for the user (a command to run): neither success nor failure.
    Info,
    /// Something happened but it could not be confirmed, or the unit is in a state the user must look at.
    Warn,
    Error,
}

pub struct RecOutcome {
    pub level: RecLevel,
    pub title: String,
    pub detail: String,
    /// Commands to show with a Copy button (the agent's ssh line).
    pub commands: Vec<String>,
    /// The unit's boot state read back after the action (replaces what the Recovery page knows).
    pub state: Option<synofurb_core::recovery::UnitBootState>,
}

impl RecOutcome {
    fn err(title: &str, detail: String) -> RecOutcome {
        RecOutcome { level: RecLevel::Error, title: title.into(), detail, commands: vec![], state: None }
    }
}

/// Text for a typed recovery failure. "Could not be confirmed" is a warning, never a success and never a plain error.
pub fn outcome_of_error(e: &synofurb_core::recovery::RecoveryError) -> RecOutcome {
    use synofurb_core::recovery::RecoveryError as E;
    match e {
        E::Unverified { what, flash_may_have_changed, .. } => RecOutcome {
            level: RecLevel::Warn,
            title: format!("{} not confirmed", capital(what)),
            detail: format!(
                "{e}. {}",
                if *flash_may_have_changed {
                    "The boot flash may have changed: read the unit again before you do anything else."
                } else {
                    "Nothing was changed on the boot flash."
                }
            ),
            commands: vec![],
            state: None,
        },
        other => RecOutcome::err("Not done", other.to_string()),
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Connect for a recovery action. A unit that runs the boot agent (dropbear on the SSH port) is reached with the unit's agent
/// key and no host-key check (the agent's key changes at every boot and no password is ever sent to it); anything else uses the
/// sign-in of the page and the host-key policy (pinned, trust on first use). Returns the session and whether it is the agent.
fn recovery_session(t: &SshTarget, unit: &str, log: &Logger) -> anyhow::Result<(SshSession, bool)> {
    use synofurb_core::hostkey::HostKeyPolicy;
    use synofurb_core::recovery::{ProbeOptions, ProbeResult, probe_unit};
    let opts = ProbeOptions { ssh_port: t.port, ..ProbeOptions::default() };
    if matches!(probe_unit(&t.host, &opts), ProbeResult::AgentStillUp { .. }) {
        let key = session::agent_key_for(unit, &session::config_keys_dir())?;
        let s = SshSession::connect_with(
            &t.host,
            t.port,
            "root",
            &synofurb_core::remote::Auth::Key(key.private_openssh),
            log,
            &HostKeyPolicy::KeyAuthOnly,
        )?;
        return Ok((s, true));
    }
    Ok((SshSession::connect(&t.host, t.port, &t.user, &t.auth, log)?, false))
}

/// Run one recovery action on a worker thread (`unit` names the agent key: the serial number, else the address).
pub fn spawn_recovery(tx: Tx, t: SshTarget, unit: String, action: RecAction) {
    use synofurb_core::recovery as rec;
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let out = (|| -> RecOutcome {
            if let RecAction::Agent = action {
                let cancel = AtomicBool::new(false);
                return match rec::enter_agent(&t, &cancel, true, &log) {
                    Ok(a) => RecOutcome {
                        level: RecLevel::Done,
                        title: "The agent answers".into(),
                        detail: format!(
                            "It reset the default to DSM when it started; its watchdog reboots the unit into DSM after {} minutes without a login. Open a shell with the command below.",
                            synofurb_core::timing::BOOT_MANAGER_WATCHDOG_MIN
                        ),
                        commands: vec![a.ssh_command],
                        state: None,
                    },
                    Err(e) => {
                        note_changed_key(&tx, &e);
                        if let Some(s) = session::SessionError::find(&e) {
                            send(
                                &tx,
                                Msg::FailureKind { agent_timeout: s.boot_default_unconfirmed(), flash_untouched: s.flash_untouched() },
                            );
                            return RecOutcome {
                                level: RecLevel::Warn,
                                title: "The agent did not answer".into(),
                                detail: s.to_string(),
                                commands: vec![],
                                state: None,
                            };
                        }
                        RecOutcome::err("The agent was not started", format!("{e:#}"))
                    }
                };
            }
            let (mut ssh, agent) = match recovery_session(&t, &unit, &log) {
                Ok(s) => s,
                Err(e) => {
                    note_changed_key(&tx, &e);
                    return RecOutcome::err("The unit could not be reached", format!("{e:#}"));
                }
            };
            let is_root = agent || ssh.exec("id -u", None).map(|r| r.stdout.trim() == "0").unwrap_or(false);
            let sudo = if agent { None } else { t.sudo_password.clone() };
            let mut p = synofurb_core::remote::Privileged { sh: &mut ssh, is_root, sudo_password: sudo };
            let out = match &action {
                RecAction::BootDsm { reboot } => match rec::boot_dsm_on(&mut p, *reboot, &log) {
                    Ok(o) => RecOutcome {
                        level: RecLevel::Done,
                        title: if o.reboot_requested { "Reboot requested".into() } else { "Default set to DSM".into() },
                        detail: o.message,
                        commands: vec![],
                        state: Some(o.state_after),
                    },
                    Err(e) => outcome_of_error(&e),
                },
                RecAction::Restore { backup, unit: id } => {
                    // the image must be this unit's (checked here again, not only when the dialog was built)
                    match rec::check_backup_belongs(backup, id, false)
                        .and_then(|_| rec::check_backup_file(backup))
                        .and_then(|checks| rec::restore_flash_on(&mut p, backup, checks, &log))
                    {
                        Ok(o) => RecOutcome {
                            level: RecLevel::Done,
                            title: "Boot flash restored".into(),
                            detail: format!(
                                "{} MiB written from {}, read back and compared{}.",
                                o.bytes / 1_048_576,
                                o.backup.display(),
                                if o.checked_against_sidecar { "; the file matched its checksum file" } else { "" }
                            ),
                            commands: vec![],
                            state: Some(o.state_after),
                        },
                        Err(e) => outcome_of_error(&e),
                    }
                }
                RecAction::Remove => match rec::remove_our_entries_on(&mut p, &log) {
                    Ok(o) => RecOutcome {
                        level: RecLevel::Done,
                        title: "Our entries are removed".into(),
                        detail: "The boot menu was read back: none of our entries are left and the default is DSM.".into(),
                        commands: vec![],
                        state: Some(o.state_after),
                    },
                    Err(e) => outcome_of_error(&e),
                },
                RecAction::Agent => unreachable!("handled above"),
            };
            ssh.close();
            out
        })();
        send(&tx, Msg::RecoveryDone(Box::new(out)));
    });
}

/// Unit state read: the agent is reached with its key, the rest with the page's sign-in.
pub fn spawn_unit_state_for(tx: Tx, t: SshTarget, unit: String) {
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let r = (|| -> anyhow::Result<synofurb_core::recovery::UnitBootState> {
            let (mut ssh, agent) = recovery_session(&t, &unit, &log)?;
            let is_root = agent || ssh.exec("id -u", None)?.stdout.trim() == "0";
            let sudo = if agent { None } else { t.sudo_password.clone() };
            let mut p = synofurb_core::remote::Privileged { sh: &mut ssh, is_root, sudo_password: sudo };
            let st = synofurb_core::recovery::read_boot_state(&mut p);
            ssh.close();
            Ok(st)
        })();
        if let Err(e) = &r {
            note_changed_key(&tx, e);
        }
        send(&tx, Msg::UnitState(r.map(Box::new).map_err(|e| format!("{e:#}"))));
    });
}

pub fn spawn_boot_manager(tx: Tx, t: SshTarget, hostname: String, arm: bool, backup_dir: PathBuf, log_dir: PathBuf) {
    std::thread::spawn(move || {
        let (etx, erx) = channel::<Event>();
        forward_events(erx, tx.clone());
        let r = match Logger::to_dir(&log_dir, Some(etx)) {
            Ok(logger) => {
                logger.add_secret(&auth_secret(&t));
                let lg = logger.clone();
                session::boot_manager_install(&t, None, &backup_dir, &hostname, arm, &logger, &mut |what, f| {
                    lg.emit(Event::Progress { what: what.to_string(), fraction: f })
                })
                .map_err(|e| format!("{e:#}"))
            }
            Err(e) => Err(format!("cannot create log file: {e}")),
        };
        send(&tx, Msg::BootManagerDone(r));
    });
}

fn auth_secret(t: &SshTarget) -> String {
    match &t.auth {
        synofurb_core::remote::Auth::Password(p) => p.clone(),
        synofurb_core::remote::Auth::Key(_) => String::new(),
    }
}

/// Move logger events from the core's std channel into the UI channel.
fn forward_events(rx: std::sync::mpsc::Receiver<Event>, tx: Tx) {
    std::thread::spawn(move || {
        for e in rx {
            send(&tx, Msg::Ev(e));
        }
    });
}

// ------------------------------------------------------------------------------------------------ install

/// Start the real install. Returns the logger (for its file path) or the error that prevented starting.
pub fn spawn_install(tx: Tx, job: InstallJob, log_dir: &Path, cancel: Arc<AtomicBool>) -> Result<Logger, String> {
    let (etx, erx) = channel::<Event>();
    let logger = Logger::to_dir(log_dir, Some(etx)).map_err(|e| format!("cannot create log file: {e}"))?;
    forward_events(erx, tx.clone());
    let lg = logger.clone();
    std::thread::spawn(move || {
        let progress = lg.clone();
        let r =
            session::run_install(&job, &lg, cancel, &mut |what, f| progress.emit(Event::Progress { what: what.to_string(), fraction: f }));
        if let Err(e) = &r {
            note_changed_key(&tx, e);
            let t = session::SessionError::find(e);
            send(
                &tx,
                Msg::FailureKind {
                    agent_timeout: t.is_some_and(|t| t.boot_default_unconfirmed()),
                    flash_untouched: t.is_some_and(|t| t.flash_untouched()),
                },
            );
        }
        send(&tx, Msg::Finished(r.map_err(|e| format!("{e:#}"))));
    });
    Ok(logger)
}

// ------------------------------------------------------------------------------------------------ compat list and updates

pub fn spawn_compat_refresh(tx: Tx, cache_dir: PathBuf) {
    std::thread::spawn(move || {
        let (summary, ok) = match CompatDb::refresh(&cache_dir, synofurb_core::compat::DEFAULT_URL) {
            Ok(RefreshOutcome::Updated { models, updated }) => (format!("Updated: {models} models, list dated {updated}."), true),
            Ok(RefreshOutcome::NotModified) => ("The cached list is already current.".to_string(), true),
            Ok(RefreshOutcome::Offline { reason }) => {
                (format!("Could not reach the server ({reason}). The cached or built-in list stays in use."), false)
            }
            Ok(RefreshOutcome::Rejected { reason }) => {
                (format!("The downloaded list was rejected ({reason}). Nothing was changed."), false)
            }
            Err(e) => (format!("Refresh failed: {e:#}"), false),
        };
        let db = CompatDb::load_best(&cache_dir);
        send(&tx, Msg::CompatRefreshed(Box::new(CompatRefresh { summary, ok, db })));
    });
}

pub fn spawn_update_check(tx: Tx, current: &'static str, channel: Channel, cache_dir: PathBuf) {
    std::thread::spawn(move || {
        let cache = ReleaseCache::on_disk(&cache_dir.join("releases"));
        let r = updater::check_with_cache(current, updater::DEFAULT_REPO, channel, &cache).map(Box::new).map_err(|e| e.to_string());
        send(&tx, Msg::UpdateChecked(r));
    });
}

pub fn spawn_update_download(tx: Tx, info: UpdateInfo, dest: PathBuf) {
    std::thread::spawn(move || {
        let r = (|| -> Result<VerifiedDownload, String> {
            updater::ensure_can_verify(updater::UPDATE_PUBLIC_KEY).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let tx2 = tx.clone();
            updater::download_verified(&info, &dest, &mut |p| send(&tx2, Msg::UpdateProgress(p))).map_err(|e| e.to_string())
        })();
        send(&tx, Msg::UpdateDownloaded(r.map(Box::new)));
    });
}

/// Pace for scripted demo events.
pub const DEMO_TICK: Duration = Duration::from_millis(220);

/// Read the converted unit's health over SSH (`synofurb_core::manage::fetch_health`). Read-only.
pub fn spawn_manage_health(tx: Tx, t: SshTarget) {
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let r = (|| -> Result<Box<synofurb_core::manage::Health>, String> {
            let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log).map_err(|e| {
                note_changed_key(&tx, &e);
                format!("{e:#}")
            })?;
            let is_root = t.user == "root";
            let mut p = synofurb_core::remote::Privileged { sh: &mut ssh, is_root, sudo_password: t.sudo_password.clone() };
            let h = synofurb_core::manage::fetch_health(&mut p).map_err(|e| e.to_string());
            ssh.close();
            h.map(Box::new)
        })();
        send(&tx, Msg::ManageHealth(r));
    });
}

/// Update the converted unit's management plane (or roll it back) with the engine's `push_update` / `push_rollback`.
/// The unit goes back to the old version by itself if the new one does not answer; the report says which state it is in.
pub fn spawn_mgmt_update(tx: Tx, t: SshTarget, rollback: bool) {
    use synofurb_core::manage::{ManageEvent, MgmtBundle, UpdateOptions, push_rollback, push_update};
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let r = (|| -> Result<MgmtDone, String> {
            let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log).map_err(|e| {
                note_changed_key(&tx, &e);
                format!("{e:#}")
            })?;
            let is_root = t.user == "root";
            let mut p = synofurb_core::remote::Privileged { sh: &mut ssh, is_root, sudo_password: t.sudo_password.clone() };
            let opts = UpdateOptions::default();
            let tx2 = tx.clone();
            let mut emit = move |e: ManageEvent| {
                if let ManageEvent::Step { n, total, .. } = e {
                    send(&tx2, Msg::MgmtProgress((n.saturating_sub(1) * 100 / total.max(1)).min(99) as u8));
                }
            };
            let rep = if rollback {
                push_rollback(&mut p, &opts, &mut emit)
            } else {
                push_update(&mut p, &MgmtBundle::embedded(), &opts, &mut emit)
            };
            ssh.close();
            rep.map(|r| MgmtDone { outcome: r.outcome, headline: r.headline() }).map_err(|e| format!("{e:#}"))
        })();
        send(&tx, Msg::MgmtDone(r));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_well_formed() {
        let t = chrono_like_now();
        assert_eq!(t.len(), 19, "{t}");
        assert!(t.starts_with("20"), "{t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], " ");
    }
}
