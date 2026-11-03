//! Background work. Everything slow runs on a plain `std::thread` and reports back through one unbounded channel;
//! the UI side (`App::run_receiver`) awaits that channel on GPUI's foreground executor and calls `notify`. The UI
//! thread never blocks. The core stays the only place that knows how to scan, inspect, plan and install.

use futures::channel::mpsc::UnboundedSender;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::time::Duration;
use synofurb_core::compat::{CompatDb, RefreshOutcome};
use synofurb_core::discovery::{self, CancelToken, Inventory, ScanEvent, ScanOptions};
use synofurb_core::logs::{Event, Logger};
use synofurb_core::model::DeviceFacts;
use synofurb_core::remote::{RemoteShell, SshSession};
use synofurb_core::session::{self, InstallJob, Outcome, SshTarget};
use synofurb_core::updater::{self, Channel, Progress, ReleaseCache, UpdateInfo, VerifiedDownload};

pub type Tx = UnboundedSender<Msg>;

/// Everything a worker can tell the UI.
pub enum Msg {
    /// One step of a running scan (which method is on which network, progress, a unit found, the end).
    ScanEv(ScanEvent),
    /// The scan could not start: the chosen networks cannot be used (an adapter is gone, a custom network is refused).
    ScanFailed(String),
    /// The units the scan saw, with their MAC addresses filled in. Sent after the last event.
    ScanDone(Inventory),
    /// One check of an Add-by-IP probe finished (`ticket` ties it to the dialog that asked).
    AddStep {
        ticket: u64,
        idx: usize,
        result: super::addhost::CheckResult,
    },
    /// The Add-by-IP probe ended with this report.
    AddDone {
        ticket: u64,
        target: super::addhost::Target,
        report: super::addhost::ProbeReport,
    },
    /// The unit's `health --json` (read-only), or why there is none.
    ManageHealth(Result<Box<synofurb_core::manage::Health>, String>),
    /// A result of work started for one unit context (`App::unit_gen`): the UI drops it when the page now shows another unit
    /// (or, for Manage results, when the user left Manage), so one unit's answer never lands on another unit's page.
    Scoped {
        epoch: u64,
        msg: Box<Msg>,
    },
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
    UpdateChecked(Result<Box<UpdateInfo>, super::updates::CheckFailure>),
    UpdateProgress(Progress),
    UpdateDownloaded(Result<Box<VerifiedDownload>, String>),
}

/// What `push_update` / `push_rollback` report, reduced to what the UI shows.
pub struct MgmtDone {
    /// The target the update ran against: the health read afterwards uses THIS, never whatever the page holds by then.
    pub target: SshTarget,
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

/// Send `m` tagged with the unit context it was started for.
fn send_scoped(tx: &Tx, epoch: u64, m: Msg) {
    send(tx, Msg::Scoped { epoch, msg: Box::new(m) });
}

/// A connection failed because a pinned unit now shows another host key: tell the UI (it opens the danger dialog).
fn note_changed_key(tx: &Tx, e: &anyhow::Error) {
    if let Some((host, port, pinned, presented)) = crate::ui::trust::changed_key(e) {
        send(tx, Msg::HostKeyChanged { host, port, pinned, presented });
    }
}

// ------------------------------------------------------------------------------------------------ discovery

/// Run the scan on its own thread: the core reports every step as a [`ScanEvent`], this forwards them to the UI.
/// `cancel` stops it (the listening methods within 300 ms, the port sweep within about a second); the core still ends the scan with
/// `Finished`, so the UI always gets an end.
pub fn spawn_scan(tx: Tx, opts: ScanOptions, cancel: CancelToken) {
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        match discovery::scan_with_callback(&opts, &cancel, |e| send(&tx2, Msg::ScanEv(e))) {
            Ok(inv) => send(&tx, Msg::ScanDone(inv)),
            Err(e) => send(&tx, Msg::ScanFailed(e.to_string())),
        }
    });
}

/// Check an address the person added: every check reports as it ends, then the whole report.
pub fn spawn_add_probe(tx: Tx, ticket: u64, target: super::addhost::Target, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let report = super::addhost::probe(&target, &cancel, &mut |(idx, result)| send(&tx2, Msg::AddStep { ticket, idx, result }));
        send(&tx, Msg::AddDone { ticket, target, report });
    });
}

/// `YYYY-MM-DD HH:MM:SS` in UTC without pulling in a date crate (the UI only shows it as "last seen").
pub fn now_text() -> String {
    chrono_like_now()
}

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
pub fn spawn_wait_dsm(tx: Tx, epoch: u64, host: String, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let w = synofurb_core::recovery::wait_for_dsm(
            &host,
            Duration::from_secs(synofurb_core::timing::DSM_WAIT_MIN as u64 * 60),
            &cancel,
            &synofurb_core::recovery::ProbeOptions::default(),
            Duration::from_secs(5),
            &mut |r, d| send_scoped(&tx2, epoch, Msg::DsmProbe { last: r.clone(), secs: d.as_secs() }),
        );
        send_scoped(&tx, epoch, Msg::DsmWaitDone(Box::new(w)));
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
                    "The boot flash may have changed: read the unit again before you do anything else, and use Manual recovery if it does not start."
                } else {
                    "Nothing was changed on the boot flash."
                }
            ),
            commands: vec![],
            state: None,
        },
        E::OtherUnit { .. } => RecOutcome::err("Refused: this image is another unit's", e.to_string()),
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

/// Who the connected unit is, read from the live session (`None` when the inspection cannot run there: the boot agent, a unit that
/// refuses the script). Never from the page's memory of an earlier inspect.
pub fn live_identity(
    sh: &mut dyn synofurb_core::remote::RemoteShell,
    is_root: bool,
    sudo: Option<String>,
    host: &str,
    log: &Logger,
) -> Option<synofurb_core::domboot::UnitIdentity> {
    session::inspect_remote(sh, is_root, sudo, log).ok().map(|(f, _)| synofurb_core::domboot::UnitIdentity::from_facts(&f, host))
}

/// Run one recovery action on a worker thread (`unit` names the agent key: the serial number, else the address).
pub fn spawn_recovery(tx: Tx, epoch: u64, t: SshTarget, unit: String, action: RecAction, cancel: Arc<AtomicBool>) {
    use synofurb_core::recovery as rec;
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let out = (|| -> RecOutcome {
            if let RecAction::Agent = action {
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
            // Restore writes the flash of the unit this session is connected to: read WHO that is from the live session (after the
            // connection and the host-key check, before anything is uploaded), never from what the page remembers of an earlier inspect.
            let live = matches!(action, RecAction::Restore { .. }).then(|| live_identity(&mut ssh, is_root, sudo.clone(), &t.host, &log));
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
                RecAction::Restore { backup, unit: dialog_id } => {
                    // 1. the connected unit must be the one the dialog, the phrase and the image were chosen for
                    let id = match crate::ui::facts::resolve_live_identity(dialog_id, live.as_ref().and_then(|l| l.as_ref())) {
                        Ok(id) => id,
                        Err(why) => {
                            ssh.close();
                            return RecOutcome::err("Refused: this is not the unit you chose", why);
                        }
                    };
                    // 2. the engine checks the image belongs to that (live) unit and refuses another unit's
                    match rec::check_backup_file(backup)
                        .and_then(|checks| rec::restore_flash_for_unit_on(&mut p, backup, checks, &id, false, &log))
                    {
                        Ok(o) if o.readback_verified => RecOutcome {
                            level: RecLevel::Done,
                            title: "Boot flash restored".into(),
                            detail: format!(
                                "{} MiB written from {}; the flash was read back and its hash equals the image's{}.",
                                o.bytes / 1_048_576,
                                o.backup.display(),
                                if o.checked_against_sidecar { "; the file matched its checksum file" } else { "" }
                            ),
                            commands: vec![],
                            state: Some(o.state_after),
                        },
                        // the tool ran but the flash could not be read back: not a success
                        Ok(o) => RecOutcome {
                            level: RecLevel::Warn,
                            title: "Restore ran but was not verified".into(),
                            detail: format!(
                                "{} MiB were written from {}, but the flash could not be read back to compare ({}). Check the unit with Recovery before you rely on it.",
                                o.bytes / 1_048_576,
                                o.backup.display(),
                                match &o.readback {
                                    rec::Readback::NotPossible(why) => why.clone(),
                                    _ => "reason unknown".into(),
                                }
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
        send_scoped(&tx, epoch, Msg::RecoveryDone(Box::new(out)));
    });
}

/// Unit state read: the agent is reached with its key, the rest with the page's sign-in.
pub fn spawn_unit_state_for(tx: Tx, epoch: u64, t: SshTarget, unit: String) {
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
        send_scoped(&tx, epoch, Msg::UnitState(r.map(Box::new).map_err(|e| format!("{e:#}"))));
    });
}

pub fn spawn_boot_manager(tx: Tx, epoch: u64, t: SshTarget, hostname: String, arm: bool, backup_dir: PathBuf, log_dir: PathBuf) {
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
        send_scoped(&tx, epoch, Msg::BootManagerDone(r));
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
        let r = updater::check_with_cache(current, updater::DEFAULT_REPO, channel, &cache)
            .map(Box::new)
            .map_err(|e| super::updates::classify(&e, github_reachable));
        send(&tx, Msg::UpdateChecked(r));
    });
}

/// A demo run's "Check for updates": the same message as a real check, after a short pause so the state is visible.
pub fn spawn_demo_update_check(tx: Tx, demo: super::state::DemoUpdate) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(900));
        send(&tx, Msg::UpdateChecked(super::demo::update_check_result(demo)));
    });
}

/// Can a TCP connection to GitHub's API host be opened? The typed answer to "is this a transport failure?" for the
/// engine's ambiguous `Http` error: a failure to connect is "offline", anything else means GitHub was reached.
pub fn github_reachable() -> bool {
    use std::net::ToSocketAddrs;
    let Ok(addrs) = ("api.github.com", 443).to_socket_addrs() else { return false };
    addrs.into_iter().any(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_secs(4)).is_ok())
}

/// Download an update. `unsigned` is true only after the person ticked "I understand and want to install it" (the
/// engine needs that proof as an `UnsignedConsent` and refuses it itself once a real signing key is embedded).
pub fn spawn_update_download(tx: Tx, info: UpdateInfo, dest: PathBuf, unsigned: bool) {
    std::thread::spawn(move || {
        let r = (|| -> Result<VerifiedDownload, String> {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            let tx2 = tx.clone();
            let mut progress = |p| send(&tx2, Msg::UpdateProgress(p));
            if unsigned {
                updater::download_unchecked_signature(&info, &dest, updater::UnsignedConsent::acknowledged(), &mut progress)
            } else {
                updater::ensure_can_verify(updater::UPDATE_PUBLIC_KEY).map_err(|e| e.to_string())?;
                updater::download_verified(&info, &dest, &mut progress)
            }
            .map_err(|e| e.to_string())
        })();
        send(&tx, Msg::UpdateDownloaded(r.map(Box::new)));
    });
}

/// Pace for scripted demo events.
pub const DEMO_TICK: Duration = Duration::from_millis(220);

/// Read the converted unit's health over SSH (`synofurb_core::manage::fetch_health`). Read-only.
pub fn spawn_manage_health(tx: Tx, epoch: u64, t: SshTarget) {
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
        send_scoped(&tx, epoch, Msg::ManageHealth(r));
    });
}

/// Update the converted unit's management plane (or roll it back) with the engine's `push_update` / `push_rollback`.
/// The unit goes back to the old version by itself if the new one does not answer; the report says which state it is in.
pub fn spawn_mgmt_update(tx: Tx, epoch: u64, t: SshTarget, rollback: bool) {
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
                    send_scoped(&tx2, epoch, Msg::MgmtProgress((n.saturating_sub(1) * 100 / total.max(1)).min(99) as u8));
                }
            };
            let rep = if rollback {
                push_rollback(&mut p, &opts, &mut emit)
            } else {
                push_update(&mut p, &MgmtBundle::embedded(), &opts, &mut emit)
            };
            ssh.close();
            rep.map(|r| MgmtDone { target: t.clone(), outcome: r.outcome, headline: r.headline() }).map_err(|e| format!("{e:#}"))
        })();
        send_scoped(&tx, epoch, Msg::MgmtDone(r));
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

    /// A scripted unit: answers `id -u` and the inspection script like a DSM 7 DS918+ would.
    struct ScriptedUnit {
        serial: &'static str,
    }

    impl synofurb_core::remote::RemoteShell for ScriptedUnit {
        fn exec(&mut self, cmd: &str, _stdin: Option<&[u8]>) -> anyhow::Result<synofurb_core::remote::ExecResult> {
            let stdout = if cmd.trim() == "id -u" {
                "0\n".to_string()
            } else {
                synofurb_core::inspect::fixtures::ds_4bay().replace("1920PDN123456", self.serial)
            };
            Ok(synofurb_core::remote::ExecResult { code: 0, stdout, stderr: String::new() })
        }
        fn exec_to(&mut self, _c: &str, _s: Option<&[u8]>, _o: &mut dyn std::io::Write) -> anyhow::Result<(u32, String)> {
            Ok((0, String::new()))
        }
        fn upload(&mut self, _p: &str, _d: &mut dyn std::io::Read, _n: u64, _m: u32, _pr: &mut dyn FnMut(u64)) -> anyhow::Result<()> {
            Ok(())
        }
        fn host_key_fingerprint(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn restore_reads_the_identity_of_the_connected_unit_not_of_the_page() {
        use synofurb_core::domboot::UnitIdentity;
        let log = Logger::memory(None);
        // the dialog was built for unit A (an earlier inspect); the address now answers as unit B
        let dialog = UnitIdentity { serial: "SN-A-0001".into(), model: "DS918+".into(), host: "192.168.1.20".into() };
        let mut b = ScriptedUnit { serial: "SN-B-0002" };
        let live = live_identity(&mut b, true, None, "192.168.1.20", &log).expect("the scripted unit inspects");
        assert_eq!((live.serial.as_str(), live.model.as_str()), ("SN-B-0002", "DS918+"));
        let refused = crate::ui::facts::resolve_live_identity(&dialog, Some(&live));
        assert!(refused.is_err(), "unit B must not receive unit A's image");
        // the same unit goes through, with the live identity (the one the engine checks the image against)
        let mut a = ScriptedUnit { serial: "SN-A-0001" };
        let live = live_identity(&mut a, true, None, "192.168.1.20", &log).unwrap();
        assert_eq!(crate::ui::facts::resolve_live_identity(&dialog, Some(&live)).unwrap().serial, "SN-A-0001");
    }
}
