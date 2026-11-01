//! `recover`, `boot-manager`, `backup-dom`, `disk-backup` and `fetch-installer`: the commands that act on one unit or one download
//! without being a full install.

use super::args::{ArchArg, BackupDomArgs, BootManagerCmd, Conn, DiskBackupArgs, FetchArgs, RecoverArgs, RecoverCmd};
use super::exit::{CliError, CliResult};
use super::facts::{SCHEMA, config_dir, conn_target, target_secrets};
use super::out::Out;
use super::prompt::{Prompter, StdPrompter};
use super::runner::{console_logger, progress_fn, run_job};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::path::PathBuf;
use synofurb_core::logs::Logger;
use synofurb_core::model::*;
use synofurb_core::recovery::{self, ProbeOptions, ProbeResult, ReadOutcome, UnitBootState};
use synofurb_core::remote::{Privileged, SshSession};
use synofurb_core::session::{self, SshTarget, config_keys_dir, inspect_remote, recover_to_dsm};

fn unit_name(facts: &DeviceFacts, t: &SshTarget) -> String {
    if facts.serial.is_empty() { t.host.clone() } else { facts.serial.clone() }
}

/// Connect, inspect read-only, hand the open session to `f`, disconnect.
fn with_unit<R>(t: &SshTarget, lg: &Logger, f: impl FnOnce(&mut SshSession, &DeviceFacts) -> Result<R>) -> Result<R> {
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, lg)?;
    let r = inspect_remote(&mut ssh, false, t.sudo_password.clone(), lg).and_then(|(facts, _)| f(&mut ssh, &facts));
    ssh.close();
    r
}

pub const RECOVERY_NOTE: &str = "The default boot entry is DSM now. The unit was not rebooted: it keeps running what it runs now. After the next reboot it starts the DSM loader. If DSM's system partition is gone (a failed install) the loader offers a reinstall: open https://find.synology.com and reinstall DSM with the .pat file for your model; synofurb cannot promise that data volumes survive. `recover status` shows what the boot flash says; `recover boot-dsm --reboot --yes` followed by `recover wait` reboots and watches for DSM.";

pub fn recover(out: &Out, a: RecoverArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    match a.action {
        None => {
            let host = a
                .host
                .ok_or_else(|| CliError::usage("recover needs --host ADDRESS, or a subcommand: status, wait, boot-dsm, restore, remove"))?;
            let t = conn_target(&Conn { host, ssh: a.ssh }, out, &mut prompt)?;
            recover_target(out, t)
        }
        Some(RecoverCmd::Status { conn, json }) => recover_status(out, conn, json, &mut prompt),
        Some(RecoverCmd::Wait { host, timeout, port, json }) => recover_wait(out, &host, timeout, port, json),
        Some(RecoverCmd::BootDsm { conn, reboot }) => {
            if reboot {
                confirm_action(
                    out,
                    &mut prompt,
                    "This sets the default boot entry to DSM and then REBOOTS the unit now (a running install or service on it is interrupted).",
                )?;
            }
            let t = conn_target(&conn, out, &mut prompt)?;
            out.event(json!({"event": "start", "command": "recover boot-dsm", "host": t.host, "reboot": reboot}));
            let secrets = target_secrets(&t);
            let rep = run_job(out, &secrets, move |lg, _| recovery::boot_dsm(&t, reboot, true, lg).map_err(anyhow::Error::new));
            if let Some(e) = rep.plain_failure() {
                return Err(e);
            }
            let o = rep.result.expect("checked");
            let mut lines = vec![o.message.clone()];
            if o.reboot_requested {
                lines.push("Next: `synofurb-cli recover wait --host <ip>` watches for DSM; it is back only when that says so.".into());
            } else {
                lines.push(RECOVERY_NOTE.into());
            }
            out.result_text(&lines);
            out.result_event("recover boot-dsm", true, json!({"default_is_dsm": o.default_is_dsm, "reboot_requested": o.reboot_requested, "state": o.state_after, "message": o.message}));
            Ok(())
        }
        Some(RecoverCmd::Restore { conn, backup }) => {
            confirm_action(
                out,
                &mut prompt,
                &format!(
                    "This OVERWRITES the unit's internal boot flash with {} (the current flash is snapshotted first, the write is read back).",
                    backup.display()
                ),
            )?;
            let t = conn_target(&conn, out, &mut prompt)?;
            out.event(json!({"event": "start", "command": "recover restore", "host": t.host}));
            let secrets = target_secrets(&t);
            let rep = run_job(out, &secrets, move |lg, _| recovery::restore_flash(&t, &backup, true, lg).map_err(anyhow::Error::new));
            if let Some(e) = rep.plain_failure() {
                return Err(e);
            }
            let o = rep.result.expect("checked");
            let mut lines = vec![
                format!("Boot flash restored from {} ({} bytes, sha256 {}).", o.backup.display(), o.bytes, o.sha256),
                if o.checked_against_sidecar {
                    "The image matched its .sha256 file before it was sent.".into()
                } else {
                    "There was no .sha256 file next to the image to compare with.".to_string()
                },
                "synofurb-dom read the flash back and compared it with the image.".into(),
            ];
            lines.extend(state_lines(&o.state_after));
            out.result_text(&lines);
            out.result_event(
                "recover restore",
                true,
                json!({"backup": o.backup.display().to_string(), "sha256": o.sha256, "bytes": o.bytes, "state": o.state_after}),
            );
            Ok(())
        }
        Some(RecoverCmd::Remove { conn }) => {
            confirm_action(
                out,
                &mut prompt,
                "This removes every boot entry and file synofurb added to the unit's boot flash and sets the default to DSM.",
            )?;
            let t = conn_target(&conn, out, &mut prompt)?;
            out.event(json!({"event": "start", "command": "recover remove", "host": t.host}));
            let secrets = target_secrets(&t);
            let rep = run_job(out, &secrets, move |lg, _| recovery::remove_our_entries(&t, true, lg).map_err(anyhow::Error::new));
            if let Some(e) = rep.plain_failure() {
                return Err(e);
            }
            let o = rep.result.expect("checked");
            let mut lines = vec!["synofurb's boot entries are gone and the default is DSM (read back from the unit).".to_string()];
            lines.extend(state_lines(&o.state_after));
            out.result_text(&lines);
            out.result_event("recover remove", true, json!({"state": o.state_after}));
            Ok(())
        }
    }
}

/// Text lines for a boot state (shared by `recover status` and the actions that report one).
pub fn state_lines(s: &UnitBootState) -> Vec<String> {
    let mut v = vec![];
    v.push(format!(
        "Running now:      {}",
        match s.running {
            recovery::RunningSystem::Dsm => "DSM",
            recovery::RunningSystem::Debian => "Debian",
            recovery::RunningSystem::Agent => "the boot agent",
            recovery::RunningSystem::Unknown => "not known",
        }
    ));
    v.push(format!(
        "Boot menu:        {}",
        match &s.read {
            ReadOutcome::Read => "read from the unit (synofurb-dom status)".to_string(),
            ReadOutcome::ToolMissing =>
                "NOT READ: synofurb-dom is not installed on the unit (synofurb never added the boot agent here, or the copy was removed)"
                    .to_string(),
            ReadOutcome::Refused(t) => format!("NOT READ: synofurb-dom refused: {t}"),
            ReadOutcome::Failed(t) => format!("NOT READ: {t}"),
        }
    ));
    v.push(format!("Default entry:    {}", s.default_entry.label()));
    if s.read == ReadOutcome::Read {
        v.push(format!("synofurb entries: {}", if s.our_entries.is_empty() { "none".to_string() } else { s.our_entries.join(", ") }));
        v.push(format!(
            "Agent files:      {}",
            match s.agent_present {
                Some(true) => "on the boot flash",
                Some(false) => "not on the boot flash",
                None => "not known",
            }
        ));
        v.push(format!(
            "Flash backup:     {}",
            match &s.flash_backup {
                Some(b) if b.exists => format!("on the unit: {}", b.path.as_deref().unwrap_or("(path not reported)")),
                Some(_) => "none on the unit".to_string(),
                None => "not reported".to_string(),
            }
        ));
    }
    if let Some(a) = &s.agent {
        v.push(format!(
            "Boot agent:       {} (watchdog {} min{}; default reset: {})",
            a.state,
            a.watchdog_timeout_min.map(|m| m.to_string()).unwrap_or_else(|| "?".into()),
            a.watchdog_remaining_s.map(|r| format!(", {} s left", r.max(0))).unwrap_or_default(),
            a.dom_default
        ));
    }
    for n in &s.notes {
        v.push(format!("Note:             {n}"));
    }
    v.push(match s.is_pristine() {
        Some(true) => "Verdict:          The boot flash is as DSM left it: the default is DSM and none of synofurb's entries are present.".to_string(),
        Some(false) => "Verdict:          Something of synofurb's is on the boot flash, or the default is not DSM. `recover boot-dsm` sets DSM as the default, `recover remove` takes our entries out.".to_string(),
        None => "Verdict:          NOT KNOWN. The boot menu could not be read, so nothing is claimed about the default. Do not assume the unit is in its original state (docs/RECOVERY.md).".to_string(),
    });
    v
}

fn recover_status(out: &Out, conn: Conn, json: bool, prompt: &mut dyn Prompter) -> CliResult<()> {
    let t = conn_target(&conn, out, prompt)?;
    let log = console_logger(out);
    let state = recovery::read_unit_boot_state(&t, &log).with_context(|| format!("asking {}", t.host))?;
    let local = recovery::find_local_backups(&config_dir().join("dom-backups"), "");
    let doc = json!({
        "schema": SCHEMA,
        "host": t.host,
        "pristine": state.is_pristine(),
        "state": state,
        "local_backups": local,
    });
    if json {
        out.json_doc(&doc);
    } else {
        let mut lines = vec![format!("Boot state of {}", t.host)];
        lines.extend(state_lines(&state).into_iter().map(|l| format!("  {l}")));
        if local.is_empty() {
            lines.push("  Saved on this computer: no boot flash image found in the data directory.".into());
        } else {
            lines.push("  Saved on this computer (newest first):".into());
            lines.extend(local.iter().take(5).map(|b| format!("    {}  ({} MiB)", b.path.display(), b.bytes / 1048576)));
        }
        out.data_lines(lines);
    }
    Ok(())
}

fn recover_wait(out: &Out, host: &str, minutes: u32, port: u16, json: bool) -> CliResult<()> {
    out.event(json!({"event": "start", "command": "recover wait", "host": host, "timeout_min": minutes}));
    let host_s = host.to_string();
    let rep = run_job(out, &[], move |lg, cancel| {
        let opts = ProbeOptions { ssh_port: port, ..Default::default() };
        let mut last = String::new();
        let w = recovery::wait_for_dsm(
            &host_s,
            std::time::Duration::from_secs(minutes as u64 * 60),
            &cancel,
            &opts,
            std::time::Duration::from_secs(5),
            &mut |r, t| {
                let line = probe_line(r);
                if line != last {
                    lg.info(format!("[{:>3}s] {line}", t.as_secs()));
                    last = line;
                }
            },
        );
        Ok(w)
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let w = rep.result.expect("checked");
    let line = probe_line(&w.last);
    let doc = json!({"dsm_back": w.last.is_dsm_back(), "last": w.last, "waited_s": w.waited_s, "timed_out": w.timed_out, "cancelled": w.cancelled});
    if json && !out.json_events() {
        out.json_doc(&doc);
    }
    if w.last.is_dsm_back() {
        out.result_text(&[format!("DSM answers at {host}: {line}")]);
        out.result_event("recover wait", true, doc);
        Ok(())
    } else {
        out.result_event("recover wait", false, doc);
        let why = if w.cancelled { "stopped by you".to_string() } else { format!("no DSM after {minutes} minutes") };
        Err(CliError::Other(anyhow!("DSM is not back ({why}). Last seen: {line}. {}", wait_advice(&w.last))))
    }
}

fn probe_line(r: &ProbeResult) -> String {
    match r {
        ProbeResult::DsmBack { ssh_banner, web_port } => format!("DSM is back (SSH {ssh_banner}, web port {web_port})"),
        ProbeResult::LoaderOnly { web_port } => {
            format!("only DSM's web port {web_port} answers, SSH does not (DSM still starting, or only the DSM loader runs)")
        }
        ProbeResult::AgentStillUp { ssh_banner } => {
            format!("the boot agent answers ({ssh_banner}); the unit is still in the agent, not in DSM")
        }
        ProbeResult::SshOnly { ssh_banner } => format!("SSH answers ({ssh_banner}) but DSM's web ports do not: some other system, not DSM"),
        ProbeResult::Unreachable => "nothing answers".to_string(),
    }
}

fn wait_advice(r: &ProbeResult) -> &'static str {
    match r {
        ProbeResult::LoaderOnly { .. } => {
            "If it stays like this, DSM's operating system may be gone: open https://find.synology.com to reinstall DSM (docs/RECOVERY.md)."
        }
        ProbeResult::AgentStillUp { .. } => {
            "Run `synofurb-cli recover boot-dsm --reboot --yes --host <ip>` (the agent has synofurb-dom), or see docs/RECOVERY.md."
        }
        _ => {
            "Do not rely on a power cycle if the agent may never have run: see docs/RECOVERY.md (SYNOLOGY_2 in the boot menu on a console)."
        }
    }
}

/// Make the unit boot DSM again (shared with the wizard).
pub fn recover_target(out: &Out, t: SshTarget) -> CliResult<()> {
    out.event(json!({"event": "start", "command": "recover", "host": t.host}));
    let secrets = target_secrets(&t);
    let rep = run_job(out, &secrets, move |lg, _| recover_to_dsm(&t, lg));
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let output = rep.result.unwrap_or_default();
    out.result_text(&[output.trim().to_string(), RECOVERY_NOTE.to_string()]);
    out.result_event("recover", true, json!({"output": output.trim(), "note": RECOVERY_NOTE}));
    Ok(())
}

/// Back up the boot flash and install the agent (shared with the wizard).
pub fn boot_manager_install_target(
    out: &Out,
    t: SshTarget,
    agent_dir: Option<PathBuf>,
    backup_dir: PathBuf,
    agent_hostname: String,
    arm: bool,
) -> CliResult<()> {
    out.event(json!({"event": "start", "command": "boot-manager install", "host": t.host}));
    let secrets = target_secrets(&t);
    let bd = backup_dir.clone();
    let rep = run_job(out, &secrets, move |lg, _| {
        let mut progress = progress_fn(lg);
        session::boot_manager_install(&t, agent_dir.as_deref(), &bd, &agent_hostname, arm, lg, &mut progress)
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    out.result_text(&[format!("Boot agent installed. Backup of the boot flash: {}", backup_dir.display())]);
    out.result_event("boot-manager install", true, json!({"backup_dir": backup_dir.display().to_string(), "armed": arm}));
    Ok(())
}

/// Ask before something that writes to the boot flash: `--yes`, or a question on a terminal; otherwise refuse.
pub(super) fn confirm_action(out: &Out, prompt: &mut dyn Prompter, question: &str) -> CliResult<()> {
    if out.yes {
        return Ok(());
    }
    if !out.interactive {
        return Err(CliError::refused(format!("{question} Pass --yes to confirm; nothing was changed.")));
    }
    if prompt.confirm(question, false)? { Ok(()) } else { Err(CliError::refused("declined; nothing was changed.")) }
}

pub fn boot_manager(out: &Out, action: BootManagerCmd) -> CliResult<()> {
    let mut prompt = StdPrompter;
    match action {
        BootManagerCmd::Install { conn, agent_dir, backup_dir, agent_hostname, arm } => {
            confirm_action(
                out,
                &mut prompt,
                "This backs up the unit's internal boot flash to this computer, then adds a boot agent entry to it (DSM stays the default unless --arm).",
            )?;
            synofurb_core::config::validate_hostname(&agent_hostname).map_err(|e| CliError::usage(e.to_string()))?;
            let t = conn_target(&conn, out, &mut prompt)?;
            let backup_dir = backup_dir.unwrap_or_else(|| config_dir().join("dom-backups"));
            boot_manager_install_target(out, t, agent_dir, backup_dir, agent_hostname, arm)
        }
        BootManagerCmd::Enter { conn } => {
            confirm_action(
                out,
                &mut prompt,
                "This reboots the unit into the boot agent. If the agent starts and nobody claims it, its watchdog sets DSM as the default again and reboots; if the agent never runs, the unit may keep booting it (docs/RECOVERY.md).",
            )?;
            let t = conn_target(&conn, out, &mut prompt)?;
            out.event(json!({"event": "start", "command": "boot-manager enter", "host": t.host}));
            let secrets = target_secrets(&t);
            let rep = run_job(out, &secrets, move |lg, cancel| recovery::enter_agent(&t, &cancel, true, lg));
            if let Some(e) = rep.plain_failure() {
                return Err(e);
            }
            let a = rep.result.expect("checked");
            let wd = synofurb_core::timing::BOOT_MANAGER_WATCHDOG_MIN;
            let how = format!(
                "Boot agent is running at {ip}. You control the bootloader over SSH:\n  {cmd}\n  sf-boot status | sf-boot kexec <kernel-url> <initrd-url> --cmdline '...' | sf-boot reboot dsm|debian\nIf nobody claims it, its watchdog sets DSM as the default again and reboots the unit {wd} minutes after the agent started.",
                ip = a.address,
                cmd = a.ssh_command,
            );
            out.result_text(&["".to_string(), how]);
            out.result_event(
                "boot-manager enter",
                true,
                json!({"agent_address": a.address.to_string(), "ssh_key": a.ssh_key.display().to_string(), "unit": a.unit, "watchdog_min": wd}),
            );
            Ok(())
        }
        BootManagerCmd::Status { conn, json } => {
            let t = conn_target(&conn, out, &mut prompt)?;
            let log = console_logger(out);
            let doc = with_unit(&t, &log, |ssh, facts| {
                let unit = unit_name(facts, &t);
                let key = config_keys_dir().join(format!(
                    "agent_{}_ed25519",
                    unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>()
                ));
                let mut p = Privileged { sh: ssh, is_root: facts.is_root, sudo_password: t.sudo_password.clone() };
                let r = p.run("synofurb-dom status --json 2>&1 || /usr/local/bin/synofurb-dom status --json 2>&1")?;
                Ok(status_doc(facts, &unit, key.is_file().then(|| key.display().to_string()), r.ok().then(|| r.stdout.clone())))
            })?;
            if json {
                out.json_doc(&doc);
            } else {
                out.data_lines(status_lines(&doc));
            }
            Ok(())
        }
    }
}

pub fn status_doc(facts: &DeviceFacts, unit: &str, agent_key: Option<String>, dom_tool_output: Option<String>) -> Value {
    let editable = facts.dom.as_ref().is_some_and(|d| d.is_editable()) && facts.firmware == Firmware::Dom;
    let reason = if editable {
        Value::Null
    } else if facts.arch.is_arm() {
        json!("ARM units boot through U-Boot; the boot manager is for x86 units with an internal boot flash (use the installer kit)")
    } else {
        json!("this unit has no editable internal boot flash (DOM)")
    };
    let tool = dom_tool_output.as_ref().map(|o| serde_json::from_str::<Value>(o.trim()).unwrap_or_else(|_| json!(o.trim())));
    json!({
        "schema": SCHEMA,
        "unit": unit,
        "model": facts.model,
        "firmware": facts.firmware.label(),
        "dom": facts.dom,
        "can_use_boot_manager": editable,
        "reason": reason,
        "agent_key": agent_key,
        "dom_tool": {"installed": tool.is_some(), "status": tool},
    })
}

pub fn status_lines(d: &Value) -> Vec<String> {
    let mut v = vec![
        format!("Unit:             {} ({})", d["unit"].as_str().unwrap_or(""), d["model"].as_str().unwrap_or("")),
        format!("Firmware:         {}", d["firmware"].as_str().unwrap_or("")),
        format!(
            "Boot manager:     {}",
            if d["can_use_boot_manager"].as_bool().unwrap_or(false) {
                "available".to_string()
            } else {
                format!("NOT available - {}", d["reason"].as_str().unwrap_or(""))
            }
        ),
        format!("Agent SSH key:    {}", d["agent_key"].as_str().unwrap_or("none yet (made on the first install)")),
    ];
    match d["dom_tool"]["status"].as_str() {
        Some(t) => {
            v.push("Boot flash tool (synofurb-dom status):".into());
            v.extend(t.lines().map(|l| format!("  {l}")));
        }
        None if d["dom_tool"]["installed"].as_bool() == Some(true) => {
            v.push(format!("Boot flash tool:  {}", d["dom_tool"]["status"]));
        }
        None => v.push("Boot flash tool:  not installed on the unit yet (run `boot-manager install`)".into()),
    }
    v
}

pub fn backup_dom(out: &Out, a: BackupDomArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let t = conn_target(&a.conn, out, &mut prompt)?;
    let dir = a.backup_dir.unwrap_or_else(|| config_dir().join("dom-backups"));
    out.event(json!({"event": "start", "command": "backup-dom", "host": t.host}));
    let secrets = target_secrets(&t);
    let rep = run_job(out, &secrets, move |lg, _| {
        with_unit(&t, lg, |ssh, facts| {
            let dom = facts
                .dom
                .as_ref()
                .filter(|d| d.is_editable() && facts.firmware == Firmware::Dom)
                .context("this unit has no editable internal boot flash (DOM)")?;
            let b = synofurb_core::domboot::backup_dom(
                ssh,
                facts.is_root,
                t.sudo_password.clone(),
                &dir,
                &unit_name(facts, &t),
                dom.sectors * SECTOR,
                lg,
            )?;
            Ok(b)
        })
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let b = rep.result.expect("checked");
    out.result_text(&[
        format!("Boot flash backed up: {}", b.path.display()),
        format!("  {} bytes, sha256 {} (verified against the NAS)", b.bytes, b.sha256),
        "KEEP THIS FILE: it is the only way to rebuild the unit's boot flash if it is ever damaged.".into(),
    ]);
    out.result_event("backup-dom", true, json!({"path": b.path.display().to_string(), "bytes": b.bytes, "sha256": b.sha256}));
    Ok(())
}

pub fn disk_backup(out: &Out, a: DiskBackupArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let t = conn_target(&a.conn, out, &mut prompt)?;
    let root = a.out_dir.unwrap_or_else(|| session::data_dir().join("backups"));
    let wanted = a.disk.clone();
    out.event(json!({"event": "start", "command": "disk-backup", "host": t.host}));
    let secrets = target_secrets(&t);
    let rep = run_job(out, &secrets, move |lg, _| {
        with_unit(&t, lg, |ssh, facts| {
            let disks = select_disks(facts, &wanted)?;
            synofurb_core::diskbackup::backup_disk_metadata(
                ssh,
                facts.is_root,
                t.sudo_password.clone(),
                &disks,
                &root,
                &unit_name(facts, &t),
                lg,
            )
            .map(|dir| (dir, disks))
        })
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let (dir, disks) = rep.result.expect("checked");
    out.result_text(&[
        format!("Disk metadata backed up to {}", dir.display()),
        format!("  disks: {}", disks.iter().map(|d| d.0.clone()).collect::<Vec<_>>().join(", ")),
        "This is the first and last MiB of each disk (partition tables), NOT a backup of your files. See the README.txt in that folder."
            .into(),
    ]);
    out.result_event(
        "disk-backup",
        true,
        json!({"dir": dir.display().to_string(), "disks": disks.iter().map(|d| &d.0).collect::<Vec<_>>()}),
    );
    Ok(())
}

/// Disks to back up: the named ones (must exist), else every internal install-target disk.
pub fn select_disks(facts: &DeviceFacts, wanted: &[String]) -> Result<Vec<(String, u64)>> {
    if wanted.is_empty() {
        let v: Vec<(String, u64)> = facts.candidate_disks().iter().map(|d| (d.name.clone(), d.sectors)).collect();
        if v.is_empty() {
            bail!("no internal disks found");
        }
        return Ok(v);
    }
    wanted
        .iter()
        .map(|n| {
            facts.disk(n).map(|d| (d.name.clone(), d.sectors)).ok_or_else(|| {
                anyhow!("no disk named '{n}' (disks: {})", facts.disks.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", "))
            })
        })
        .collect()
}

pub fn fetch_installer(out: &Out, a: FetchArgs) -> CliResult<()> {
    let arch = match a.arch {
        ArchArg::Amd64 => Arch::X86_64,
        ArchArg::Arm64 => Arch::Arm64,
        ArchArg::Armhf => Arch::Armhf,
    };
    if a.dtb.is_some() && arch != Arch::Armhf {
        return Err(CliError::usage("--dtb applies to armhf only"));
    }
    let cache = config_dir().join("cache");
    let (suite, firmware, dtb) = (a.suite.clone(), a.firmware, a.dtb.clone());
    out.event(json!({"event": "start", "command": "fetch-installer", "arch": arch.debian()}));
    let rep = run_job(out, &[], move |lg, _| {
        let mut progress = progress_fn(lg);
        synofurb_core::debian::fetch_installer(arch, &suite, &cache, firmware, dtb.as_deref(), lg, &mut progress)
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let f = rep.result.expect("checked");
    let doc = json!({
        "kernel": f.kernel.display().to_string(),
        "initrd": f.initrd.display().to_string(),
        "firmware": f.firmware.as_ref().map(|p| p.display().to_string()),
        "dtb": f.dtb.as_ref().map(|p| p.display().to_string()),
    });
    if a.json && !out.json_events() {
        out.json_doc(&doc);
    } else {
        let mut lines = vec![format!("kernel: {}", f.kernel.display()), format!("initrd: {}", f.initrd.display())];
        if let Some(p) = &f.firmware {
            lines.push(format!("firmware: {}", p.display()));
        }
        if let Some(p) = &f.dtb {
            lines.push(format!("dtb: {}", p.display()));
        }
        out.result_text(&lines);
    }
    out.result_event("fetch-installer", true, doc);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::facts::demo_facts;
    use super::*;

    #[test]
    fn disk_selection() {
        let (facts, _) = demo_facts();
        let all = select_disks(&facts, &[]).unwrap();
        assert_eq!(all.len(), 4);
        let one = select_disks(&facts, &[facts.disks[1].name.clone()]).unwrap();
        assert_eq!(one.len(), 1);
        assert!(select_disks(&facts, &["nope".into()]).unwrap_err().to_string().contains("no disk named 'nope'"));
    }

    #[test]
    fn boot_manager_status_documents() {
        let (mut facts, _) = demo_facts();
        let d = status_doc(&facts, "1920PDN123456", None, None);
        assert_eq!(d["can_use_boot_manager"], false);
        assert!(d["reason"].as_str().unwrap().contains("boot flash"));
        assert_eq!(d["dom_tool"]["installed"], false);
        let text = status_lines(&d).join("\n");
        assert!(text.contains("NOT available") && text.contains("not installed on the unit yet"));
        facts.firmware = Firmware::Dom;
        facts.dom = Some(DomFacts { sectors: 262_144, p1_sectors: 65_536, p2_sectors: 196_608, p1_magic: true, p2_magic: true });
        let d = status_doc(&facts, "u", Some("/k/agent_u_ed25519".into()), Some("{\"device\":\"/dev/synoboot\"}".into()));
        assert_eq!(d["can_use_boot_manager"], true);
        assert_eq!(d["dom_tool"]["status"]["device"], "/dev/synoboot");
        assert!(status_lines(&d).join("\n").contains("available"));
    }

    #[test]
    fn writing_to_the_boot_flash_needs_a_yes() {
        let mut prompt = super::super::prompt::Scripted::new(Vec::<String>::new());
        let out = Out::plain();
        assert_eq!(confirm_action(&out, &mut prompt, "q?").err().unwrap().code(), 3);
        let mut out = Out::plain();
        out.yes = true;
        assert!(confirm_action(&out, &mut prompt, "q?").is_ok());
        let mut out = Out::plain();
        out.interactive = true;
        let mut prompt = super::super::prompt::Scripted::new(["n"]);
        assert_eq!(confirm_action(&out, &mut prompt, "q?").err().unwrap().code(), 3);
        let mut prompt = super::super::prompt::Scripted::new(["y"]);
        assert!(confirm_action(&out, &mut prompt, "q?").is_ok());
    }
}
