//! `wizard`: the app's nine steps in plain terminal prompts.
//!
//!  1 Find the NAS   2 Connect   3 Inspect   4 Goal   5 Storage   6 System   7 Confirm   8 Install   9 Done
//!
//! It uses the same core calls as the app (`inspect_remote`, `build_plan`, `session::run_install`, the gate). All questions go through
//! [`Prompter`] and all network work through [`Backend`], so the whole flow is tested with scripted answers and a fake backend.
//! Prompts are line based and ASCII only: they work over SSH, on a serial console and with `TERM=dumb`. Without a terminal the wizard
//! refuses and points at the flags.

use super::args::{GateOpts, MethodArg, WizardArgs};
use super::boot;
use super::dryrun;
use super::exit::{CliError, CliResult};
use super::facts::{
    Assessment, Inspection, Source, assess, demo_facts, inspect_live, load_compat_db, method_available, parse_facts_text,
    resolve_auto_method,
};
use super::gate;
use super::install::execute;
use super::out::Out;
use super::prompt::{Prompter, StdPrompter};
use super::render;
use super::scan::{device_line, inventory_path, scan_stream};
use super::spec::{InstallSpec, KeyChoice, PasswordChoice, build_job, build_material, parse_serial, resolve_method, serial_text};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::Duration;
use synofurb_core::config::KernelChannel;
use synofurb_core::discovery::{Found, Inventory, ScanOptions};
use synofurb_core::model::*;
use synofurb_core::plan::{DataRaid, Plan, PlanRequest, StorageMode, build_plan, default_system_copies};
use synofurb_core::remote::Auth;
use synofurb_core::session::{InstallJob, SshTarget, confirm_phrase};

/// Everything the wizard does to the outside world.
pub trait Backend {
    /// Units remembered from earlier scans.
    fn known(&mut self) -> Vec<Found>;
    /// Scan the network now.
    fn scan(&mut self, out: &Out) -> Vec<Found>;
    fn inspect(&mut self, t: &SshTarget) -> Result<(DeviceFacts, String)>;
    fn install(
        &mut self,
        out: &Out,
        job: InstallJob,
        mat: &super::spec::Material,
        method: &str,
        save: Option<&std::path::Path>,
    ) -> CliResult<()>;
    fn recover(&mut self, out: &Out, t: SshTarget) -> CliResult<()>;
    fn boot_manager(&mut self, out: &Out, t: SshTarget, agent_hostname: String, arm: bool) -> CliResult<()>;
}

pub struct RealBackend;

impl Backend for RealBackend {
    fn known(&mut self) -> Vec<Found> {
        Inventory::load(&inventory_path()).devices.into_iter().filter(|d| d.synology || d.synofurb_debian).collect()
    }

    fn scan(&mut self, out: &Out) -> Vec<Found> {
        out.say("Scanning (a few seconds) ...");
        let opts = ScanOptions { timeout: Duration::from_secs(4), sweep: true };
        let inv = scan_stream(&opts, Inventory::default(), |_| {}, |_| {});
        let path = inventory_path();
        let mut merged = Inventory::load(&path);
        for d in &inv.devices {
            merged.merge(d.clone());
        }
        let _ = merged.save(&path);
        inv.devices.into_iter().filter(|d| d.synology || d.synofurb_debian).collect()
    }

    fn inspect(&mut self, t: &SshTarget) -> Result<(DeviceFacts, String)> {
        inspect_live(t, &super::runner::console_logger(&Out::plain()))
    }

    fn install(
        &mut self,
        out: &Out,
        job: InstallJob,
        mat: &super::spec::Material,
        method: &str,
        save: Option<&std::path::Path>,
    ) -> CliResult<()> {
        execute(out, job, mat, method, save)
    }

    fn recover(&mut self, out: &Out, t: SshTarget) -> CliResult<()> {
        boot::recover_target(out, t)
    }

    fn boot_manager(&mut self, out: &Out, t: SshTarget, agent_hostname: String, arm: bool) -> CliResult<()> {
        boot::boot_manager_install_target(out, t, None, super::facts::config_dir().join("dom-backups"), agent_hostname, arm)
    }
}

pub const NEEDS_TERMINAL: &str = "the wizard needs an interactive terminal (stdin and stderr must be terminals, and --non-interactive/CI must not be set). \
Without a terminal use the commands: `scan`, `inspect --host H`, `plan --host H`, `install --host H --dry-run`, then `install --host H ... --have-backup --accept-data-loss --confirm \"ERASE <MODEL>\"`. See docs/CLI.md";

pub fn run_wizard(out: &Out, a: WizardArgs) -> CliResult<()> {
    if !out.interactive {
        return Err(CliError::usage(NEEDS_TERMINAL));
    }
    let rehearsal = if a.demo {
        let (facts, raw) = demo_facts();
        Some(Inspection { facts, raw, source: Source::Demo })
    } else if let Some(f) = &a.facts {
        let text = std::fs::read_to_string(f).with_context(|| format!("reading {f}"))?;
        let (facts, raw) = parse_facts_text(&text).with_context(|| f.clone())?;
        Some(Inspection { facts, raw, source: Source::File(f.clone()) })
    } else {
        None
    };
    wizard(out, &mut StdPrompter, &mut RealBackend, rehearsal, a.host)
}

fn heading(p: &mut dyn Prompter, n: u32, title: &str) {
    p.note("");
    p.note(&format!("--- Step {n} of 9: {title} ---"));
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Goal {
    Install,
    BootManager,
    Recover,
    Quit,
}

/// The whole flow. `rehearsal`: facts to use instead of connecting; the end is then a dry run.
pub fn wizard(
    out: &Out,
    p: &mut dyn Prompter,
    be: &mut dyn Backend,
    rehearsal: Option<Inspection>,
    host_hint: Option<String>,
) -> CliResult<()> {
    p.note("synofurb wizard. Nothing on the NAS is changed before step 7 (Confirm); Ctrl-C quits at any prompt.");
    let is_rehearsal = rehearsal.is_some();
    let insp = match rehearsal {
        Some(i) => {
            heading(p, 1, "Find the NAS");
            p.note(&format!("Rehearsal: using the {}. No network connection is made and nothing can be changed.", i.describe()));
            heading(p, 2, "Connect");
            p.note("(skipped in a rehearsal)");
            i
        }
        None => connect_and_inspect(out, p, be, host_hint)?,
    };

    // ---- 3 inspect
    heading(p, 3, "Inspect");
    let db = load_compat_db();
    let asm = assess(&db, &insp.facts);
    for l in render::hardware_lines(&insp.facts, &asm) {
        p.note(&l);
    }
    p.note("");
    for l in render::storage_lines(&insp.facts) {
        p.note(&l);
    }
    if let Some(why) = asm.blocking() {
        p.note("");
        p.note(&format!("This unit cannot be converted: {why}"));
        return Err(CliError::Unsupported(why));
    }

    // ---- 4 goal
    heading(p, 4, "Goal");
    let goal = choose_goal(p, &insp.facts)?;
    match goal {
        Goal::Quit => {
            p.note("Nothing was changed.");
            return Ok(());
        }
        Goal::Recover => return recover_flow(out, p, be, &insp, is_rehearsal),
        Goal::BootManager => return boot_manager_flow(out, p, be, &insp, is_rehearsal),
        Goal::Install => {}
    }

    // ---- 5 storage
    heading(p, 5, "Storage");
    let (_req, plan) = choose_storage(p, &insp.facts)?;

    // ---- 6 system
    heading(p, 6, "System");
    let (spec, method_name) = choose_system(p, &insp.facts, &asm)?;

    // ---- 7 confirm
    heading(p, 7, "Confirm");
    summary(p, &insp, &plan, &spec, &method_name);
    let address = match &insp.source {
        Source::Live(t) => t.host.clone(),
        _ => "(rehearsal: no address)".into(),
    };
    gate::enforce(&GateOpts { have_backup: false, accept_data_loss: false, confirm: None }, &insp.facts, &address, out, p)?;

    // ---- 8 install (or the dry run)
    heading(p, 8, if is_rehearsal { "Install (rehearsal: render only)" } else { "Install" });
    if is_rehearsal {
        let mat = build_material(&spec, &insp.facts, true, out)?;
        let rendered = dryrun::render(&mat.cfg, &plan, &insp.facts, &insp.raw, dryrun::bind_ip_for(None), spec.bundle.as_deref())?;
        let dir = p.ask(
            "Write the generated files (stage-2 script, payload, plan, preseed) to a folder? Folder name (Enter = print them here)",
            Some(""),
        )?;
        let lines = render::plan_lines(&insp.facts, &plan);
        let dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
        dryrun::deliver(
            out,
            dryrun::Delivery { json_doc: false, dir: dir.as_deref() },
            &insp.facts,
            &plan,
            &lines,
            &method_name,
            &rendered,
        )?;
        heading(p, 9, "Done");
        p.note("Rehearsal finished. Nothing was contacted and nothing was changed.");
        return Ok(());
    }
    let save = p.ask("Save the logins to a file when the install is done? File name (Enter = no, they are shown on screen)", Some(""))?;
    let save = (!save.is_empty()).then(|| PathBuf::from(save));
    let target = insp.target().cloned().ok_or_else(|| CliError::usage("an install needs a connection to the NAS"))?;
    let (_, method) = resolve_method(&spec, &insp.facts)?;
    let mat = build_material(&spec, &insp.facts, false, out)?;
    let job = build_job(&spec, &insp, plan, target, &mat, method);
    p.note("Starting. Ctrl-C cancels until the point of no return; after it nothing can be cancelled.");
    be.install(out, job, &mat, &method_name, save.as_deref())?;
    heading(p, 9, "Done");
    p.note("The new system is up. The logins are above; keep them and the SSH key somewhere safe.");
    Ok(())
}

// ---------------------------------------------------------------------------------------------- steps 1-2

fn connect_and_inspect(out: &Out, p: &mut dyn Prompter, be: &mut dyn Backend, host_hint: Option<String>) -> CliResult<Inspection> {
    let mut host = host_hint;
    loop {
        heading(p, 1, "Find the NAS");
        let host_now = match host.take() {
            Some(h) => h,
            None => pick_host(out, p, be)?,
        };
        heading(p, 2, "Connect");
        let target = ask_target(out, p, &host_now)?;
        heading(p, 3, "Inspect");
        p.note(&format!("Connecting to {}@{}:{} and reading (only reading) ...", target.user, target.host, target.port));
        match be.inspect(&target) {
            Ok((facts, raw)) => return Ok(Inspection { facts, raw, source: Source::Live(target) }),
            Err(e) => {
                p.note(&format!("Could not inspect the NAS: {e:#}"));
                if !p.confirm("Try again with other details?", true)? {
                    return Err(CliError::Other(e));
                }
            }
        }
    }
}

fn pick_host(out: &Out, p: &mut dyn Prompter, be: &mut dyn Backend) -> CliResult<String> {
    let mut found = be.known();
    loop {
        let mut items: Vec<String> = vec!["Scan the network now".into(), "Type an address".into()];
        items.extend(found.iter().map(device_line));
        let c = p.choose("Which NAS?", &items, if found.is_empty() { 0 } else { 2 })?;
        match c {
            0 => {
                found = be.scan(out);
                if found.is_empty() {
                    p.note("No Synology unit answered. You can scan again or type an address.");
                } else {
                    p.note(&format!("{} unit(s) found.", found.len()));
                }
            }
            1 => {
                let a = p.ask("Address or name of the NAS", None)?;
                if a.is_empty() {
                    p.note("An address is needed.");
                    continue;
                }
                return Ok(a);
            }
            n => {
                let d = &found[n - 2];
                match d.primary_ip() {
                    Some(ip) => return Ok(ip.to_string()),
                    None => p.note("That entry has no address; pick another."),
                }
            }
        }
    }
}

fn ask_target(out: &Out, p: &mut dyn Prompter, host: &str) -> CliResult<SshTarget> {
    let user = p.ask("SSH user (a DSM administrator)", Some("admin"))?;
    let port: u16 = loop {
        let t = p.ask("SSH port", Some("22"))?;
        match t.parse() {
            Ok(v) => break v,
            Err(_) => p.note("  not a port number"),
        }
    };
    // first contact: show the unit's host key and ask, before a password is typed
    super::hostkey::install_policy(host, port, &super::args::HostKeyOpts::default(), out, p)?;
    let by_key = p.choose("Log in with", &["Password".to_string(), "Private key file".to_string()], 0)? == 1;
    if by_key {
        let path = p.ask("Path of the private key (OpenSSH format, no passphrase)", None)?;
        let pem = std::fs::read_to_string(&path).with_context(|| format!("reading the key {path}"))?;
        let sudo = if user == "root" {
            None
        } else {
            Some(p.secret(&format!("sudo password for {user} (Enter if none)"))?).filter(|s| !s.is_empty())
        };
        return Ok(SshTarget { host: host.to_string(), port, user, auth: Auth::Key(pem), sudo_password: sudo });
    }
    let pw = match std::env::var("SYNOFURB_PASSWORD").ok().filter(|s| !s.is_empty()) {
        Some(v) => {
            p.note("Using the password from $SYNOFURB_PASSWORD.");
            v
        }
        None => p.secret(&format!("SSH password for {user}@{host}"))?,
    };
    let sudo = if user == "root" || p.confirm("Is the sudo password the same as the login password?", true)? {
        pw.clone()
    } else {
        p.secret("sudo password")?
    };
    Ok(SshTarget { host: host.to_string(), port, user, auth: Auth::Password(pw), sudo_password: Some(sudo).filter(|s| !s.is_empty()) })
}

// ---------------------------------------------------------------------------------------------- step 4

fn choose_goal(p: &mut dyn Prompter, facts: &DeviceFacts) -> CliResult<Goal> {
    let dom_ok = method_available("dom", facts).is_ok();
    let items = vec![
        "Install Debian 13 (reinstall this unit; destroys data, asks for confirmation)".to_string(),
        format!(
            "Boot manager (add an SSH-controlled boot environment to the boot flash; DSM stays default){}",
            if dom_ok { "" } else { "  [not available on this unit]" }
        ),
        "Recover (make the unit boot the original DSM again)".to_string(),
        "Quit without changing anything".to_string(),
    ];
    Ok(match p.choose("What do you want to do?", &items, 0)? {
        0 => Goal::Install,
        1 => Goal::BootManager,
        2 => Goal::Recover,
        _ => Goal::Quit,
    })
}

fn recover_flow(out: &Out, p: &mut dyn Prompter, be: &mut dyn Backend, insp: &Inspection, rehearsal: bool) -> CliResult<()> {
    if rehearsal {
        p.note("Rehearsal: recovery would set the boot flash default to DSM. Nothing was contacted.");
        return Ok(());
    }
    if !p.confirm("Make the unit boot DSM again at its next reboot?", true)? {
        return Err(CliError::refused("declined; nothing was changed."));
    }
    let t = insp.target().cloned().ok_or_else(|| CliError::usage("recovery needs a connection to the NAS"))?;
    be.recover(out, t)
}

fn boot_manager_flow(out: &Out, p: &mut dyn Prompter, be: &mut dyn Backend, insp: &Inspection, rehearsal: bool) -> CliResult<()> {
    if let Err(why) = method_available("dom", &insp.facts) {
        p.note(why);
        p.note("The boot manager needs an x86 unit with an editable internal boot flash. Use the installer kit for other units.");
        return Err(CliError::usage(why));
    }
    for l in render::wrap(
        "The boot manager first backs up the unit's whole internal boot flash to this computer (verified against the NAS), then adds a small boot agent entry. DSM stays the default boot entry unless you arm the agent for the next boot.",
        90,
        2,
    ) {
        p.note(&l);
    }
    let name = p.ask("Host name of the agent on the network", Some("synofurb-agent"))?;
    synofurb_core::config::validate_hostname(&name).map_err(|e| CliError::usage(e.to_string()))?;
    let arm = p.confirm("Boot into the agent at the next reboot (it returns to DSM by itself if nobody claims it)?", false)?;
    if rehearsal {
        p.note("Rehearsal: nothing was contacted.");
        return Ok(());
    }
    if !p.confirm("Back up the boot flash and install the agent now?", false)? {
        return Err(CliError::refused("declined; nothing was changed."));
    }
    let t = insp.target().cloned().ok_or_else(|| CliError::usage("the boot manager needs a connection to the NAS"))?;
    be.boot_manager(out, t, name, arm)
}

// ---------------------------------------------------------------------------------------------- step 5

fn parse_gib(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() { Some(0) } else { t.parse().ok() }
}

fn choose_storage(p: &mut dyn Prompter, facts: &DeviceFacts) -> CliResult<(PlanRequest, Plan)> {
    let arrays: Vec<String> = facts.data_arrays().iter().map(|a| a.name.clone()).collect();
    let disks = facts.candidate_disks().len().max(1);
    let mut req = PlanRequest::default();
    loop {
        if arrays.is_empty() {
            p.note("This unit has no data volumes to keep: every disk is used for the new system and a new data volume.");
            req.mode = StorageMode::WipeAll;
            req.keep_arrays = None;
        } else {
            let items = vec![
                "Wipe everything: destroy all data on every disk and start clean".to_string(),
                format!(
                    "Keep data volumes: reclaim only DSM's system and swap, keep {} (no guarantee, see the warnings)",
                    arrays.join(", ")
                ),
            ];
            let c =
                p.choose("What should happen to the data on the disks?", &items, if req.mode == StorageMode::KeepVolumes { 1 } else { 0 })?;
            if c == 1 {
                req.mode = StorageMode::KeepVolumes;
                loop {
                    let a = p.ask(
                        &format!("Data arrays to keep, comma separated (available: {})", arrays.join(", ")),
                        Some(&arrays.join(",")),
                    )?;
                    let list: Vec<String> = a.split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect();
                    if !list.is_empty() && list.iter().all(|x| arrays.contains(x)) {
                        req.keep_arrays = Some(list);
                        break;
                    }
                    p.note("  name one or more of the available arrays");
                }
            } else {
                req.mode = StorageMode::WipeAll;
                req.keep_arrays = None;
            }
        }
        let probe = build_plan(facts, &req);
        let (min, rec, max) =
            (probe.sizes.min_system_mib / 1024, probe.sizes.recommended_system_mib / 1024, probe.sizes.max_system_mib / 1024);
        req.system_size_mib = loop {
            let t = p.ask(&format!("System size in GiB (recommended {rec}, min {min}, max {max}; Enter = recommended)"), Some(""))?;
            match parse_gib(&t) {
                Some(g) => break g * 1024,
                None => p.note("  a whole number of GiB, or Enter"),
            }
        };
        req.system_copies = loop {
            let def = default_system_copies(disks, disks);
            let t = p.ask(&format!("Disks that carry a copy of the system (1-{disks}; Enter = automatic, now {def})"), Some(""))?;
            if t.is_empty() {
                break None;
            }
            match t.parse::<usize>() {
                Ok(n) if (1..=disks).contains(&n) => break Some(n),
                _ => p.note("  a number within the range, or Enter"),
            }
        };
        let raids: Vec<String> = DataRaid::ALL.iter().map(|r| r.label().to_string()).collect();
        let cur = DataRaid::ALL.iter().position(|r| *r == req.data_raid).unwrap_or(0);
        req.data_raid = DataRaid::ALL[p.choose("RAID level of the new data volume", &raids, cur)?];
        req.data_free_percent = loop {
            let t = p.ask(
                "Percent of the data volume group left free for later volumes and snapshots (0-90)",
                Some(&req.data_free_percent.to_string()),
            )?;
            match t.parse::<u8>() {
                Ok(n) if n <= 90 => break n,
                _ => p.note("  0 to 90"),
            }
        };
        let plan = build_plan(facts, &req);
        p.note("");
        for l in render::plan_lines(facts, &plan) {
            p.note(&l);
        }
        if !plan.is_ok() {
            p.note("");
            p.note("This plan has errors; change the settings above.");
            continue;
        }
        if p.confirm("Use this plan?", true)? {
            return Ok((req, plan));
        }
    }
}

// ---------------------------------------------------------------------------------------------- step 6

fn choose_system(p: &mut dyn Prompter, facts: &DeviceFacts, asm: &Assessment<'_>) -> CliResult<(InstallSpec, String)> {
    let _ = asm;
    let mut spec = InstallSpec::defaults();
    let detected = facts.serial_console();
    loop {
        spec.hostname = p.ask("Host name of the new system", Some(&spec.hostname))?;
        spec.username = p.ask("Name of the new administrator account", Some(&spec.username))?;
        let pw =
            p.choose("Password for that account", &["Generate a strong one (shown at the end)".to_string(), "Type my own".to_string()], 0)?;
        spec.password = if pw == 0 {
            PasswordChoice::Generate
        } else {
            loop {
                let a = p.secret("Password")?;
                let b = p.secret("Repeat the password")?;
                if a.is_empty() {
                    p.note("  the password cannot be empty");
                } else if a != b {
                    p.note("  the two passwords differ");
                } else {
                    break PasswordChoice::Given(a);
                }
            }
        };
        let k = p.choose(
            "SSH key for that account",
            &[
                "Generate one (saved on this computer)".to_string(),
                "Use a public key file".to_string(),
                "No key (password login only)".to_string(),
            ],
            0,
        )?;
        spec.key = match k {
            0 => KeyChoice::Generate,
            1 => loop {
                let path = p.ask("Path of the PUBLIC key file", None)?;
                match std::fs::read_to_string(&path) {
                    Ok(t) => break KeyChoice::Line(t.trim().to_string()),
                    Err(e) => p.note(&format!("  cannot read {path}: {e}")),
                }
            },
            _ => KeyChoice::None,
        };
        spec.timezone = p.ask("Time zone (for example Europe/Stockholm)", Some(&spec.timezone))?;
        let kern = p.choose(
            "Linux kernel",
            &[KernelChannel::Newest.label().to_string(), KernelChannel::Stable.label().to_string()],
            if spec.kernel == KernelChannel::Newest { 0 } else { 1 },
        )?;
        spec.kernel = if kern == 0 { KernelChannel::Newest } else { KernelChannel::Stable };
        spec.mgmt = p.confirm("Install the web management plane (https://<address>:8443)?", spec.mgmt)?;

        // boot method
        let auto = resolve_auto_method(facts);
        let methods: Vec<(MethodArg, String)> = [
            (MethodArg::Auto, format!("Automatic (recommended): {auto}")),
            (MethodArg::Dom, "Through the internal boot flash".to_string()),
            (MethodArg::Kexec, "kexec over SSH".to_string()),
            (MethodArg::Kit, "Installer kit (USB / TFTP / U-Boot)".to_string()),
        ]
        .into_iter()
        .map(|(m, label)| {
            let name = match m {
                MethodArg::Auto => "auto",
                MethodArg::Dom => "dom",
                MethodArg::Kexec => "kexec",
                MethodArg::Kit => "kit",
            };
            match method_available(name, facts) {
                Ok(()) => (m, label),
                Err(why) => (m, format!("{label}  [unavailable: {why}]")),
            }
        })
        .collect();
        let labels: Vec<String> = methods.iter().map(|m| m.1.clone()).collect();
        loop {
            let i = p.choose("How should the installer be started?", &labels, 0)?;
            spec.method = methods[i].0;
            let name = match spec.method {
                MethodArg::Auto => auto,
                MethodArg::Dom => "dom",
                MethodArg::Kexec => "kexec",
                MethodArg::Kit => "kit",
            };
            match method_available(name, facts) {
                Ok(()) => break,
                Err(why) => p.note(&format!("  {why} Pick another method.")),
            }
        }
        if resolve_auto_method(facts) == "kit" && spec.method != MethodArg::Dom && spec.method != MethodArg::Kexec
            || spec.method == MethodArg::Kit
        {
            spec.kit_dir = PathBuf::from(p.ask("Folder for the installer kit", Some(&spec.kit_dir.display().to_string()))?);
        }

        // serial console
        let cur = serial_text(&detected);
        let s = p.ask(&format!("Serial console (detected from DSM: {}; Enter keeps it)", detected.kernel_arg()), Some(&cur))?;
        spec.serial =
            if s == cur { None } else { Some(parse_serial(&s, Some(detected.speed)).map_err(|e| CliError::usage(e.to_string()))?) };
        if facts.arch.is_arm() {
            let def = synofurb_core::platform::mainline_dtb(&facts.model)
                .or_else(|| synofurb_core::platform::mainline_dtb(&facts.unique))
                .unwrap_or("");
            let d =
                p.ask("Board device tree file for this ARM unit (for example armada-385-synology-ds116.dtb; empty = none)", Some(def))?;
            spec.dtb = Some(d).filter(|d| !d.is_empty());
        }
        let pk = p.ask("Extra Debian packages, comma separated (Enter = none)", Some(""))?;
        spec.packages = pk.split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect();
        spec.firmware = p.confirm("Include non-free firmware packages?", spec.firmware)?;
        spec.verbose_installer = p.confirm("Verbose installer debug logging (recommended)?", spec.verbose_installer)?;

        if p.confirm("Use an offline bundle instead of downloading from the Internet?", false)? {
            loop {
                let d = p.ask("Bundle directory (made by `bundle create`)", None)?;
                let dir = PathBuf::from(&d);
                match super::bundle::open_bundle(&dir) {
                    Ok(_) => {
                        spec.bundle = Some(dir);
                        break;
                    }
                    Err(e) => p.note(&format!("  {e}")),
                }
            }
        } else {
            spec.bundle = None;
        }
        if p.confirm("Change the language, keyboard layout or Debian mirror?", false)? {
            spec.locale = p.ask("Locale", Some(&spec.locale))?;
            spec.keymap = p.ask("Keyboard layout", Some(&spec.keymap))?;
            spec.mirror_host = p.ask("Debian mirror host", Some(&spec.mirror_host))?;
        }

        // validate everything now, not after the confirmation
        match build_material(&spec, facts, true, &Out::plain()) {
            Ok(_) => {}
            Err(e) => {
                p.note(&format!("Some of these settings are not valid: {e}. Let us go through them again."));
                continue;
            }
        }
        if let Some(dir) = &spec.bundle {
            match super::bundle::check_for_install(
                dir,
                facts.arch,
                &super::spec::default_suite(),
                spec.kernel,
                &spec.packages,
                spec.firmware,
            ) {
                Ok(warnings) => warnings.iter().for_each(|w| p.note(&format!("  warning: {w}"))),
                Err(e) => {
                    p.note(&format!(
                        "{e}. Let us go through the settings again (choose the same kernel and packages as the bundle, or no bundle)."
                    ));
                    continue;
                }
            }
        }
        match resolve_method(&spec, facts) {
            Ok((name, _)) => return Ok((spec, name.to_string())),
            Err(e) => {
                p.note(&format!("{e}. Let us go through the settings again."));
                continue;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- step 7

fn summary(p: &mut dyn Prompter, insp: &Inspection, plan: &Plan, spec: &InstallSpec, method: &str) {
    let f = &insp.facts;
    p.note(&format!("Unit:        {} (serial {}), {}", f.model, if f.serial.is_empty() { "unknown" } else { &f.serial }, insp.describe()));
    p.note(&format!(
        "New system:  Debian 13 {}, host name {}, user {}, kernel {:?}",
        f.arch.debian(),
        spec.hostname,
        spec.username,
        spec.kernel
    ));
    p.note(&format!("Started by:  {method}{}", if spec.bundle.is_some() { ", offline bundle" } else { "" }));
    p.note("");
    for l in plan.summary() {
        p.note(&format!("  {l}"));
    }
    if !plan.destroyed.is_empty() {
        p.note("");
        p.note("WILL BE DESTROYED:");
        for d in &plan.destroyed {
            p.note(&format!("  - {d}"));
        }
    }
    if !plan.preserved.is_empty() {
        p.note("");
        p.note("Will be preserved (no guarantee):");
        for d in &plan.preserved {
            p.note(&format!("  - {}: {}", d.what, d.detail));
        }
    }
    p.note("");
    p.note("If something goes wrong:");
    for (i, l) in render::fallback_lines(f.firmware).iter().enumerate() {
        for (j, w) in render::wrap(l, 92, 5).iter().enumerate() {
            p.note(&if j == 0 { format!("  {}. {}", i + 1, w.trim_start()) } else { w.clone() });
        }
    }
    p.note("");
    p.note(&format!(
        "Everything on {} can be lost, including volumes marked as preserved. There is no guarantee.",
        confirm_phrase(f).trim_start_matches("ERASE ")
    ));
}

#[cfg(test)]
mod tests {
    use super::super::prompt::Responder;
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        inspected: Vec<String>,
        installs: RefCell<Vec<(String, String)>>,
        recovers: usize,
        boots: usize,
        fail_inspect_first: bool,
    }

    impl Backend for Fake {
        fn known(&mut self) -> Vec<Found> {
            vec![Found { ips: vec!["192.168.1.20".parse().unwrap()], model: "DS918+".into(), synology: true, ..Default::default() }]
        }
        fn scan(&mut self, _: &Out) -> Vec<Found> {
            self.known()
        }
        fn inspect(&mut self, t: &SshTarget) -> Result<(DeviceFacts, String)> {
            self.inspected.push(format!("{}@{}:{}", t.user, t.host, t.port));
            if self.fail_inspect_first && self.inspected.len() == 1 {
                anyhow::bail!("connection refused");
            }
            Ok(demo_facts())
        }
        fn install(
            &mut self,
            _: &Out,
            job: InstallJob,
            mat: &super::super::spec::Material,
            method: &str,
            _: Option<&std::path::Path>,
        ) -> CliResult<()> {
            self.installs.borrow_mut().push((job.cfg.hostname.clone(), method.to_string()));
            assert!(
                mat.cfg.password_hash.starts_with("$6$") && !mat.cfg.password_hash.contains("DRYRUN"),
                "a real install gets the real hash"
            );
            Ok(())
        }
        fn recover(&mut self, _: &Out, _: SshTarget) -> CliResult<()> {
            self.recovers += 1;
            Ok(())
        }
        fn boot_manager(&mut self, _: &Out, _: SshTarget, _: String, _: bool) -> CliResult<()> {
            self.boots += 1;
            Ok(())
        }
    }

    fn tty() -> Out {
        let mut o = Out::plain();
        o.interactive = true;
        o
    }

    fn live_answers() -> Vec<(&'static str, &'static str)> {
        vec![("Which NAS?", "3"), ("SSH password", "pw"), ("Is the sudo password the same", "y")]
    }

    #[test]
    fn the_wizard_refuses_without_a_terminal() {
        let e = run_wizard(&Out::plain(), WizardArgs { demo: false, facts: None, host: None }).err().unwrap();
        assert_eq!(e.code(), 2);
        assert!(e.to_string().contains("interactive terminal") && e.to_string().contains("install --host H --dry-run"));
    }

    #[test]
    fn a_wrong_confirmation_phrase_stops_before_anything_happens() {
        let mut be = Fake::default();
        let mut rules = live_answers();
        rules.extend([("Type ERASE", "ERASE DS923+"), ("complete, verified backup", "y"), ("Do you understand", "y")]);
        let mut p = Responder::new(rules);
        let e = wizard(&tty(), &mut p, &mut be, None, None).err().unwrap();
        assert_eq!(e.code(), 3, "{e}");
        assert!(be.installs.borrow().is_empty(), "the install must not start");
        assert_eq!(be.inspected, vec!["admin@192.168.1.20:22"]);
        assert!(p.said("Step 1 of 9") && p.said("Step 7 of 9") && !p.said("Step 8 of 9"));
        assert!(p.said("serial 1920PDN123456") && p.said("192.168.1.20"), "the confirmation shows the unit's address and serial");
    }

    #[test]
    fn declining_the_backup_question_refuses() {
        let mut be = Fake::default();
        let mut rules = live_answers();
        rules.extend([("complete, verified backup", "n")]);
        let e = wizard(&tty(), &mut Responder::new(rules), &mut be, None, None).err().unwrap();
        assert_eq!(e.code(), 3);
        assert!(be.installs.borrow().is_empty());
    }

    #[test]
    fn the_happy_path_runs_the_install_once_with_the_chosen_settings() {
        let mut be = Fake::default();
        let mut rules = live_answers();
        rules.extend([
            ("Host name of the new system", "vault"),
            ("Name of the new administrator", "andreas"),
            ("complete, verified backup", "y"),
            ("Do you understand", "y"),
            ("Type ERASE", "ERASE DS918+"),
            ("Extra Debian packages", "htop, tmux"),
        ]);
        let mut p = Responder::new(rules);
        wizard(&tty(), &mut p, &mut be, None, None).unwrap();
        assert_eq!(*be.installs.borrow(), vec![("vault".to_string(), "kexec".to_string())]);
        for step in 1..=9 {
            assert!(p.said(&format!("Step {step} of 9")), "step {step}");
        }
        assert!(p.said("WILL BE DESTROYED") && p.said("If something goes wrong"));
    }

    #[test]
    fn picking_a_scanned_unit_and_a_failed_connection_can_be_retried() {
        let mut be = Fake { fail_inspect_first: true, ..Default::default() };
        let mut rules = live_answers();
        rules.extend([("Try again", "y"), ("Goal", "")]);
        rules.extend([("What do you want to do?", "4")]);
        let mut p = Responder::new(rules);
        wizard(&tty(), &mut p, &mut be, None, None).unwrap();
        assert_eq!(be.inspected.len(), 2, "the second attempt goes through");
        assert!(p.said("Could not inspect the NAS: connection refused"));
        assert!(be.installs.borrow().is_empty());
    }

    #[test]
    fn recover_and_boot_manager_goals() {
        let mut be = Fake::default();
        let mut rules = live_answers();
        rules.extend([("What do you want to do?", "3"), ("Make the unit boot DSM again", "y")]);
        wizard(&tty(), &mut Responder::new(rules), &mut be, None, None).unwrap();
        assert_eq!(be.recovers, 1);
        // the demo DS918+ has no boot flash: the boot manager is refused politely
        let mut be = Fake::default();
        let mut rules = live_answers();
        rules.extend([("What do you want to do?", "2")]);
        let e = wizard(&tty(), &mut Responder::new(rules), &mut be, None, None).err().unwrap();
        assert_eq!(e.code(), 2);
        assert_eq!(be.boots, 0);
    }

    #[test]
    fn rehearsal_goes_all_the_way_without_a_backend() {
        let mut be = Fake::default();
        let (facts, raw) = demo_facts();
        let insp = Inspection { facts, raw, source: Source::Demo };
        let dir = std::env::temp_dir().join(format!("sf-wiz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rules = vec![
            ("complete, verified backup", "y"),
            ("Do you understand", "y"),
            ("Type ERASE", "ERASE DS918+"),
            ("Write the generated files", dir.to_str().unwrap()),
        ];
        // rules need 'static strs; leak the path string for the test
        let rules: Vec<(&'static str, &'static str)> =
            rules.into_iter().map(|(a, b)| (a, &*Box::leak(b.to_string().into_boxed_str()))).collect();
        let mut p = Responder::new(rules);
        wizard(&tty(), &mut p, &mut be, Some(insp), None).unwrap();
        assert!(be.inspected.is_empty() && be.installs.borrow().is_empty());
        assert!(dir.join("stage2.sh").is_file() && dir.join("preseed.cfg").is_file() && dir.join("plan.json").is_file());
        assert!(p.said("Rehearsal finished"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_wizard_can_use_an_offline_bundle_and_renders_the_offline_mirror() {
        use synofurb_core::offline::{ArchManifest, BundleManifest, FORMAT_VERSION, MANIFEST_FILE};
        let root = std::env::temp_dir().join(format!("sf-wiz-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bundle = root.join("bundle");
        std::fs::create_dir_all(bundle.join("mirror/debian/dists/trixie")).unwrap();
        let mut m = BundleManifest {
            format: FORMAT_VERSION,
            tool_version: "t".into(),
            created: "2026-11-01T00:00:00+00:00".into(),
            suite: "trixie".into(),
            suites: vec!["trixie".into()],
            kernel: KernelChannel::Newest,
            with_firmware: true,
            extra_packages: vec![],
            release_dates: Default::default(),
            pgp_verified: false,
            warnings: vec![],
            archs: Default::default(),
            files: vec![],
        };
        m.archs.insert("amd64".into(), ArchManifest::default());
        std::fs::write(bundle.join(MANIFEST_FILE), serde_json::to_string(&m).unwrap()).unwrap();
        let review = root.join("review");
        let leak = |s: &std::path::Path| -> &'static str { Box::leak(s.to_str().unwrap().to_string().into_boxed_str()) };
        let mut p = Responder::new([
            ("Use an offline bundle", "y"),
            ("Bundle directory", leak(&bundle)),
            ("complete, verified backup", "y"),
            ("Do you understand", "y"),
            ("Type ERASE", "ERASE DS918+"),
            ("Write the generated files", leak(&review)),
        ]);
        let (facts, raw) = demo_facts();
        wizard(&tty(), &mut p, &mut Fake::default(), Some(Inspection { facts, raw, source: Source::Demo }), None).unwrap();
        assert!(p.said("offline bundle"), "the summary names the bundle");
        let preseed = std::fs::read_to_string(review.join("preseed.cfg")).unwrap();
        assert!(preseed.contains("/DRYRUN/mirror/debian"), "{preseed}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn storage_choices_reach_the_plan() {
        let (facts, _) = demo_facts();
        let mut p = Responder::new([
            ("What should happen to the data", "2"),
            ("Data arrays to keep", "md2"),
            ("System size", "9"),
            ("Disks that carry a copy", "2"),
            ("RAID level", "4"),
            ("Percent of the data volume group", "10"),
        ]);
        let (req, plan) = choose_storage(&mut p, &facts).unwrap();
        assert_eq!(req.mode, StorageMode::KeepVolumes);
        assert_eq!(req.keep_arrays, Some(vec!["md2".to_string()]));
        assert_eq!((req.system_size_mib, req.system_copies, req.data_free_percent), (9 * 1024, Some(2), 10));
        assert_eq!(req.data_raid, DataRaid::Raid1);
        assert_eq!(plan.kept_arrays, vec!["md2"]);
    }

    #[test]
    fn invalid_system_settings_are_asked_again() {
        let (facts, _) = demo_facts();
        let db = load_compat_db();
        let asm = assess(&db, &facts);
        struct Flip(Responder, usize);
        impl Prompter for Flip {
            fn ask(&mut self, q: &str, d: Option<&str>) -> Result<String> {
                if q.starts_with("Host name of the new system") {
                    self.1 += 1;
                    return Ok(if self.1 == 1 { "bad host name".into() } else { "good-host".into() });
                }
                self.0.ask(q, d)
            }
            fn secret(&mut self, q: &str) -> Result<String> {
                self.0.secret(q)
            }
            fn confirm(&mut self, q: &str, d: bool) -> Result<bool> {
                self.0.confirm(q, d)
            }
            fn choose(&mut self, q: &str, i: &[String], d: usize) -> Result<usize> {
                self.0.choose(q, i, d)
            }
            fn note(&mut self, t: &str) {
                self.0.note(t)
            }
        }
        let mut p = Flip(Responder::new([]), 0);
        let (spec, method) = choose_system(&mut p, &facts, &asm).unwrap();
        assert_eq!(spec.hostname, "good-host");
        assert_eq!(method, "kexec");
        assert!(p.0.said("not valid"));
    }
}
