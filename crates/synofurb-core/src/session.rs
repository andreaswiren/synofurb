//! The install job: inspect -> plan -> stage -> boot installer -> follow it -> verify the result.

use crate::config::{InstallConfig, random_token};
use crate::debian::{self, InstallerFiles, PreseedParams};
use crate::discovery::mdns;
use crate::inspect::{inspect_script, parse_inspect};
use crate::kexec::{self, KexecJob, KitParams};
use crate::logs::{Event, Logger, Phase};
use crate::model::*;
use crate::offline::{Bundle, MIRROR_NAME};
use crate::payload::{OfflineEnv, build_payload, build_payload_offline};
use crate::plan::Plan;
use crate::remote::{Auth, Privileged, RemoteShell, SshSession, fetch_host_fingerprint};
use crate::script::{ScriptParams, render_stage2};
use crate::server::{OFFLINE_MIRROR_ROUTE, Served, Server, local_ip_towards};
use anyhow::{Context, Result, bail};
use std::net::{IpAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// Failures of the boot-flash flows that callers (the GUI, the CLI) must tell apart. They travel inside the `anyhow::Error` the flows
/// return: `SessionError::find(&err)` or `err.downcast_ref::<SessionError>()`. Never match on the message text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The unit was asked to reboot into the boot agent and the agent did not answer within `waited_min` minutes. The state of the unit
    /// is unknown: see [`agent_timeout_message`] for the two possible outcomes. Nothing was reset by the app after the wait.
    #[error("{}", agent_timeout_message(*waited_min, *watchdog_min))]
    AgentTimeout { waited_min: u32, watchdog_min: u32 },
    /// The backup of the boot flash did not complete or did not verify. **The boot flash was not written to** (the backup is the first
    /// step and a prerequisite for every write), so DSM and the boot menu are exactly as they were. Different from "the flash changed".
    #[error("backing up the boot flash failed, so nothing was written to it and the boot menu is unchanged: {reason}")]
    FlashBackupFailed { reason: String },
}

impl SessionError {
    /// The typed error inside `e`, if there is one.
    pub fn find(e: &anyhow::Error) -> Option<&SessionError> {
        e.downcast_ref::<SessionError>()
    }

    /// Is the boot flash certainly untouched? (Only said for the failures that happen before the first write.)
    pub fn flash_untouched(&self) -> bool {
        matches!(self, SessionError::FlashBackupFailed { .. })
    }

    /// Is the default boot entry possibly still the agent (so a power cycle may boot the agent again)?
    pub fn boot_default_unconfirmed(&self) -> bool {
        matches!(self, SessionError::AgentTimeout { .. })
    }
}

/// The text of [`SessionError::AgentTimeout`]. It states both outcomes and promises nothing about DSM: when the agent never ran, the
/// unit may keep booting the agent after every power cycle (`boot/dom/README.md`, "Fail-safe design").
pub fn agent_timeout_message(waited_min: u32, watchdog_min: u32) -> String {
    format!(
        "The boot agent did not answer within {waited_min} minutes of the reboot request, and nothing was reset after the wait. \
         What happened on the unit is not known. If the agent started, its watchdog sets the default boot entry back to DSM and reboots \
         the unit {watchdog_min} minutes after the agent started (counted from the agent's start, not from the reboot request), so the \
         unit can still come back by itself. If the agent never ran, the default boot entry may still point at the agent and the unit may \
         boot the agent again after every power cycle, so do not rely on power-cycling it. To recover by hand you need physical access: \
         choose SYNOLOGY_2 in the boot loader menu on a serial or VGA console for one boot and then run `synofurb-dom default dsm --yes` \
         on DSM, or take the boot flash to another Linux computer and restore the saved image (`synofurb-dom restore --yes FILE`). \
         See docs/RECOVERY.md."
    )
}

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
    /// x86 units with an internal boot flash: back it up, add the small boot agent, reboot into it and let it
    /// `kexec` the installer. Needs only SSH; DSM stays the fail-safe default.
    Dom { agent_dir: Option<PathBuf>, backup_dir: PathBuf },
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
    /// Directory of an offline bundle (see `offline.rs`): when set, the installer files, boot agent and the Debian packages are
    /// served from it and nothing is downloaded from the Internet. `None` = online install.
    pub offline_bundle: Option<PathBuf>,
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
///
/// With `job.offline_bundle` set nothing is downloaded: the installer files come from the bundle (hashes checked
/// against its manifest), the bundle's mirror is served at `/<token>/mirror/` and `mirror_host`/`mirror_dir` point
/// the installer, `debootstrap` and `apt` at it.
pub fn stage(job: &InstallJob, bind_ip: IpAddr, inspect_raw: &str, log: &Logger, progress: &mut dyn FnMut(&str, f32)) -> Result<Staged> {
    if !job.plan.is_ok() {
        bail!("the install plan has errors: {}", job.plan.errors.join("; "));
    }
    job.cfg.validate()?;
    let (http_port, tcp_port, udp_port) = free_ports(bind_ip)?;
    let token = random_token(16);
    let http_base = format!("http://{bind_ip}:{http_port}/{token}");
    let mut cfg = job.cfg.clone();
    let bundle = match &job.offline_bundle {
        Some(dir) => {
            let b = Bundle::open(dir).with_context(|| format!("opening the offline bundle {}", dir.display()))?;
            let warnings = b.check_for_install(job.facts.arch, &cfg.suite, cfg.kernel, &cfg.extra_packages, job.with_firmware)?;
            for w in warnings {
                log.warn(w);
            }
            if !bind_ip.is_ipv4() {
                bail!(
                    "offline installs need an IPv4 address of this computer (the installer fetches packages from {bind_ip}); pick the LAN IPv4 address"
                );
            }
            // The installer, debootstrap and apt all read this: the bundle is the mirror.
            cfg.mirror_host = format!("{bind_ip}:{http_port}");
            cfg.mirror_dir = format!("/{token}/{OFFLINE_MIRROR_ROUTE}/{MIRROR_NAME}");
            cfg.validate()?;
            log.info(format!(
                "OFFLINE install: packages come from the bundle {} (suites: {}) served at http://{}{}",
                dir.display(),
                b.suites().join(", "),
                cfg.mirror_host,
                cfg.mirror_dir
            ));
            Some(b)
        }
        None => None,
    };
    let payload = match &bundle {
        Some(b) => build_payload_offline(&cfg, &job.plan, &job.facts, inspect_raw, &OfflineEnv { suites: b.suites().to_vec() })?,
        None => build_payload(&cfg, &job.plan, &job.facts, inspect_raw)?,
    };
    let run_id = random_token(4);
    let stage2 = render_stage2(&ScriptParams {
        plan: &job.plan,
        facts: &job.facts,
        cfg: &cfg,
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
    let preseed = debian::render_preseed(&PreseedParams { cfg: &cfg, http_base: &http_base });
    let files = match &bundle {
        Some(b) => {
            progress("verifying the offline bundle's installer files", 0.0);
            b.verify_installer(job.facts.arch)?;
            let mut f = b.installer_files(job.facts.arch)?;
            if !job.with_firmware {
                f.firmware = None;
            }
            if let (Arch::Armhf, Some(d)) = (job.facts.arch, cfg.dtb.as_deref()) {
                f.dtb = b.dtb(job.facts.arch, d);
                if f.dtb.is_none() {
                    log.warn(format!("{d} is not in the offline bundle; supply your own DTB"));
                }
            }
            progress("verifying the offline bundle's installer files", 1.0);
            f
        }
        None => debian::fetch_installer(job.facts.arch, &cfg.suite, &job.cache_dir, job.with_firmware, cfg.dtb.as_deref(), log, progress)?,
    };
    let initrd = job.cache_dir.join(format!("initrd-synofurb-{run_id}.gz"));
    std::fs::create_dir_all(&job.cache_dir)?;
    debian::build_initrd(&files, &preseed, &initrd)?;
    let cmdline = debian::installer_cmdline(&cfg.serial, &bind_ip.to_string(), udp_port, job.verbose_installer, None);
    log.info(format!("installer kernel command line: {cmdline}"));
    let mut served_files = std::collections::HashMap::new();
    served_files.insert("installer-kernel".to_string(), files.kernel.clone());
    served_files.insert("installer-initrd".to_string(), initrd.clone());
    let server = Server::start(
        bind_ip,
        Served {
            files: served_files,
            token,
            stage2,
            payload: payload.bytes,
            preseed: preseed.clone(),
            mirror: bundle.as_ref().map(|b| b.mirror_dir()),
        },
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

/// Persistent per-unit key for the boot agent (so the agent can be re-entered later from this computer).
pub fn agent_key_for(unit: &str, dir: &std::path::Path) -> Result<crate::config::SshKeypair> {
    let safe: String = unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    std::fs::create_dir_all(dir)?;
    let (priv_p, pub_p) = (dir.join(format!("agent_{safe}_ed25519")), dir.join(format!("agent_{safe}_ed25519.pub")));
    if let (Ok(k), Ok(pubk)) = (std::fs::read_to_string(&priv_p), std::fs::read_to_string(&pub_p)) {
        return Ok(crate::config::SshKeypair { private_openssh: k, public_openssh: pubk.trim().to_string(), fingerprint: String::new() });
    }
    let kp = crate::domboot::agent_session_key(&safe)?;
    std::fs::write(&priv_p, &kp.private_openssh)?;
    std::fs::write(&pub_p, format!("{}\n", kp.public_openssh))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&priv_p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(kp)
}

/// Console arguments for the agent: whatever DSM uses (ttyS0 or an AMD MMIO UART) plus the VGA console.
fn agent_cmdline(facts: &DeviceFacts) -> String {
    format!("console=tty0 {} panic=30", facts.serial_console().kernel_arg())
}

/// DOM flow up to and including handing control to the Debian installer.
#[allow(clippy::too_many_arguments)]
fn dom_install_flow(
    job: &InstallJob,
    fresh: &DeviceFacts,
    ssh: &mut SshSession,
    agent_dir: Option<&std::path::Path>,
    backup_dir: &std::path::Path,
    staged: &Staged,
    cancel: &AtomicBool,
    logger: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<()> {
    use crate::domboot::*;
    let dom = fresh
        .dom
        .as_ref()
        .filter(|d| d.is_editable() && fresh.firmware == Firmware::Dom)
        .context("this unit has no editable internal boot flash (DOM); use the installer kit instead")?;
    let is_root = fresh.is_root;
    let pw = job.target.sudo_password.clone();
    let unit = if fresh.serial.is_empty() { job.target.host.clone() } else { fresh.serial.clone() };
    let backup = backup_dom(ssh, is_root, pw.clone(), backup_dir, &unit, dom.sectors * SECTOR, logger)?; // typed: SessionError::FlashBackupFailed
    logger
        .warn(format!("KEEP THIS FILE: {} is the only way to rebuild the unit's boot flash if it is ever damaged", backup.path.display()));
    let files = match &job.offline_bundle {
        // offline: the agent comes from the bundle, never from the Internet
        Some(dir) => {
            let b = Bundle::open(dir)?;
            let adir = b.agent_dir(job.facts.arch).with_context(|| {
                format!(
                    "the offline bundle has no boot agent for {}; build it with `synofurb bundle create` (without --no-agent) while online",
                    job.facts.arch.debian()
                )
            })?;
            AgentFiles::locate(job.facts.arch, Some(&adir))?
        }
        None => AgentFiles::locate_or_fetch(job.facts.arch, agent_dir, &job.cache_dir, logger, progress)?,
    };
    let key = agent_key_for(&unit, &crate::session::config_keys_dir())?;
    let overlay = AgentOverlay {
        hostname: format!("{}-agent", job.cfg.hostname),
        timeout_min: crate::timing::INSTALL_WATCHDOG_MIN,
        authorized_keys: vec![key.public_openssh.clone()],
    };
    let dsm_fp = ssh.host_key_fingerprint();
    let result = (|| -> Result<()> {
        install_agent(
            ssh,
            is_root,
            pw.clone(),
            crate::payload::DOM_SCRIPT,
            &files,
            &overlay,
            &agent_cmdline(fresh),
            true,
            logger,
            progress,
        )?;
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled before the reboot");
        }
        logger.phase(Phase::Booting);
        logger.warn("rebooting DSM into the boot agent now (DSM services stop)");
        let mut p = Privileged { sh: &mut *ssh, is_root, sudo_password: pw.clone() };
        let _ = p.run("nohup /bin/sh -c 'sleep 2; sync; reboot' >/dev/null 2>&1 &")?;
        Ok(())
    })();
    if let Err(e) = result {
        // The agent entry may be armed: make sure the next boot is DSM again.
        let _ = default_to_dsm(ssh, is_root, pw.clone());
        return Err(e);
    }
    let peer: IpAddr = job.target.host.parse().context("host must be an IP address for the DOM flow")?;
    // typed on timeout: SessionError::AgentTimeout
    let (mut agent, _ip) = wait_for_agent(
        &[peer],
        &key.private_openssh,
        dsm_fp.as_deref(),
        AgentWait {
            timeout: Duration::from_secs(crate::timing::AGENT_WAIT_MIN as u64 * 60),
            watchdog_min: crate::timing::INSTALL_WATCHDOG_MIN,
        },
        cancel,
        logger,
    )?;
    agent.exec("sf-boot claim", None)?;
    let k = format!("{}/files/installer-kernel", staged.http_base);
    let i = format!("{}/files/installer-initrd", staged.http_base);
    let kh = crate::debian::sha256_hex(&std::fs::read(&staged.files.kernel)?);
    let ih = crate::debian::sha256_hex(&std::fs::read(&staged.initrd)?);
    let cmd = agent_kexec_command(&k, &i, &staged.cmdline, &kh, &ih)?;
    logger.info("handing over to the Debian installer from the boot agent");
    let r = agent.exec(&cmd, None)?;
    if !r.ok() {
        let _ = agent.exec("sf-boot reboot dsm", None);
        bail!("the boot agent could not start the installer: {} {}", r.stdout.trim(), r.stderr.trim());
    }
    Ok(())
}

/// Per-user data directory of synofurb (logs, backups, caches, keys).
pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "synofurb", "synofurb")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".synofurb"))
}

/// Directory for SSH keys generated by synofurb.
pub fn config_keys_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "synofurb", "synofurb")
        .map(|d| d.data_dir().join("keys"))
        .unwrap_or_else(|| PathBuf::from(".synofurb/keys"))
}

/// Install the boot agent only (no OS install): the "boot manager" mode.
pub fn boot_manager_install(
    target: &SshTarget,
    agent_dir: Option<&std::path::Path>,
    backup_dir: &std::path::Path,
    hostname: &str,
    arm: bool,
    logger: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<()> {
    use crate::domboot::*;
    let mut ssh = SshSession::connect(&target.host, target.port, &target.user, &target.auth, logger)?;
    let (facts, _) = inspect_remote(&mut ssh, false, target.sudo_password.clone(), logger)?;
    let dom = facts
        .dom
        .as_ref()
        .filter(|d| d.is_editable() && facts.firmware == Firmware::Dom)
        .context("this unit has no editable internal boot flash (DOM)")?;
    let unit = if facts.serial.is_empty() { target.host.clone() } else { facts.serial.clone() };
    backup_dom(&mut ssh, facts.is_root, target.sudo_password.clone(), backup_dir, &unit, dom.sectors * SECTOR, logger)?;
    let files = AgentFiles::locate_or_fetch(
        facts.arch,
        agent_dir,
        &directories::ProjectDirs::from("dev", "synofurb", "synofurb")
            .map(|d| d.data_dir().join("cache"))
            .unwrap_or_else(|| PathBuf::from(".synofurb/cache")),
        logger,
        progress,
    )?;
    let key = agent_key_for(&unit, &config_keys_dir())?;
    let overlay = AgentOverlay {
        hostname: hostname.to_string(),
        timeout_min: crate::timing::BOOT_MANAGER_WATCHDOG_MIN,
        authorized_keys: vec![key.public_openssh],
    };
    install_agent(
        &mut ssh,
        facts.is_root,
        target.sudo_password.clone(),
        crate::payload::DOM_SCRIPT,
        &files,
        &overlay,
        &agent_cmdline(&facts),
        arm,
        logger,
        progress,
    )?;
    logger.info("boot agent installed. DSM is still the default; use `synofurb boot-manager enter` to boot into it.");
    Ok(())
}

/// Recovery: make the unit boot DSM again (works from DSM, the installed Debian, or the boot agent).
pub fn recover_to_dsm(target: &SshTarget, logger: &Logger) -> Result<String> {
    let mut ssh = SshSession::connect(&target.host, target.port, &target.user, &target.auth, logger)?;
    let id = ssh.exec("id -u", None)?;
    let out = crate::domboot::default_to_dsm(&mut ssh, id.stdout.trim() == "0", target.sudo_password.clone())?;
    logger.info("the boot flash default is DSM again; reboot the unit to return to it");
    Ok(out)
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

    // Back up the first/last MiB of every disk the plan touches before anything can change.
    let touched: Vec<(String, u64)> =
        job.plan.disks.iter().filter_map(|dp| fresh.disk(&dp.disk).map(|d| (d.name.clone(), d.sectors))).collect();
    let meta_root = data_dir().join("backups");
    let unit = if fresh.serial.is_empty() { job.target.host.clone() } else { fresh.serial.clone() };
    let meta = crate::diskbackup::backup_disk_metadata(
        &mut ssh,
        fresh.is_root,
        job.target.sudo_password.clone(),
        &touched,
        &meta_root,
        &unit,
        logger,
    )?;
    logger.warn(format!("disk metadata backup (partition tables, not your files): {}", meta.display()));

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
            BootMethod::Dom { agent_dir, backup_dir } => {
                dom_install_flow(job, &fresh, &mut ssh, agent_dir.as_deref(), backup_dir, &staged, &cancel, logger, progress)?;
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
            if let (Some(ip), Some(fp)) = (o.ip, o.ssh_fingerprint.as_deref())
                && crate::hostkey::repin_after_install(&ip.to_string(), 22, fp)
            {
                logger.info(format!("pinned the new system's SSH host key {fp} for {ip} (the old key was DSM's)"));
            }
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
    fn the_agent_timeout_is_typed_and_states_both_outcomes_without_promising_dsm() {
        let e = anyhow::Error::new(SessionError::AgentTimeout { waited_min: 10, watchdog_min: 30 });
        let found = SessionError::find(&e).expect("typed, found by downcast");
        assert_eq!(found, &SessionError::AgentTimeout { waited_min: 10, watchdog_min: 30 });
        assert!(found.boot_default_unconfirmed() && !found.flash_untouched());
        let t = e.to_string();
        let l = t.to_lowercase();
        // outcome 1: the agent started, the watchdog resets the default, counted from the agent's start
        assert!(t.contains("10 minutes") && t.contains("30 minutes after the agent started"), "{t}");
        // outcome 2: the agent never ran
        assert!(
            l.contains("never ran") && l.contains("again after every power cycle") && l.contains("do not rely on power-cycling"),
            "{t}"
        );
        // manual recovery, both routes
        assert!(t.contains("SYNOLOGY_2") && t.contains("serial or VGA console") && t.contains("another Linux computer"), "{t}");
        assert!(t.contains("synofurb-dom restore --yes FILE") && t.contains("docs/RECOVERY.md"));
        // the false promises of the old text must not come back
        for bad in ["boot dsm normally", "defaults to dsm", "dsm is the default", "dsm is already", "starts as before"] {
            assert!(!l.contains(bad), "{bad}: {t}");
        }
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
