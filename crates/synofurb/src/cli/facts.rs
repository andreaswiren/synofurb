//! Getting hold of a unit's facts (live over SSH, from a saved file, or the built-in demo unit) and turning facts, plans and
//! compatibility verdicts into the JSON documents the commands print. The JSON builders are pure so tests can check their shape.

use super::args::{Conn, LiveOrFacts, SshOpts};
use super::exit::{CliError, CliResult};
use super::out::Out;
use super::prompt::Prompter;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use synofurb_core::compat::{CompatDb, CompatVerdict, Status};
use synofurb_core::logs::Logger;
use synofurb_core::model::*;
use synofurb_core::plan::Plan;
use synofurb_core::platform::{self, PlatformInfo, Support};
use synofurb_core::remote::{Auth, SshSession};
use synofurb_core::session::{self, SshTarget, confirm_phrase};

/// Per-user data directory (logs, inventory, keys, caches). Same as the desktop app uses.
pub fn config_dir() -> PathBuf {
    session::data_dir()
}

// ---------------------------------------------------------------------------------------------- credentials

fn first_line_of(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let line = text.lines().next().unwrap_or("").trim_end_matches('\r').to_string();
    if line.is_empty() {
        bail!("{} is empty (the password is its first line)", path.display());
    }
    Ok(line)
}

/// Build the SSH target: key file, or password from file / environment / prompt. Nothing is ever taken from argv.
pub fn ssh_target(host: &str, o: &SshOpts, out: &Out, prompt: &mut dyn Prompter) -> CliResult<SshTarget> {
    // the host key is settled (and, on a first contact, confirmed) before any password is asked for or sent
    super::hostkey::install_policy(host, o.port, &o.hostkey, out, prompt)?;
    let sudo_env = std::env::var(&o.sudo_password_env).ok().filter(|s| !s.is_empty());
    let (auth, sudo) = match &o.key {
        Some(k) => {
            let pem = std::fs::read_to_string(k).with_context(|| format!("reading the key {}", k.display()))?;
            let sudo = match sudo_env {
                Some(s) => Some(s),
                None if o.user != "root" && out.interactive => {
                    Some(prompt.secret(&format!("sudo password for {}@{host} (Enter if none)", o.user))?).filter(|s| !s.is_empty())
                }
                None => None,
            };
            (Auth::Key(pem), sudo)
        }
        None => {
            let pw = if let Some(f) = &o.ssh_password_file {
                first_line_of(f)?
            } else if let Some(v) = std::env::var(&o.ssh_password_env).ok().filter(|s| !s.is_empty()) {
                v
            } else if out.interactive {
                prompt.secret(&format!("SSH password for {}@{host}", o.user))?
            } else {
                return Err(CliError::usage(format!(
                    "no SSH password: set ${} (or use --ssh-password-file FILE or --key FILE); there is no terminal to ask on",
                    o.ssh_password_env
                )));
            };
            let sudo = sudo_env.unwrap_or_else(|| pw.clone());
            (Auth::Password(pw), Some(sudo))
        }
    };
    Ok(SshTarget { host: host.to_string(), port: o.port, user: o.user.clone(), auth, sudo_password: sudo.filter(|s| !s.is_empty()) })
}

pub fn conn_target(c: &Conn, out: &Out, prompt: &mut dyn Prompter) -> CliResult<SshTarget> {
    ssh_target(&c.host, &c.ssh, out, prompt)
}

/// Every secret in a target (for log redaction).
pub fn target_secrets(t: &SshTarget) -> Vec<String> {
    let mut v = vec![];
    if let Auth::Password(p) = &t.auth {
        v.push(p.clone());
    }
    if let Some(s) = &t.sudo_password {
        v.push(s.clone());
    }
    v
}

// ---------------------------------------------------------------------------------------------- inspection

/// Where facts came from.
pub enum Source {
    Live(SshTarget),
    File(String),
    Demo,
}

pub struct Inspection {
    pub facts: DeviceFacts,
    /// Raw output of the inspection script (kept with the install). Empty when loaded from a facts file.
    pub raw: String,
    pub source: Source,
}

impl Inspection {
    pub fn target(&self) -> Option<&SshTarget> {
        match &self.source {
            Source::Live(t) => Some(t),
            _ => None,
        }
    }
    pub fn describe(&self) -> String {
        match &self.source {
            Source::Live(t) => format!("{}@{}", t.user, t.host),
            Source::File(p) => format!("saved inspection {p}"),
            Source::Demo => "built-in sample unit (demo)".into(),
        }
    }
}

/// Connect, run the read-only inspection script, disconnect.
pub fn inspect_live(t: &SshTarget, log: &Logger) -> Result<(DeviceFacts, String)> {
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, log)?;
    let r = session::inspect_remote(&mut ssh, false, t.sudo_password.clone(), log);
    ssh.close();
    r
}

pub fn demo_facts() -> (DeviceFacts, String) {
    let raw = synofurb_core::inspect::fixtures::ds_4bay();
    (synofurb_core::inspect::parse_inspect(&raw).expect("the built-in fixture parses"), raw)
}

/// Read facts from a file: a bare `DeviceFacts` JSON, the `inspect --json` document, or raw inspection output (`KV|...` lines).
pub fn parse_facts_text(text: &str) -> Result<(DeviceFacts, String)> {
    let t = text.trim_start();
    if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).context("not valid JSON")?;
        let f = if v.get("facts").is_some_and(Value::is_object) { v["facts"].clone() } else { v };
        let facts: DeviceFacts =
            serde_json::from_value(f).context("JSON does not look like inspection facts (write them with `inspect --save`)")?;
        return Ok((facts, String::new()));
    }
    if t.starts_with("KV|") || t.contains("\nKV|") {
        return Ok((synofurb_core::inspect::parse_inspect(text)?, text.to_string()));
    }
    bail!("unrecognised facts file: expected JSON written by `inspect --save`/`--json`, or raw inspection output")
}

/// Load facts from the live NAS, a file, or the demo unit, according to the flags.
pub fn load(src: &LiveOrFacts, out: &Out, prompt: &mut dyn Prompter, log: &Logger) -> CliResult<Inspection> {
    if let Some(f) = &src.facts {
        if f == "demo" {
            let (facts, raw) = demo_facts();
            return Ok(Inspection { facts, raw, source: Source::Demo });
        }
        let text = std::fs::read_to_string(f).with_context(|| format!("reading {f}"))?;
        let (facts, raw) = parse_facts_text(&text).with_context(|| f.clone())?;
        return Ok(Inspection { facts, raw, source: Source::File(f.clone()) });
    }
    let host = src.host.as_deref().ok_or_else(|| CliError::usage("give --host ADDRESS (or --facts FILE)"))?;
    let target = ssh_target(host, &src.ssh, out, prompt)?;
    out.info(&format!("Connecting to {}@{}:{} ...", target.user, target.host, target.port));
    let (facts, raw) = inspect_live(&target, log).with_context(|| format!("inspecting {host}"))?;
    Ok(Inspection { facts, raw, source: Source::Live(target) })
}

// ---------------------------------------------------------------------------------------------- compat / platform

pub fn load_compat_db() -> CompatDb {
    CompatDb::load_best(&config_dir())
}

pub fn support_word(s: Support) -> &'static str {
    s.label()
}

/// Everything the commands need to judge a unit.
pub struct Assessment<'a> {
    pub platform: PlatformInfo,
    pub verdict: CompatVerdict<'a>,
}

pub fn assess<'a>(db: &'a CompatDb, facts: &DeviceFacts) -> Assessment<'a> {
    Assessment { platform: platform::classify(&facts.unique, &facts.uname_m), verdict: db.status_for(facts) }
}

impl Assessment<'_> {
    /// Hardware that Debian 13 cannot run, according to live detection or the compatibility list. This is exit code 5.
    pub fn blocking(&self) -> Option<String> {
        if self.platform.support == Support::Unsupported {
            return Some(format!("this NAS platform is not supported: {}", self.platform.note));
        }
        if self.verdict.blocking {
            return Some(format!("{} ({})", self.verdict.headline, self.verdict.details.first().cloned().unwrap_or_default()));
        }
        None
    }
}

pub fn status_key(s: Status) -> &'static str {
    match s {
        Status::Verified => "verified",
        Status::Community => "community",
        Status::Untested => "untested",
        Status::KnownIssues => "known-issues",
        Status::Unsupported => "unsupported",
    }
}

pub fn parse_status(s: &str) -> Option<Status> {
    Status::ALL.into_iter().find(|st| status_key(*st) == s.trim().to_ascii_lowercase() || st.label().eq_ignore_ascii_case(s.trim()))
}

// ---------------------------------------------------------------------------------------------- JSON documents

pub const SCHEMA: u32 = 1;

pub fn platform_json(p: &PlatformInfo) -> Value {
    json!({"platform": p.platform, "arch": p.arch.debian(), "soc": p.soc, "support": p.support.label(), "note": p.note})
}

pub fn verdict_json(v: &CompatVerdict<'_>) -> Value {
    json!({
        "status": status_key(v.status),
        "status_label": v.status.label(),
        "headline": v.headline,
        "details": v.details,
        "blocking": v.blocking,
        "entry": v.entry.map(|e| serde_json::to_value(e).unwrap_or(Value::Null)),
    })
}

/// `inspect --json`
pub fn inspect_doc(facts: &DeviceFacts, a: &Assessment<'_>) -> Value {
    let serial = facts.serial_console();
    json!({
        "schema": SCHEMA,
        "facts": facts,
        "arch": facts.arch.debian(),
        "firmware": facts.firmware.label(),
        "cpu_vendor": facts.cpu_vendor.label(),
        "serial_console": {"tty": serial.tty, "speed": serial.speed, "raw": serial.raw, "kernel_arg": serial.kernel_arg()},
        "platform": platform_json(&a.platform),
        "compat": verdict_json(&a.verdict),
        "auto_boot_method": resolve_auto_method(facts),
        "confirm_phrase": confirm_phrase(facts),
        "supported": a.blocking().is_none(),
    })
}

/// `plan --json`
pub fn plan_doc(facts: &DeviceFacts, plan: &Plan, a: &Assessment<'_>) -> Value {
    json!({
        "schema": SCHEMA,
        "model": facts.model,
        "serial": facts.serial,
        "confirm_phrase": confirm_phrase(facts),
        "ok": plan.is_ok() && a.blocking().is_none(),
        "plan_ok": plan.is_ok(),
        "supported": a.blocking().is_none(),
        "errors": plan.errors,
        "warnings": plan.warnings,
        "summary": plan.summary(),
        "destroyed": plan.destroyed,
        "preserved": plan.preserved,
        "platform": platform_json(&a.platform),
        "compat": verdict_json(&a.verdict),
        "plan": plan,
    })
}

pub fn kernel_json(k: synofurb_core::config::KernelChannel) -> Value {
    use synofurb_core::config::KernelChannel;
    json!({"channel": if k == KernelChannel::Newest { "newest" } else { "stable" }, "label": k.label()})
}

/// What `--method auto` means for this unit (same rule as the GUI).
pub fn resolve_auto_method(facts: &DeviceFacts) -> &'static str {
    if facts.firmware == Firmware::Dom {
        "dom"
    } else if facts.kexec {
        "kexec"
    } else {
        "kit"
    }
}

/// `Err(reason)` when this unit cannot use the method (same rule as the GUI).
pub fn method_available(method: &str, facts: &DeviceFacts) -> Result<(), &'static str> {
    match method {
        "dom" => {
            if facts.firmware == Firmware::Dom && facts.dom.as_ref().is_some_and(|d| d.is_editable()) {
                Ok(())
            } else {
                Err("This unit has no editable internal boot flash (DOM).")
            }
        }
        "kexec" => {
            if facts.kexec {
                Ok(())
            } else {
                Err("The DSM kernel on this unit has no kexec support.")
            }
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::plan::{PlanRequest, build_plan};

    fn demo() -> DeviceFacts {
        demo_facts().0
    }

    #[test]
    fn facts_files_in_all_three_formats() {
        let (f, raw) = demo_facts();
        let (a, _) = parse_facts_text(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(a, f, "bare facts JSON round-trips");
        let db = CompatDb::embedded();
        let doc = inspect_doc(&f, &assess(&db, &f));
        let (b, _) = parse_facts_text(&doc.to_string()).unwrap();
        assert_eq!(b, f, "the inspect --json document is accepted too");
        let (c, kept) = parse_facts_text(&raw).unwrap();
        assert_eq!(c, f);
        assert_eq!(kept, raw, "raw output is kept for the payload");
        assert!(parse_facts_text("hello").is_err());
        assert!(parse_facts_text("{\"x\":1}").is_err());
    }

    #[test]
    fn inspect_document_shape() {
        let f = demo();
        let db = CompatDb::embedded();
        let a = assess(&db, &f);
        let v = inspect_doc(&f, &a);
        assert_eq!(v["schema"], 1);
        assert_eq!(v["facts"]["model"], "DS918+");
        assert_eq!(v["confirm_phrase"], "ERASE DS918+");
        assert_eq!(v["arch"], "amd64");
        assert_eq!(v["serial_console"]["kernel_arg"], "console=ttyS0,115200n8");
        assert_eq!(v["platform"]["support"], "supported");
        assert!(v["compat"]["status"].is_string() && v["compat"]["details"].is_array());
        assert_eq!(v["supported"], true);
        assert_eq!(v["auto_boot_method"], "kexec");
        assert!(v["facts"]["disks"].as_array().is_some_and(|d| d.len() >= 4));
        assert!(v["facts"]["arrays"].is_array() && v["facts"]["lvs"].is_array() && v["facts"]["mounts"].is_array());
    }

    #[test]
    fn plan_document_shape() {
        let f = demo();
        let plan = build_plan(&f, &PlanRequest::default());
        let db = CompatDb::embedded();
        let v = plan_doc(&f, &plan, &assess(&db, &f));
        assert_eq!(v["schema"], 1);
        assert_eq!(v["ok"], true);
        assert_eq!(v["confirm_phrase"], "ERASE DS918+");
        assert!(v["summary"].as_array().is_some_and(|s| !s.is_empty()));
        assert!(v["destroyed"].is_array() && v["preserved"].is_array() && v["errors"].is_array() && v["warnings"].is_array());
        assert_eq!(v["plan"]["mode"], "WipeAll");
        assert!(v["plan"]["sizes"]["chosen_system_mib"].as_u64().unwrap() > 0);
    }

    #[test]
    fn unsupported_platforms_block() {
        let mut f = demo();
        f.unique = "synology_88f6281_ds109".into();
        f.uname_m = "armv5tel".into();
        let db = CompatDb::embedded();
        let a = assess(&db, &f);
        assert!(a.blocking().is_some());
    }

    #[test]
    fn boot_method_rules_follow_the_gui() {
        let mut f = demo();
        assert_eq!(resolve_auto_method(&f), "kexec");
        assert!(method_available("dom", &f).is_err());
        assert!(method_available("kexec", &f).is_ok());
        f.kexec = false;
        assert_eq!(resolve_auto_method(&f), "kit");
        assert!(method_available("kexec", &f).is_err());
        assert!(method_available("kit", &f).is_ok());
    }

    #[test]
    fn status_names() {
        assert_eq!(parse_status("known-issues"), Some(Status::KnownIssues));
        assert_eq!(parse_status("Community-reported"), Some(Status::Community));
        assert_eq!(parse_status("nonsense"), None);
    }
}
