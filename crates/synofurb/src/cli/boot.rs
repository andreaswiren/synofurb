//! `recover`, `boot-manager`, `backup-dom`, `disk-backup` and `fetch-installer`: the commands that act on one unit or one download
//! without being a full install.

use super::args::{ArchArg, BackupDomArgs, BootManagerCmd, ConnArgs, DiskBackupArgs, FetchArgs};
use super::exit::{CliError, CliResult};
use super::facts::{SCHEMA, config_dir, conn_target, target_secrets};
use super::out::Out;
use super::prompt::{Prompter, StdPrompter};
use super::runner::{console_logger, progress_fn, run_job};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use synofurb_core::logs::Logger;
use synofurb_core::model::*;
use synofurb_core::remote::{Privileged, RemoteShell, SshSession};
use synofurb_core::session::{self, SshTarget, agent_key_for, config_keys_dir, inspect_remote, recover_to_dsm};

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

pub const RECOVERY_NOTE: &str = "The unit will boot DSM at its next reboot. If DSM's system partition is gone (a failed install), DSM starts in its recovery mode: open https://find.synology.com and choose to reinstall DSM; Synology keeps data volumes on a reinstall (see https://kb.synology.com).";

pub fn recover(out: &Out, a: ConnArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let t = conn_target(&a.conn, out, &mut prompt)?;
    recover_target(out, t)
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
fn confirm_action(out: &Out, prompt: &mut dyn Prompter, question: &str) -> CliResult<()> {
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
                "This reboots the unit into the boot agent (the agent returns to DSM by itself if nobody claims it).",
            )?;
            let t = conn_target(&conn, out, &mut prompt)?;
            out.event(json!({"event": "start", "command": "boot-manager enter", "host": t.host}));
            let secrets = target_secrets(&t);
            let rep = run_job(out, &secrets, move |lg, cancel| enter_agent(&t, &cancel, lg));
            if let Some(e) = rep.plain_failure() {
                return Err(e);
            }
            let (ip, key_path, unit) = rep.result.expect("checked");
            let how = format!(
                "Boot agent is running at {ip}. You control the bootloader over SSH:\n  ssh -i {} root@{ip}\n  sf-boot status | sf-boot kexec <kernel-url> <initrd-url> --cmdline '...' | sf-boot reboot dsm|debian\nIt returns to DSM by itself if nobody claims it within the timeout.",
                key_path.display()
            );
            out.result_text(&["".to_string(), how]);
            out.result_event(
                "boot-manager enter",
                true,
                json!({"agent_address": ip.to_string(), "ssh_key": key_path.display().to_string(), "unit": unit}),
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

/// Arm the agent, reboot DSM and wait until the agent answers over SSH. Returns (agent address, private key path, unit).
fn enter_agent(t: &SshTarget, cancel: &std::sync::atomic::AtomicBool, log: &Logger) -> Result<(std::net::IpAddr, PathBuf, String)> {
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, log)?;
    let fp = ssh.host_key_fingerprint();
    let (facts, _) = inspect_remote(&mut ssh, false, t.sudo_password.clone(), log)?;
    let unit = unit_name(&facts, t);
    let key = agent_key_for(&unit, &config_keys_dir())?;
    let mut p = Privileged { sh: &mut ssh, is_root: facts.is_root, sudo_password: t.sudo_password.clone() };
    let r = p.run("synofurb-dom default agent --yes || /usr/local/bin/synofurb-dom default agent --yes")?;
    if !r.ok() {
        bail!("could not arm the agent (is it installed? run `boot-manager install` first): {}", r.stdout.trim());
    }
    if cancel.load(Ordering::Relaxed) {
        let _ = synofurb_core::domboot::default_to_dsm(&mut ssh, facts.is_root, t.sudo_password.clone());
        bail!("cancelled before the reboot; the default boot entry is DSM again");
    }
    let mut p = Privileged { sh: &mut ssh, is_root: facts.is_root, sudo_password: t.sudo_password.clone() };
    let _ = p.run("nohup /bin/sh -c 'sleep 2; sync; reboot' >/dev/null 2>&1 &");
    let ip: std::net::IpAddr = t.host.parse().context("host must be an IP address for this command")?;
    let (agent, ip) = synofurb_core::domboot::wait_for_agent(
        &[ip],
        &key.private_openssh,
        fp.as_deref(),
        std::time::Duration::from_secs(600),
        cancel,
        log,
    )?;
    drop(agent);
    let safe: String = unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    Ok((ip, config_keys_dir().join(format!("agent_{safe}_ed25519")), unit))
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
