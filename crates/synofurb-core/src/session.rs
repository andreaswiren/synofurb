//! The install job: inspect -> plan -> stage -> boot installer -> follow it -> verify the result.

use crate::config::{InstallConfig, random_token};
use crate::debian::{self, InstallerFiles, PreseedParams};
use crate::discovery::mdns;
use crate::inspect::{inspect_script, parse_inspect};
use crate::kexec::{self, KexecJob, KitParams};
use crate::logs::{Event, Logger, Phase};
use crate::model::*;
use crate::payload::build_payload;
use crate::plan::Plan;
use crate::remote::{Auth, Privileged, RemoteShell, SshSession, fetch_host_fingerprint};
use crate::script::{ScriptParams, render_stage2};
use crate::server::{Served, Server, local_ip_towards};
use anyhow::{Context, Result, bail};
use std::net::{IpAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    /// Needed when `user` is not root (DSM 7 admins use sudo).
    pub sudo_password: Option<String>,
}

#[derive(Clone)]
pub enum BootMethod {
    /// Upload a static kexec + the installer and jump into it from the running DSM.
    Kexec { kexec_bin: Option<PathBuf> },
    /// Write kernel/initrd/README to a folder; the user boots the NAS from it (USB, TFTP, U-Boot).
    Kit { dir: PathBuf },
}

pub struct InstallJob {
    pub target: SshTarget,
    pub facts: DeviceFacts,
    pub plan: Plan,
    pub cfg: InstallConfig,
    pub method: BootMethod,
    pub cache_dir: PathBuf,
    pub with_firmware: bool,
    pub verbose_installer: bool,
    /// Plain-text password for the final summary only (never logged).
    pub display_password: String,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub hostname: String,
    pub ip: Option<IpAddr>,
    pub ssh_fingerprint: Option<String>,
    pub mgmt_url: Option<String>,
    pub mgmt_reachable: bool,
    pub username: String,
    pub log_path: Option<PathBuf>,
}

/// Inspect a running DSM over SSH. Returns parsed facts and the raw output (kept with the install).
pub fn inspect_remote(
    sh: &mut dyn RemoteShell,
    is_root: bool,
    sudo_password: Option<String>,
    log: &Logger,
) -> Result<(DeviceFacts, String)> {
    let id = sh.exec("id -u", None)?;
    let root = id.stdout.trim() == "0";
    let mut p = Privileged { sh, is_root: root || is_root, sudo_password };
    log.info("running the read-only inspection script on the NAS");
    let r = p.run_script(&inspect_script())?;
    if !r.ok() && r.stdout.trim().is_empty() {
        bail!("inspection failed (exit {}): {}", r.code, r.stderr.trim());
    }
    let mut facts = parse_inspect(&r.stdout)?;
    facts.is_root = root;
    log.info(format!(
        "NAS: {} {} DSM {} | {} | {} | {} MiB RAM | {} disks, {} arrays | kexec {}",
        facts.model,
        facts.arch.label(),
        facts.dsm_version,
        facts.firmware.label(),
        facts.cpu,
        facts.mem_kib / 1024,
        facts.disks.len(),
        facts.arrays.len(),
        if facts.kexec { "available" } else { "NOT available" }
    ));
    Ok((facts, r.stdout))
}

/// Pre-pick ports so the script can embed them before the server starts.
fn free_ports(ip: IpAddr) -> Result<(u16, u16, u16)> {
    let t1 = std::net::TcpListener::bind((ip, 0))?;
    let t2 = std::net::TcpListener::bind((ip, 0))?;
    let u = UdpSocket::bind((ip, 0))?;
    Ok((t1.local_addr()?.port(), t2.local_addr()?.port(), u.local_addr()?.port()))
}

pub struct Staged {
    pub server: Server,
    pub files: InstallerFiles,
    pub initrd: PathBuf,
    pub preseed: String,
    pub cmdline: String,
    pub desktop_ip: IpAddr,
    pub http_base: String,
}

/// Render script/payload/preseed, start the local server and build the installer initrd.
pub fn stage(job: &InstallJob, bind_ip: IpAddr, inspect_raw: &str, log: &Logger, progress: &mut dyn FnMut(&str, f32)) -> Result<Staged> {
    if !job.plan.is_ok() {
        bail!("the install plan has errors: {}", job.plan.errors.join("; "));
    }
    job.cfg.validate()?;
    let (http_port, tcp_port, udp_port) = free_ports(bind_ip)?;
    let token = random_token(16);
    let http_base = format!("http://{bind_ip}:{http_port}/{token}");
    let payload = build_payload(&job.cfg, &job.plan, &job.facts, inspect_raw)?;
    let run_id = random_token(4);
    let stage2 = render_stage2(&ScriptParams {
        plan: &job.plan,
        facts: &job.facts,
        cfg: &job.cfg,
        run_id: &run_id,
        http_base: &http_base,
        payload_sha256: &payload.sha256,
        log_host: &bind_ip.to_string(),
        log_tcp_port: tcp_port,
        dry_run: false,
    })?;
    log.debug(format!("stage-2 script: {} lines, payload sha256 {}", stage2.lines().count(), payload.sha256));
    if let Some(p) = log.path() {
        let dir = p.parent().unwrap().to_path_buf();
        let _ = std::fs::write(dir.join(format!("stage2-{run_id}.sh")), &stage2);
        let _ = std::fs::write(dir.join(format!("plan-{run_id}.json")), serde_json::to_string_pretty(&job.plan)?);
        log.info(format!("saved the generated installer script to {}/stage2-{run_id}.sh for review", dir.display()));
    }
    let preseed = debian::render_preseed(&PreseedParams { cfg: &job.cfg, http_base: &http_base });
    let files =
        debian::fetch_installer(job.facts.arch, &job.cfg.suite, &job.cache_dir, job.with_firmware, job.cfg.dtb.as_deref(), log, progress)?;
    let initrd = job.cache_dir.join(format!("initrd-synofurb-{run_id}.gz"));
    debian::build_initrd(&files, &preseed, &initrd)?;
    let cmdline = debian::installer_cmdline(&job.cfg.serial, &bind_ip.to_string(), udp_port, job.verbose_installer, None);
    log.info(format!("installer kernel command line: {cmdline}"));
    let server = Server::start(
        bind_ip,
        Served { token, stage2, payload: payload.bytes, preseed: preseed.clone() },
        log.clone(),
        (http_port, tcp_port, udp_port),
    )?;
    Ok(Staged { server, files, initrd, preseed, cmdline, desktop_ip: bind_ip, http_base })
}

/// Wait until the installer reports done/failure on the event channel.
fn follow_installer(rx: &std::sync::mpsc::Receiver<Event>, cancel: &AtomicBool, first_contact: Duration, log: &Logger) -> Result<()> {
    let start = Instant::now();
    let mut heard = false;
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled by the user (the installer on the NAS keeps running if it has already started)");
        }
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::Log(l)) if matches!(l.source, crate::logs::Source::Installer | crate::logs::Source::Syslog) => {
                heard = true;
                last = Instant::now();
            }
            Ok(Event::InstallerDone) => return Ok(()),
            Ok(Event::InstallerFailed(m)) => bail!("the installer on the NAS reported a failure: {m}"),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                if !heard && start.elapsed() > first_contact {
                    bail!(
                        "the NAS did not contact this computer within {} minutes. Check the serial console / VGA output of the NAS and that this computer's firewall allows the ports shown in the log",
                        first_contact.as_secs() / 60
                    );
                }
                if heard && last.elapsed() > Duration::from_secs(1800) {
                    log.warn("no installer output for 30 minutes - a long RAID/format step, or the NAS hung; check its console");
                    last = Instant::now();
                }
            }
            Err(RecvTimeoutError::Disconnected) => bail!("event channel closed"),
        }
    }
}

/// After the reboot: find the new system (DHCP may have changed its address) and collect login facts.
pub fn wait_for_new_system(job: &InstallJob, old_ip: Option<IpAddr>, cancel: &AtomicBool, log: &Logger) -> Result<Outcome> {
    log.phase(Phase::WaitingForReboot);
    let deadline = Instant::now() + Duration::from_secs(20 * 60);
    let mut candidates: Vec<IpAddr> = old_ip.into_iter().collect();
    let mut last_scan = Instant::now() - Duration::from_secs(60);
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if last_scan.elapsed() > Duration::from_secs(20) {
            last_scan = Instant::now();
            let (tx, rx) = channel();
            let _ = mdns::scan(Duration::from_secs(4), &tx);
            drop(tx);
            for f in rx {
                if f.synofurb_debian && f.hostname.eq_ignore_ascii_case(&job.cfg.hostname) {
                    for ip in f.ips {
                        if !candidates.contains(&ip) {
                            log.info(format!("found {} via mDNS at {ip}", job.cfg.hostname));
                            candidates.insert(0, ip);
                        }
                    }
                }
            }
        }
        for ip in candidates.clone() {
            if TcpStream::connect_timeout(&std::net::SocketAddr::new(ip, 22), Duration::from_secs(2)).is_ok()
                && let Ok(fp) = fetch_host_fingerprint(&ip.to_string(), 22, Duration::from_secs(8))
            {
                // A DSM that is still up would also answer on 22; the new system advertises our hostname via mDNS or 8443.
                let mgmt = TcpStream::connect_timeout(&std::net::SocketAddr::new(ip, 8443), Duration::from_secs(2)).is_ok();
                log.phase(Phase::Verifying);
                log.info(format!("SSH is up on {ip}, host key {fp}; management plane reachable: {mgmt}"));
                if !mgmt && job.cfg.enable_mgmt {
                    // keep waiting a bit: the management service starts after networking
                    std::thread::sleep(Duration::from_secs(5));
                    if TcpStream::connect_timeout(&std::net::SocketAddr::new(ip, 8443), Duration::from_secs(2)).is_err() {
                        continue;
                    }
                }
                return Ok(Outcome {
                    hostname: job.cfg.hostname.clone(),
                    ip: Some(ip),
                    ssh_fingerprint: Some(fp),
                    mgmt_url: job.cfg.enable_mgmt.then(|| format!("https://{ip}:8443/")),
                    mgmt_reachable: true,
                    username: job.cfg.username.clone(),
                    log_path: log.path().map(|p| p.to_path_buf()),
                });
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    bail!(
        "the new system did not come up within 20 minutes. Look at the NAS console (serial/VGA) and your DHCP server's lease list for '{}'",
        job.cfg.hostname
    )
}

/// Run the whole job. Blocks; call from a worker thread.
pub fn run_install(job: &InstallJob, logger: &Logger, cancel: Arc<AtomicBool>, progress: &mut dyn FnMut(&str, f32)) -> Result<Outcome> {
    logger.add_secret(&job.display_password);
    if let Auth::Password(p) = &job.target.auth {
        logger.add_secret(p);
    }
    if let Some(p) = &job.target.sudo_password {
        logger.add_secret(p);
    }
    logger.info(format!("synofurb {} starting install on {} ({})", env!("CARGO_PKG_VERSION"), job.facts.model, job.target.host));
    logger.info(format!("plan: {}", job.plan.summary().join(" | ")));
    for d in &job.plan.destroyed {
        logger.warn(format!("WILL DESTROY: {d}"));
    }
    for d in &job.plan.preserved {
        logger.info(format!("will preserve: {} {}", d.what, d.detail));
    }

    let (tx, rx) = channel();
    logger.add_sender(tx);

    logger.phase(Phase::Connecting);
    let peer: IpAddr = job.target.host.parse().or_else(|_| {
        use std::net::ToSocketAddrs;
        (job.target.host.as_str(), 22).to_socket_addrs()?.next().map(|a| a.ip()).context("cannot resolve host")
    })?;
    let bind_ip = local_ip_towards(peer)?;
    logger.info(format!("this computer reaches the NAS from {bind_ip}"));

    let mut ssh = SshSession::connect(&job.target.host, job.target.port, &job.target.user, &job.target.auth, logger)?;

    logger.phase(Phase::Inspecting);
    let (fresh, raw) = inspect_remote(&mut ssh, false, job.target.sudo_password.clone(), logger)?;
    // The disks must still look like what the user confirmed.
    let drift: Vec<String> = job
        .plan
        .disks
        .iter()
        .filter_map(|dp| {
            let d = fresh.disks.iter().find(|d| d.ident_matches(&dp.ident))?;
            (d.parts.iter().map(|p| (p.number, p.start, p.sectors)).collect::<Vec<_>>() != dp.expect).then(|| dp.disk.clone())
        })
        .collect();
    if !drift.is_empty() {
        bail!("the partition layout of {} changed since the plan was made; re-scan and review again", drift.join(", "));
    }

    logger.phase(Phase::Staging);
    let staged = stage(job, bind_ip, &raw, logger, progress)?;

    let result = (|| -> Result<()> {
        match &job.method {
            BootMethod::Kexec { kexec_bin } => {
                if !fresh.kexec {
                    bail!("the DSM kernel on this NAS has no kexec support; use the installer kit instead");
                }
                let bin = kexec::find_kexec_binary(job.facts.arch, kexec_bin.as_deref())?;
                let kj = KexecJob {
                    arch: job.facts.arch,
                    kexec_bin: &bin,
                    kernel: &staged.files.kernel,
                    initrd: &staged.initrd,
                    dtb: staged.files.dtb.as_deref(),
                    cmdline: &staged.cmdline,
                };
                let is_root = fresh.is_root;
                let stage_dir = kexec::stage_and_load(&mut ssh, is_root, job.target.sudo_password.clone(), &kj, logger, progress)?;
                if cancel.load(Ordering::Relaxed) {
                    bail!("cancelled before the point of no return");
                }
                logger.phase(Phase::Booting);
                kexec::execute(&mut ssh, is_root, job.target.sudo_password.clone(), &stage_dir, logger)?;
            }
            BootMethod::Kit { dir } => {
                let serial = job.cfg.serial.clone();
                let url = format!("{}/preseed.cfg", staged.http_base);
                let p = KitParams {
                    facts: &job.facts,
                    cmdline: &staged.cmdline,
                    serial: &serial,
                    desktop_ip: &staged.desktop_ip.to_string(),
                    preseed_url: &url,
                    dtb_name: job.cfg.dtb.as_deref(),
                };
                kexec::export_kit(dir, &staged.files, &staged.initrd, &staged.preseed, &p)?;
                logger.info(format!("installer kit written to {}; boot the NAS from it now (see README.txt)", dir.display()));
                logger.phase(Phase::Booting);
            }
        }
        logger.phase(Phase::Installing);
        let wait = if matches!(job.method, BootMethod::Kit { .. }) { Duration::from_secs(2 * 3600) } else { Duration::from_secs(10 * 60) };
        follow_installer(&rx, &cancel, wait, logger)
    })();
    drop(ssh);
    staged.server.stop();
    if let Err(e) = result {
        logger.phase(Phase::Failed);
        logger.error(format!("{e:#}"));
        return Err(e);
    }
    let outcome = wait_for_new_system(job, Some(peer), &cancel, logger);
    match &outcome {
        Ok(o) => {
            logger.phase(Phase::Finished);
            logger.info(format!(
                "done: ssh {}@{}  |  {}",
                o.username,
                o.ip.map(|i| i.to_string()).unwrap_or_default(),
                o.mgmt_url.clone().unwrap_or_default()
            ));
        }
        Err(e) => {
            logger.phase(Phase::Failed);
            logger.error(format!("{e:#}"));
        }
    }
    outcome
}

impl Disk {
    /// Does this disk match the identity recorded in a plan (serial if known, else model+size)?
    pub fn ident_matches(&self, id: &crate::plan::DiskIdent) -> bool {
        self.sectors == id.sectors && if !id.serial.is_empty() { self.serial == id.serial } else { self.model == id.model }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::fixtures;
    use crate::remote::mock::MockShell;

    #[test]
    fn inspect_remote_parses_output_and_uses_sudo_when_not_root() {
        let mut m = MockShell::default();
        m.push_ok("1000\n"); // id -u
        m.push_ok(""); // rm of script after... (order: upload, run, rm)
        // run -> sudo-wrapped script execution result
        m.results.clear();
        m.push_ok("1000\n");
        m.push_ok(&fixtures::ds_4bay());
        let log = Logger::memory(None);
        let (facts, raw) = inspect_remote(&mut m, false, Some("pw".into()), &log).unwrap();
        assert_eq!(facts.model, "DS918+");
        assert!(!facts.is_root, "ssh user was uid 1000");
        assert!(raw.contains("END|ok"));
        assert!(m.commands.iter().any(|c| c.starts_with("sudo -S -p ''")));
        assert!(m.commands.iter().all(|c| !c.contains("pw\n")));
    }

    #[test]
    fn ident_matching() {
        let facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let d = &facts.disks[0];
        let id = crate::plan::DiskIdent { model: d.model.clone(), serial: d.serial.clone(), sectors: d.sectors };
        assert!(d.ident_matches(&id));
        let other = crate::plan::DiskIdent { serial: "different".into(), ..id };
        assert!(!d.ident_matches(&other));
    }

    #[test]
    fn follow_installer_reports_failure_and_success() {
        let (tx, rx) = channel();
        let log = Logger::memory(Some(tx));
        log.installer_line(crate::logs::Source::Installer, "x ERROR boom");
        log.installer_line(crate::logs::Source::Installer, "x @@FAIL disk missing");
        let cancel = AtomicBool::new(false);
        let e = follow_installer(&rx, &cancel, Duration::from_secs(5), &log).unwrap_err();
        assert!(e.to_string().contains("disk missing"), "{e}");

        let (tx, rx) = channel();
        let log = Logger::memory(Some(tx));
        log.installer_line(crate::logs::Source::Installer, "x @@DONE ok");
        assert!(follow_installer(&rx, &cancel, Duration::from_secs(5), &log).is_ok());
    }

    #[test]
    fn first_contact_timeout_gives_actionable_advice() {
        let (_tx, rx) = channel();
        let log = Logger::memory(None);
        let cancel = AtomicBool::new(false);
        let e = follow_installer(&rx, &cancel, Duration::from_millis(1), &log).unwrap_err();
        assert!(e.to_string().contains("serial console"));
    }
}

/// The phrase the user must type before anything is destroyed (GUI and CLI share it).
pub fn confirm_phrase(facts: &DeviceFacts) -> String {
    let name = if facts.model.is_empty() { facts.hostname.clone() } else { facts.model.clone() };
    format!("ERASE {}", name.trim().to_uppercase())
}

#[cfg(test)]
mod phrase_tests {
    use super::*;
    use crate::inspect::fixtures;

    #[test]
    fn phrase_uses_model() {
        let f = parse_inspect(&fixtures::ds_4bay()).unwrap();
        assert_eq!(confirm_phrase(&f), "ERASE DS918+");
    }
}
