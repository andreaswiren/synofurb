//! Command line interface: everything the GUI does, scriptable, plus dev helpers.

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use synofurb_core::config::*;
use synofurb_core::discovery::{self, ScanOptions};
use synofurb_core::logs::{Event, Level, Logger};
use synofurb_core::model::*;
use synofurb_core::plan::*;
use synofurb_core::remote::{Auth, RemoteShell, SshSession};
use synofurb_core::session::*;

#[derive(Parser)]
#[command(name = "synofurb", version, about = "Reinstall Synology NAS units with plain Debian 13 (trixie)")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once at startup; boxing would only obscure the clap derive
pub enum Cmd {
    /// Open the graphical wizard (default)
    Gui {
        /// load a built-in demo NAS (4-bay DS918+) instead of scanning; nothing touches the network
        #[arg(long)]
        demo: bool,
        /// with --demo: start on this screen (inventory, connect, inspect, goal, plan, system, confirm, install, failed,
        /// done, update, settings, bootmgr, recover)
        #[arg(long, default_value = "inventory")]
        screen: String,
    },
    /// Find Synology NAS units on the network
    Scan {
        #[arg(long)]
        json: bool,
        /// skip the /24 port sweep (multicast/broadcast methods only)
        #[arg(long)]
        no_sweep: bool,
    },
    /// Inspect a running DSM over SSH (read-only) and show the install plan
    Plan {
        #[command(flatten)]
        conn: Conn,
        #[command(flatten)]
        opts: PlanOpts,
        /// print the whole plan as JSON
        #[arg(long)]
        json: bool,
    },
    /// Install Debian. DESTROYS DATA. Needs --accept-data-loss and --confirm
    Install {
        #[command(flatten)]
        conn: Conn,
        #[command(flatten)]
        opts: PlanOpts,
        #[command(flatten)]
        acct: Account,
        #[arg(long, value_enum, default_value = "auto")]
        method: Method,
        /// which Linux kernel to install
        #[arg(long, value_enum, default_value = "newest")]
        kernel: KernelArg,
        /// directory with agent-<arch>.vmlinuz/.initrd (default: assets/agent or $SYNOFURB_AGENT_DIR)
        #[arg(long)]
        agent_dir: Option<PathBuf>,
        /// where the boot flash backup is written (default: <data dir>/dom-backups)
        #[arg(long)]
        backup_dir: Option<PathBuf>,
        /// static kexec binary for the NAS architecture (see docs/kexec.md)
        #[arg(long)]
        kexec: Option<PathBuf>,
        /// output folder for --method kit
        #[arg(long, default_value = "synofurb-kit")]
        kit_dir: PathBuf,
        #[arg(long)]
        no_firmware: bool,
        /// I understand that data on this NAS can be destroyed, even data marked as preserved
        #[arg(long)]
        accept_data_loss: bool,
        /// I have a complete, verified backup
        #[arg(long)]
        have_backup: bool,
        /// type the confirmation phrase shown by `plan` (for example "ERASE DS918+")
        #[arg(long)]
        confirm: Option<String>,
    },
    /// Boot manager mode: add a small SSH-controllable boot environment to the unit's internal boot flash so you
    /// can netboot/kexec any OS later, always with DSM as the fail-safe default
    BootManager {
        #[command(subcommand)]
        action: BootManagerAction,
    },
    /// Recovery: make the unit boot the original DSM again (from DSM, Debian or the boot agent)
    Recover {
        #[command(flatten)]
        conn: Conn,
    },
    /// Check for (and install) a newer synofurb from the GitHub releases; verifies SHA-256 and the minisign signature
    SelfUpdate {
        /// only report whether an update exists
        #[arg(long)]
        check: bool,
        /// `stable` (default) or `prerelease`
        #[arg(long)]
        channel: Option<String>,
    },
    /// Look up a Synology model in the compatibility list (embedded copy; `--refresh` fetches the latest from the repo)
    Compat {
        /// model name, e.g. "DS923+" or "RS3618xs"
        model: Option<String>,
        #[arg(long)]
        refresh: bool,
    },
    /// Download and verify the Debian installer files (kernel/initrd) for an architecture
    FetchInstaller {
        #[arg(value_enum)]
        arch: ArchArg,
        #[arg(long, default_value = "trixie")]
        suite: String,
    },
}

#[derive(Subcommand)]
pub enum BootManagerAction {
    /// Back up the boot flash, then install the agent (DSM stays the default unless --arm)
    Install {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        agent_dir: Option<PathBuf>,
        #[arg(long)]
        backup_dir: Option<PathBuf>,
        /// boot into the agent on the next reboot (it resets the default to DSM itself)
        #[arg(long)]
        arm: bool,
    },
    /// Arm the agent for the next boot, reboot DSM, wait for it and print how to use it
    Enter {
        #[command(flatten)]
        conn: Conn,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum ArchArg {
    Amd64,
    Arm64,
    Armhf,
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Method {
    /// choose by hardware: boot flash (DOM) units use `dom`, units with kexec use `kexec`, otherwise `kit`
    Auto,
    /// through the internal boot flash and the boot agent (x86 Synology), SSH only
    Dom,
    /// upload a static kexec to DSM
    Kexec,
    /// export an installer kit and boot it yourself (USB/TFTP/U-Boot)
    Kit,
}

#[derive(Copy, Clone, ValueEnum)]
pub enum KernelArg {
    /// Linux 7.x from trixie-backports
    Newest,
    /// Debian 13's 6.12 LTS kernel
    Stable,
}

#[derive(Args, Clone)]
pub struct Conn {
    #[arg(long)]
    pub host: String,
    #[arg(long, default_value_t = 22)]
    pub port: u16,
    #[arg(long, default_value = "admin")]
    pub user: String,
    /// environment variable holding the SSH password (default SYNOFURB_PASSWORD; prompts if unset)
    #[arg(long, default_value = "SYNOFURB_PASSWORD")]
    pub password_env: String,
    /// use this OpenSSH private key instead of a password
    #[arg(long)]
    pub key: Option<PathBuf>,
    /// environment variable with the sudo password if different from the SSH password
    #[arg(long, default_value = "SYNOFURB_SUDO_PASSWORD")]
    pub sudo_password_env: String,
}

#[derive(Args, Clone)]
pub struct PlanOpts {
    #[arg(long, value_enum, default_value = "wipe")]
    pub mode: ModeArg,
    /// system size in GiB (0 = recommended)
    #[arg(long, default_value_t = 0)]
    pub system_gib: u64,
    /// number of disks carrying a copy of the system (default: automatic)
    #[arg(long)]
    pub copies: Option<usize>,
    #[arg(long, value_enum, default_value = "auto")]
    pub raid: RaidArg,
    /// percent of the data volume group left unallocated (0-90)
    #[arg(long, default_value_t = 20)]
    pub free_percent: u8,
    /// with --mode keep: arrays to preserve (default all data arrays), e.g. --keep md2
    #[arg(long)]
    pub keep: Vec<String>,
}

#[derive(Args, Clone)]
pub struct Account {
    #[arg(long, default_value = "nas")]
    pub hostname: String,
    #[arg(long, default_value = "nasadmin")]
    pub username: String,
    /// file with the new account's password (a strong one is generated if omitted)
    #[arg(long)]
    pub password_file: Option<PathBuf>,
    /// public key file to authorise for SSH (a new ed25519 key is generated if omitted)
    #[arg(long)]
    pub ssh_pubkey: Option<PathBuf>,
    #[arg(long, default_value = "UTC")]
    pub timezone: String,
    #[arg(long)]
    pub no_mgmt: bool,
    /// board device tree for ARM U-Boot units (file name known to Debian, e.g. armada-385-synology-ds116.dtb)
    #[arg(long)]
    pub dtb: Option<String>,
}

#[derive(Copy, Clone, ValueEnum)]
pub enum ModeArg {
    Wipe,
    Keep,
}
#[derive(Copy, Clone, ValueEnum)]
pub enum RaidArg {
    Auto,
    None,
    #[value(name = "0")]
    R0,
    #[value(name = "1")]
    R1,
    #[value(name = "5")]
    R5,
    #[value(name = "6")]
    R6,
    #[value(name = "10")]
    R10,
}

impl PlanOpts {
    pub fn request(&self) -> PlanRequest {
        PlanRequest {
            mode: match self.mode {
                ModeArg::Wipe => StorageMode::WipeAll,
                ModeArg::Keep => StorageMode::KeepVolumes,
            },
            system_copies: self.copies,
            system_size_mib: self.system_gib * 1024,
            data_raid: match self.raid {
                RaidArg::Auto => DataRaid::Auto,
                RaidArg::None => DataRaid::None,
                RaidArg::R0 => DataRaid::Raid0,
                RaidArg::R1 => DataRaid::Raid1,
                RaidArg::R5 => DataRaid::Raid5,
                RaidArg::R6 => DataRaid::Raid6,
                RaidArg::R10 => DataRaid::Raid10,
            },
            data_free_percent: self.free_percent,
            keep_arrays: if self.keep.is_empty() { None } else { Some(self.keep.clone()) },
        }
    }
}

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "synofurb", "synofurb")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".synofurb"))
}

fn secret(env: &str, prompt: &str) -> Result<String> {
    if let Ok(v) = std::env::var(env) {
        return Ok(v);
    }
    rpassword::prompt_password(prompt).context("reading password")
}

impl Conn {
    pub fn target(&self) -> Result<SshTarget> {
        let (auth, sudo) = match &self.key {
            Some(k) => (
                Auth::Key(std::fs::read_to_string(k).with_context(|| format!("reading {}", k.display()))?),
                std::env::var(&self.sudo_password_env).ok(),
            ),
            None => {
                let pw = secret(&self.password_env, &format!("SSH password for {}@{}: ", self.user, self.host))?;
                (Auth::Password(pw.clone()), Some(std::env::var(&self.sudo_password_env).unwrap_or(pw)))
            }
        };
        Ok(SshTarget { host: self.host.clone(), port: self.port, user: self.user.clone(), auth, sudo_password: sudo })
    }
}

pub fn inspect_target(t: &SshTarget, log: &Logger) -> Result<(DeviceFacts, String)> {
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, log)?;
    let r = inspect_remote(&mut ssh, false, t.sudo_password.clone(), log);
    ssh.close();
    r
}

fn cli_logger(verbose: bool) -> Result<Logger> {
    let (tx, rx) = std::sync::mpsc::channel::<Event>();
    let log = Logger::to_dir(&config_dir().join("logs"), Some(tx))?;
    std::thread::spawn(move || {
        for e in rx {
            match e {
                Event::Log(l) if verbose || l.level >= Level::Info => {
                    eprintln!("{} {:5} [{}] {}", l.ts, format!("{:?}", l.level).to_uppercase(), l.source.tag(), l.msg)
                }
                Event::Progress { what, fraction } => eprint!("\r  {what}: {:3.0}%   ", fraction * 100.0),
                _ => {}
            }
        }
    });
    Ok(log)
}

fn print_plan(facts: &DeviceFacts, plan: &Plan) {
    let info = synofurb_core::platform::classify(&facts.unique, &facts.uname_m);
    println!(
        "{} | {} | {} | DSM {} | {} MiB RAM | firmware {}",
        facts.model,
        info.soc,
        facts.arch.label(),
        facts.dsm_version,
        facts.mem_kib / 1024,
        facts.firmware.label()
    );
    println!("platform support: {} - {}", info.support.label(), info.note);
    println!("\nDisks:");
    for d in facts.candidate_disks() {
        println!("  {:8} {:>10}  {}  {}  ({} partitions)", d.name, d.human_size(), d.model, d.serial, d.parts.len());
    }
    println!("\nPlan ({:?}):", plan.mode);
    for l in plan.summary() {
        println!("  {l}");
    }
    println!(
        "  system size: {} GiB (min {}, recommended {}, max {})",
        plan.sizes.chosen_system_mib / 1024,
        plan.sizes.min_system_mib / 1024,
        plan.sizes.recommended_system_mib / 1024,
        plan.sizes.max_system_mib / 1024
    );
    if !plan.destroyed.is_empty() {
        println!("\nWILL BE DESTROYED:");
        plan.destroyed.iter().for_each(|d| println!("  - {d}"));
    }
    if !plan.preserved.is_empty() {
        println!("\nWill be preserved (no guarantee, see warnings):");
        plan.preserved.iter().for_each(|d| println!("  - {}: {}", d.what, d.detail));
    }
    plan.warnings.iter().for_each(|w| println!("warning: {w}"));
    plan.errors.iter().for_each(|e| println!("ERROR: {e}"));
    println!("\nconfirmation phrase for install: \"{}\"", confirm_phrase(facts));
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Gui { .. } => unreachable!("handled in main"),
        Cmd::Scan { json, no_sweep } => {
            let log = Logger::memory(None);
            let inv = discovery::scan(&ScanOptions { sweep: !no_sweep, ..Default::default() }, &log);
            if json {
                println!("{}", serde_json::to_string_pretty(&inv)?);
            } else {
                let n = inv.synology_devices().count();
                println!("{n} Synology device(s) found");
                for d in inv.synology_devices() {
                    println!(
                        "  {:28} mac {:17} DSM {:12} serial {:14} via {}",
                        d.title(),
                        d.mac.clone().unwrap_or_default(),
                        d.dsm_version,
                        d.serial,
                        d.sources.join(",")
                    );
                }
            }
            inv.save(&config_dir().join("inventory.json"))?;
            Ok(())
        }
        Cmd::Plan { conn, opts, json } => {
            let log = cli_logger(false)?;
            let (facts, _) = inspect_target(&conn.target()?, &log)?;
            let plan = build_plan(&facts, &opts.request());
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print_plan(&facts, &plan);
            }
            Ok(())
        }
        Cmd::SelfUpdate { check, channel } => crate::update_cmd::run(check, channel),
        Cmd::Compat { model, refresh } => {
            use synofurb_core::compat::CompatDb;
            let cache = config_dir().join("compat");
            if refresh {
                let r = CompatDb::refresh(&cache, "https://raw.githubusercontent.com/andreaswiren/synofurb/main/compat/models.json")?;
                println!("refresh: {r:?}");
            }
            let db = CompatDb::load_best(&cache);
            match model {
                Some(m) => match db.lookup(&m) {
                    Some(e) => println!(
                        "{} ({}, {}): {}\n  {}\n  platform {} | cpu {} | boot {:?} | evidence items: {}",
                        e.model,
                        e.arch.debian(),
                        e.status.label(),
                        e.status.explain(),
                        e.notes.clone().unwrap_or_default(),
                        e.platform,
                        e.cpu.clone().unwrap_or_default(),
                        e.boot,
                        e.evidence.len()
                    ),
                    None => println!("{m}: not in the list. Treated as untested; please file a compatibility report."),
                },
                None => {
                    println!("{} models (list updated {})", db.len(), db.updated());
                    for (s, n) in db.count_by_status() {
                        println!("  {:<18} {n}", s.label());
                    }
                }
            }
            Ok(())
        }
        Cmd::Recover { conn } => {
            let log = cli_logger(true)?;
            let out = recover_to_dsm(&conn.target()?, &log)?;
            println!(
                "{}\nThe unit will boot DSM at its next reboot. If DSM's system partition is gone (a failed install), DSM starts in its recovery mode: open https://find.synology.com and choose to reinstall DSM; Synology keeps data volumes on a reinstall (see https://kb.synology.com).",
                out.trim()
            );
            Ok(())
        }
        Cmd::BootManager { action } => {
            let log = cli_logger(true)?;
            match action {
                BootManagerAction::Install { conn, agent_dir, backup_dir, arm } => {
                    let backup_dir = backup_dir.unwrap_or_else(|| config_dir().join("dom-backups"));
                    boot_manager_install(&conn.target()?, agent_dir.as_deref(), &backup_dir, "synofurb-agent", arm, &log, &mut |_, _| {})?;
                    println!("Boot agent installed. Backup of the boot flash: {}", backup_dir.display());
                    Ok(())
                }
                BootManagerAction::Enter { conn } => {
                    let t = conn.target()?;
                    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log)?;
                    let fp = ssh.host_key_fingerprint();
                    let (facts, _) = inspect_remote(&mut ssh, false, t.sudo_password.clone(), &log)?;
                    let unit = if facts.serial.is_empty() { t.host.clone() } else { facts.serial.clone() };
                    let key = agent_key_for(&unit, &config_keys_dir())?;
                    let mut p =
                        synofurb_core::remote::Privileged { sh: &mut ssh, is_root: facts.is_root, sudo_password: t.sudo_password.clone() };
                    let r = p.run("synofurb-dom default agent --yes || /usr/local/bin/synofurb-dom default agent --yes")?;
                    if !r.ok() {
                        bail!("could not arm the agent (is it installed? run `synofurb boot-manager install` first): {}", r.stdout.trim());
                    }
                    let _ = p.run("nohup /bin/sh -c 'sleep 2; sync; reboot' >/dev/null 2>&1 &");
                    let ip: std::net::IpAddr = t.host.parse().context("host must be an IP address")?;
                    let (agent, ip) = synofurb_core::domboot::wait_for_agent(
                        &[ip],
                        &key.private_openssh,
                        fp.as_deref(),
                        std::time::Duration::from_secs(600),
                        &AtomicBool::new(false),
                        &log,
                    )?;
                    drop(agent);
                    println!(
                        "\nBoot agent is running at {ip}. You control the bootloader over SSH:\n  ssh -i {}/agent_{unit}_ed25519 root@{ip}\n  sf-boot status | sf-boot kexec <kernel-url> <initrd-url> --cmdline '...' | sf-boot reboot dsm|debian\nIt returns to DSM by itself if nobody claims it within the timeout.",
                        config_keys_dir().display()
                    );
                    Ok(())
                }
            }
        }
        Cmd::FetchInstaller { arch, suite } => {
            let log = cli_logger(true)?;
            let a = match arch {
                ArchArg::Amd64 => Arch::X86_64,
                ArchArg::Arm64 => Arch::Arm64,
                ArchArg::Armhf => Arch::Armhf,
            };
            let f = synofurb_core::debian::fetch_installer(a, &suite, &config_dir().join("cache"), false, None, &log, &mut |_, _| {})?;
            println!("kernel: {}\ninitrd: {}", f.kernel.display(), f.initrd.display());
            Ok(())
        }
        Cmd::Install {
            conn,
            opts,
            acct,
            method,
            kernel,
            agent_dir,
            backup_dir,
            kexec,
            kit_dir,
            no_firmware,
            accept_data_loss,
            have_backup,
            confirm,
        } => {
            let log = cli_logger(true)?;
            let target = conn.target()?;
            let (facts, _) = inspect_target(&target, &log)?;
            let plan = build_plan(&facts, &opts.request());
            print_plan(&facts, &plan);
            if !plan.is_ok() {
                bail!("the plan has errors; nothing was changed");
            }
            let info = synofurb_core::platform::classify(&facts.unique, &facts.uname_m);
            if info.support == synofurb_core::platform::Support::Unsupported {
                bail!("this NAS platform is not supported: {}", info.note);
            }
            if !accept_data_loss || !have_backup || confirm.as_deref() != Some(&confirm_phrase(&facts)) {
                bail!(
                    "refusing to continue: pass --have-backup --accept-data-loss --confirm \"{}\" after reading the plan above. There is NO guarantee any data on this NAS survives, including volumes marked as preserved. Back up first.",
                    confirm_phrase(&facts)
                );
            }
            let password = match &acct.password_file {
                Some(p) => std::fs::read_to_string(p)?.trim().to_string(),
                None => generate_password(20),
            };
            let mut cfg = InstallConfig::new(&acct.hostname, &acct.username, hash_password(&password)?);
            cfg.timezone = acct.timezone.clone();
            cfg.serial = facts.serial_console();
            cfg.enable_mgmt = !acct.no_mgmt;
            cfg.kernel = match kernel {
                KernelArg::Newest => KernelChannel::Newest,
                KernelArg::Stable => KernelChannel::Stable,
            };
            cfg.dtb = acct.dtb.clone().or_else(|| synofurb_core::platform::mainline_dtb(&facts.model).map(String::from));
            let mut key_note = String::new();
            match &acct.ssh_pubkey {
                Some(p) => cfg.ssh_authorized_keys.push(std::fs::read_to_string(p)?.trim().to_string()),
                None => {
                    let kp = generate_ssh_keypair(&format!("synofurb-{}", acct.hostname))?;
                    let dir = config_dir().join("keys");
                    std::fs::create_dir_all(&dir)?;
                    let path = dir.join(format!("{}_ed25519", acct.hostname));
                    std::fs::write(&path, &kp.private_openssh)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                    }
                    key_note = format!("generated SSH key: {} ({})", path.display(), kp.fingerprint);
                    cfg.ssh_authorized_keys.push(kp.public_openssh);
                }
            }
            cfg.validate()?;
            let (facts_firmware_is_dom, facts_kexec) = (facts.firmware == Firmware::Dom, facts.kexec);
            let job = InstallJob {
                target,
                facts,
                plan,
                cfg,
                method: {
                    let backup_dir = backup_dir.unwrap_or_else(|| config_dir().join("dom-backups"));
                    let method = match method {
                        Method::Auto if facts_firmware_is_dom => Method::Dom,
                        Method::Auto if facts_kexec => Method::Kexec,
                        Method::Auto => Method::Kit,
                        m => m,
                    };
                    match method {
                        Method::Dom => BootMethod::Dom { agent_dir, backup_dir },
                        Method::Kexec => BootMethod::Kexec { kexec_bin: kexec },
                        _ => BootMethod::Kit { dir: kit_dir },
                    }
                },
                cache_dir: config_dir().join("cache"),
                with_firmware: !no_firmware,
                verbose_installer: true,
                display_password: password.clone(),
            };
            let out = run_install(&job, &log, Arc::new(AtomicBool::new(false)), &mut |_, _| {})?;
            println!("\n=== INSTALLED ===");
            println!("hostname:   {}", out.hostname);
            println!("address:    {}", out.ip.map(|i| i.to_string()).unwrap_or_else(|| "see your DHCP server".into()));
            println!("login:      {} / {}", out.username, password);
            println!(
                "ssh:        ssh {}@{}   (host key {})",
                out.username,
                out.ip.map(|i| i.to_string()).unwrap_or_default(),
                out.ssh_fingerprint.unwrap_or_default()
            );
            if !key_note.is_empty() {
                println!("{key_note}");
            }
            if let Some(u) = out.mgmt_url {
                println!("management: {u}  (self-signed certificate)");
            }
            println!("serial console: {} at {} baud (also on USB serial adapters)", job.cfg.serial.tty, job.cfg.serial.speed);
            if let Some(p) = out.log_path {
                println!("debug log:  {}", p.display());
            }
            Ok(())
        }
    }
}
