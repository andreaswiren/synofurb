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
use synofurb_core::remote::SshSession;
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
    Recovered(Result<String, String>),
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
        send(&tx, Msg::Inspected(r.map(|(f, raw)| (Box::new(f), raw)).map_err(|e| format!("{e:#}"))));
    });
}

/// Read the unit's boot flash state (`recovery::read_unit_boot_state`): the real menu, never this session's memory.
pub fn spawn_unit_state(tx: Tx, t: SshTarget) {
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let r = synofurb_core::recovery::read_unit_boot_state(&t, &log).map(Box::new).map_err(|e| format!("{e:#}"));
        send(&tx, Msg::UnitState(r));
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

/// Set the unit's default boot entry to DSM (`recovery::boot_dsm`, no reboot), read it back, and report the engine's sentence.
pub fn spawn_boot_dsm(tx: Tx, t: SshTarget) {
    std::thread::spawn(move || {
        let log = Logger::memory(None);
        let r = synofurb_core::recovery::boot_dsm(&t, false, false, &log).map(|o| o.message).map_err(|e| e.to_string());
        send(&tx, Msg::Recovered(r));
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
            let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log).map_err(|e| format!("{e:#}"))?;
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
            let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log).map_err(|e| format!("{e:#}"))?;
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
