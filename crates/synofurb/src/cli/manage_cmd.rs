//! `manage health|version|update|rollback|open`: day-two management of a converted unit - basic health and updates of the web
//! management UI that runs on it. The engine is `synofurb_core::manage`; this file is argument handling, confirmation, output and exit
//! codes. Nothing here touches disks or the boot flash.

use super::args::{Conn, ManageCmd};
use super::boot::confirm_action;
use super::exit::{CliError, CliResult};
use super::facts::{SCHEMA, conn_target, target_secrets};
use super::out::{Out, Style};
use super::prompt::StdPrompter;
use super::runner::{console_logger, run_job};
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::path::PathBuf;
use synofurb_core::logs::Logger;
use synofurb_core::manage::{
    self, Health, HealthSummary, ManageError, ManageStatus, MgmtBundle, Overall, UpdateDecision, UpdateOptions, UpdatePlan, UpdateReport,
    VersionRelation, plan_update, push_rollback, push_update, web_url,
};
use synofurb_core::remote::{Privileged, RemoteShell, SshSession};
use synofurb_core::session::SshTarget;

pub fn manage(out: &Out, action: ManageCmd) -> CliResult<()> {
    match action {
        ManageCmd::Open { host } => open(out, &host),
        ManageCmd::Health { conn, json } => health(out, conn, json),
        ManageCmd::Version { conn, json } => version(out, conn, json),
        ManageCmd::Update { conn, from_bundle, force, dry_run } => update(out, conn, from_bundle, force, dry_run),
        ManageCmd::Rollback { conn } => rollback(out, conn),
    }
}

/// Connect, find out whether the SSH user is root, run `f`, disconnect.
fn with_unit<R>(t: &SshTarget, lg: &Logger, f: impl FnOnce(&mut Privileged<'_>) -> Result<R>) -> Result<R> {
    let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, lg)?;
    let r = (|| {
        let id = ssh.exec("id -u", None)?;
        let mut p = Privileged { sh: &mut ssh, is_root: id.stdout.trim() == "0", sudo_password: t.sudo_password.clone() };
        f(&mut p)
    })();
    ssh.close();
    r
}

// ---------------------------------------------------------------------------------------------- open

fn open(out: &Out, host: &str) -> CliResult<()> {
    let h = host.trim();
    if h.is_empty() || h.chars().any(|c| c.is_whitespace() || matches!(c, '/' | '\\' | '@' | '?' | '#' | '\'' | '"')) {
        return Err(CliError::usage(format!("--host {host:?} is not an address or host name")));
    }
    let url = web_url(h);
    out.event(json!({"event": "start", "command": "manage open", "host": h}));
    out.result_text(std::slice::from_ref(&url));
    out.info("The web UI uses a self-signed certificate; the browser warns once. Compare the fingerprint with `curl -k` on the unit (see docs/MANAGE.md).");
    out.result_event("manage open", true, json!({"host": h, "url": url}));
    Ok(())
}

// ---------------------------------------------------------------------------------------------- health

fn health(out: &Out, conn: Conn, as_json: bool) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let t = conn_target(&conn, out, &mut prompt)?;
    let log = console_logger(out);
    out.info(&format!("Connecting to {}@{}:{} ...", t.user, t.host, t.port));
    let fetched: Result<Health, ManageError> = with_unit(&t, &log, |p| match manage::fetch_health(p) {
        Ok(h) => Ok(Ok(h)),
        Err(ManageError::Transport(e)) => Err(e),
        // an old management plane: say which version it is
        Err(ManageError::HealthUnsupported { .. }) => {
            let installed = manage::read_installed(p.sh)?;
            Ok(Err(ManageError::HealthUnsupported { installed_version: installed.version }))
        }
        Err(e) => Ok(Err(e)),
    })
    .with_context(|| format!("asking {}", t.host))?;
    let h = fetched.map_err(|e| CliError::Other(anyhow!("{}: {e}", t.host)))?;
    let s = h.summary();
    let doc = health_doc(&t.host, &h, &s);
    if as_json {
        out.json_doc(&doc);
    } else {
        let tag = out.paint_out(overall_style(s.overall), s.overall.tag());
        out.result_text(&health_lines(&t.host, &h, &s, &tag));
    }
    out.result_event("manage health", true, doc);
    match s.overall {
        Overall::Crit => Err(CliError::Health { critical: true, message: format!("{}: health CRIT ({} problems)", t.host, s.problems) }),
        Overall::Warn => Err(CliError::Health { critical: false, message: format!("{}: health WARN ({} problems)", t.host, s.problems) }),
        _ => Ok(()),
    }
}

fn overall_style(o: Overall) -> Style {
    match o {
        Overall::Ok => Style::Green,
        Overall::Warn => Style::Yellow,
        Overall::Crit => Style::Red,
        Overall::Unknown => Style::Dim,
    }
}

/// The `--json` document of `manage health`.
pub fn health_doc(host: &str, h: &Health, s: &HealthSummary) -> Value {
    json!({"schema": SCHEMA, "host": host, "overall": s.overall.as_str(), "summary": s, "health": h, "web_ui": web_url(host)})
}

/// "3d 4h", "5h 12m", "42m", "30s".
pub fn human_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{secs}s"),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// Decimal units, like disk labels: "4.0 TB", "512 GB", "200 MB".
pub fn human_bytes(b: u64) -> String {
    const U: [(&str, f64); 4] = [("TB", 1e12), ("GB", 1e9), ("MB", 1e6), ("kB", 1e3)];
    for (name, size) in U {
        if b as f64 >= size {
            let v = b as f64 / size;
            return if v >= 100.0 { format!("{v:.0} {name}") } else { format!("{v:.1} {name}") };
        }
    }
    format!("{b} B")
}

fn pct(p: Option<f64>) -> String {
    p.map(|p| format!("{}%", p.round() as i64)).unwrap_or_else(|| "?".into())
}

/// The human report. `tag` is the (possibly coloured) verdict tag, so the caller owns the colour policy.
pub fn health_lines(host: &str, h: &Health, s: &HealthSummary, tag: &str) -> Vec<String> {
    let mut v = vec![];
    let name = if h.hostname.is_empty() || h.hostname == host { host.to_string() } else { format!("{host} ({})", h.hostname) };
    v.push(format!("{name}: {tag} - {}", s.headline));
    let mut sys = vec![format!("management plane {}", h.version)];
    if !h.os.is_empty() {
        sys.push(h.os.clone());
    }
    if !h.kernel.is_empty() {
        sys.push(format!("kernel {}", h.kernel));
    }
    v.push(format!("  {}", sys.join(" | ")));
    let mut run = vec![];
    if let Some(u) = h.uptime_s {
        run.push(format!("up {}", human_uptime(u)));
    }
    if !h.load.is_empty() {
        run.push(format!("load {}", h.load.iter().map(|l| format!("{l:.2}")).collect::<Vec<_>>().join(" ")));
    }
    if let Some(m) = h.mem.used_pct() {
        run.push(format!("memory {}% used", m.round() as i64));
    }
    if let Some(r) = h.root_fs.used_pct {
        run.push(format!("system disk {}% used", r.round() as i64));
    }
    if !run.is_empty() {
        v.push(format!("  {}", run.join(" | ")));
    }
    if !s.bullets.is_empty() {
        v.push("Problems:".to_string());
        for b in &s.bullets {
            v.push(format!("  {} {}", if b.level == Overall::Crit { "!!" } else { "! " }, b.text));
        }
    }
    if !h.arrays.is_empty() {
        v.push("Arrays:".to_string());
        for a in &h.arrays {
            let mut l = format!(
                "  {}  {}  {}",
                a.name,
                if a.level.is_empty() { "-" } else { a.level.as_str() },
                if a.degraded { "DEGRADED" } else { a.state.as_str() }
            );
            if !a.sync_action.is_empty() {
                l.push_str(&format!("  {} {}", a.sync_action, pct(a.sync_pct)));
            }
            v.push(l);
        }
    }
    if !h.disks.is_empty() {
        v.push("Disks:".to_string());
        for d in &h.disks {
            let mut l = format!("  {}  {}", d.dev.trim_start_matches("/dev/"), if d.model.is_empty() { "-" } else { d.model.as_str() });
            if let Some(sz) = d.size {
                l.push_str(&format!("  {}", human_bytes(sz)));
            }
            l.push_str(&format!("  SMART {}", d.smart.as_str()));
            if let Some(t) = d.temp_c {
                l.push_str(&format!("  {} C", t.round() as i64));
            }
            v.push(l);
        }
    }
    if !h.volumes.is_empty() {
        v.push("Volumes:".to_string());
        for vol in &h.volumes {
            v.push(format!("  {}  {} used  {}", vol.mount, pct(vol.used_pct), vol.fstype));
        }
    }
    if !h.services.is_empty() {
        let list: Vec<String> = h
            .services
            .iter()
            .map(|x| {
                format!(
                    "{} {}",
                    x.name,
                    match x.active {
                        Some(true) => "up",
                        Some(false) => "DOWN",
                        None => "?",
                    }
                )
            })
            .collect();
        v.push(format!("Services: {}", list.join(", ")));
    }
    if h.updates.apt_pending.is_some() || h.updates.security_pending.is_some() {
        v.push(format!(
            "Debian updates: {} pending, {} security",
            h.updates.apt_pending.map(|n| n.to_string()).unwrap_or_else(|| "?".into()),
            h.updates.security_pending.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
        ));
    }
    v.push(format!("Web UI: {}", web_url(host)));
    for n in &h.parse_notes {
        v.push(format!("note: {n}"));
    }
    v
}

// ---------------------------------------------------------------------------------------------- version

fn version(out: &Out, conn: Conn, as_json: bool) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let t = conn_target(&conn, out, &mut prompt)?;
    let log = console_logger(out);
    out.info(&format!("Connecting to {}@{}:{} ...", t.user, t.host, t.port));
    let bundle = MgmtBundle::embedded();
    let st = with_unit(&t, &log, |p| manage::status(p, &bundle)).with_context(|| format!("asking {}", t.host))?;
    let doc = version_doc(&t.host, &st);
    if as_json {
        out.json_doc(&doc);
    } else {
        out.result_text(&version_lines(&t.host, &st));
    }
    out.result_event("manage version", true, doc);
    Ok(())
}

pub fn version_doc(host: &str, st: &ManageStatus) -> Value {
    json!({
        "schema": SCHEMA,
        "host": host,
        "installed": st.installed,
        "bundled": {"version": st.bundled_version},
        "relation": st.relation,
        "update_available": st.plan.decision.applies() && st.plan.decision != UpdateDecision::Reinstall && st.plan.decision != UpdateDecision::Downgrade,
        "decision": st.plan.decision,
        "message": st.plan.message,
        "health_error": st.health_error,
        "overall": st.summary.as_ref().map(|s| s.overall.as_str()),
    })
}

pub fn version_lines(host: &str, st: &ManageStatus) -> Vec<String> {
    let mut v = vec![];
    match (&st.installed.present, &st.installed.version) {
        (true, Some(ver)) => v.push(format!("On {host}: management plane {ver}")),
        (true, None) => v.push(format!("On {host}: management plane (version unknown)")),
        (false, _) => v.push(format!("On {host}: the management plane is not installed")),
    }
    v.push(format!("This program ships: {}", st.bundled_version));
    v.push(match st.relation {
        Some(VersionRelation::Same) => "The versions match.".to_string(),
        Some(VersionRelation::NasOlder) => "The unit is older.".to_string(),
        Some(VersionRelation::NasNewer) => "The unit is newer than this program's copy.".to_string(),
        Some(VersionRelation::Unknown) | None => "The versions cannot be compared.".to_string(),
    });
    if let Some(e) = &st.health_error {
        v.push(format!("Health: not available ({e})"));
    } else if let Some(s) = &st.summary {
        v.push(format!("Health: {} ({})", s.overall.tag(), s.headline));
    }
    v.push(st.plan.message.clone());
    if st.plan.decision.applies() && st.plan.decision != UpdateDecision::Downgrade {
        v.push(format!("To update: synofurb-cli manage update --host {host}"));
    }
    v
}

// ---------------------------------------------------------------------------------------------- update

fn plan_lines(host: &str, st: &ManageStatus, plan: &UpdatePlan) -> Vec<String> {
    let installed = match (&st.installed.present, &st.installed.version) {
        (true, Some(v)) => format!("management plane {v}"),
        (true, None) => "management plane (version unknown)".to_string(),
        (false, _) => "no management plane".to_string(),
    };
    let health = match (&st.summary, &st.health_error) {
        (Some(s), _) => format!(", health {}", s.overall.tag()),
        (None, Some(_)) if st.installed.present => ", no health report".to_string(),
        _ => String::new(),
    };
    vec![
        format!("{host}: {installed}{health}"),
        format!("Source: {} (version {})", plan.source.label(), plan.new_version),
        plan.message.clone(),
    ]
}

fn update(out: &Out, conn: Conn, from_bundle: Option<PathBuf>, force: bool, dry_run: bool) -> CliResult<()> {
    let bundle = match &from_bundle {
        Some(d) => MgmtBundle::from_dir(d).map_err(|e| CliError::usage(format!("--from-bundle {}: {e:#}", d.display())))?,
        None => MgmtBundle::embedded(),
    };
    let mut prompt = StdPrompter;
    let t = conn_target(&conn, out, &mut prompt)?;
    out.event(json!({"event": "start", "command": "manage update", "host": t.host, "version": bundle.version, "dry_run": dry_run}));
    let log = console_logger(out);
    out.info(&format!("Connecting to {}@{}:{} ...", t.user, t.host, t.port));
    // a read-only look first, so the question can name versions and "up to date" needs no question
    let st = with_unit(&t, &log, |p| manage::status(p, &bundle)).with_context(|| format!("asking {}", t.host))?;
    let plan = plan_update(&st.installed, &bundle, force);
    for l in plan_lines(&t.host, &st, &plan) {
        out.info(&l);
    }
    if dry_run {
        out.result_text(&[format!("Dry run, nothing was changed. {}", plan.message)]);
        out.result_event("manage update", true, json!({"dry_run": true, "host": t.host, "plan": plan, "installed": st.installed}));
        return Ok(());
    }
    match plan.decision {
        UpdateDecision::UpToDate => {
            out.result_text(std::slice::from_ref(&plan.message));
            out.result_event("manage update", true, json!({"outcome": "up_to_date", "host": t.host, "plan": plan}));
            return Ok(());
        }
        UpdateDecision::DowngradeNeedsForce => return Err(CliError::refused(format!("{} Nothing was changed.", plan.message))),
        _ => {}
    }
    confirm_action(
        out,
        &mut prompt,
        &format!(
            "{} The web UI on {} restarts and is unavailable for a few seconds; the unit goes back to the old version by itself if the new one does not come up. Disks and data are not touched.",
            plan.message, t.host
        ),
    )?;
    let secrets = target_secrets(&t);
    let (tt, b2) = (t.clone(), bundle.clone());
    let rep = run_job(out, &secrets, move |lg, cancel| {
        let opts = UpdateOptions { force, cancel: Some(cancel), ..Default::default() };
        let lg2 = lg.clone();
        with_unit(&tt, lg, |p| push_update(p, &b2, &opts, &mut |e| e.emit(&lg2)))
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let log_path = rep.log_path.clone();
    let report = rep.result.expect("checked above");
    finish(out, "manage update", &t.host, &report, log_path.as_deref())
}

// ---------------------------------------------------------------------------------------------- rollback

fn rollback(out: &Out, conn: Conn) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let t = conn_target(&conn, out, &mut prompt)?;
    confirm_action(
        out,
        &mut prompt,
        &format!(
            "Go back to the management plane version that {} had before its last update? The web UI restarts and is unavailable for a few seconds.",
            t.host
        ),
    )?;
    out.event(json!({"event": "start", "command": "manage rollback", "host": t.host}));
    let secrets = target_secrets(&t);
    let tt = t.clone();
    let rep = run_job(out, &secrets, move |lg, cancel| {
        let opts = UpdateOptions { cancel: Some(cancel), ..Default::default() };
        let lg2 = lg.clone();
        with_unit(&tt, lg, |p| push_rollback(p, &opts, &mut |e| e.emit(&lg2)))
    });
    if let Some(e) = rep.plain_failure() {
        return Err(e);
    }
    let log_path = rep.log_path.clone();
    let report = rep.result.expect("checked above");
    finish(out, "manage rollback", &t.host, &report, log_path.as_deref())
}

// ---------------------------------------------------------------------------------------------- result

/// Print the outcome and turn it into the exit code: success 0, a downgrade that needs `--force` 3, a failed update 1 (the message
/// says whether the unit is back on its old version).
fn finish(out: &Out, command: &str, host: &str, r: &UpdateReport, log: Option<&std::path::Path>) -> CliResult<()> {
    let data = serde_json::to_value(r).unwrap_or(Value::Null);
    out.result_event(command, r.ok(), data);
    match r.outcome {
        manage::UpdateOutcome::Updated | manage::UpdateOutcome::UpToDate | manage::UpdateOutcome::RolledBack => {
            let mut lines = vec![r.headline()];
            if let Some(s) = &r.summary_after {
                lines.push(format!("Health: {} ({})", s.overall.tag(), s.headline));
                lines.extend(s.bullets.iter().map(|b| format!("  ! {}", b.text)));
            }
            lines.push(format!("Web UI: {}", web_url(host)));
            out.result_text(&lines);
            Ok(())
        }
        manage::UpdateOutcome::Refused => Err(CliError::refused(r.headline())),
        manage::UpdateOutcome::Failed => {
            let mut m = r.headline();
            if let Some(p) = log {
                m.push_str(&format!(" (log: {})", p.display()));
            }
            Err(CliError::Other(anyhow!(m)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::manage::parse_health;

    const DOC: &str = r#"{"schema":1,"version":"0.4.0","hostname":"nas","uptime_s":273600,"load":[0.12,0.1,0.05],
    "mem":{"total":8000000000,"avail":6000000000},"os":"Debian GNU/Linux 13 (trixie)","kernel":"6.12.41","root_fs":{"used_pct":23},
    "arrays":[{"name":"md2","level":"raid5","state":"degraded","degraded":true,"sync_pct":42.5,"sync_action":"recovery"}],
    "disks":[{"dev":"/dev/sda","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":34},
             {"dev":"/dev/sdb","model":null,"size":null,"smart":"failed","temp_c":null}],
    "volumes":[{"mount":"/srv/data","used_pct":41,"fstype":"ext4"}],
    "services":[{"name":"synofurb-mgmt","active":true},{"name":"ssh","active":false}],
    "updates":{"apt_pending":4,"security_pending":1},"overall":"crit","warnings":["md2 degraded"]}"#;

    #[test]
    fn units_are_human() {
        assert_eq!(human_uptime(30), "30s");
        assert_eq!(human_uptime(42 * 60), "42m");
        assert_eq!(human_uptime(5 * 3600 + 12 * 60), "5h 12m");
        assert_eq!(human_uptime(3 * 86_400 + 4 * 3600 + 59), "3d 4h");
        assert_eq!(human_bytes(4_000_787_030_016), "4.0 TB");
        assert_eq!(human_bytes(512_000_000_000), "512 GB");
        assert_eq!(human_bytes(12_000_000), "12.0 MB");
        assert_eq!(human_bytes(12), "12 B");
    }

    #[test]
    fn the_health_report_reads_well_and_stays_short() {
        let h = parse_health(DOC).unwrap();
        let s = h.summary();
        let lines = health_lines("192.168.1.20", &h, &s, "CRIT");
        assert_eq!(lines[0], "192.168.1.20 (nas): CRIT - Problem");
        assert!(lines[1].contains("management plane 0.4.0") && lines[1].contains("kernel 6.12.41"), "{lines:?}");
        assert!(
            lines[2].contains("up 3d 4h") && lines[2].contains("load 0.12 0.10 0.05") && lines[2].contains("memory 25% used"),
            "{lines:?}"
        );
        let text = lines.join("\n");
        assert!(text.contains("Problems:\n  !! md2 is degraded, recovery 43%."), "{text}");
        assert!(text.contains("md2  raid5  DEGRADED  recovery 43%"), "{text}");
        assert!(text.contains("sda  WDC WD40EFRX  4.0 TB  SMART passed  34 C"), "{text}");
        assert!(text.contains("sdb  -  SMART failed"), "{text}");
        assert!(text.contains("/srv/data  41% used  ext4"));
        assert!(text.contains("Services: synofurb-mgmt up, ssh DOWN"));
        assert!(text.contains("Debian updates: 4 pending, 1 security"));
        assert!(text.ends_with("Web UI: https://192.168.1.20:8443"));
        assert!(lines.len() < 25);
        // a minimal report prints only what it has
        let h = parse_health(r#"{"schema":1,"version":"0.4.0","overall":"ok"}"#).unwrap();
        let lines = health_lines("nas.lan", &h, &h.summary(), "OK");
        assert_eq!(lines[0], "nas.lan: OK - Healthy");
        assert_eq!(lines.len(), 3, "{lines:?}");
    }

    #[test]
    fn the_json_document_has_a_stable_shape() {
        let h = parse_health(DOC).unwrap();
        let doc = health_doc("10.0.0.2", &h, &h.summary());
        assert_eq!((doc["schema"].as_u64(), doc["host"].as_str(), doc["overall"].as_str()), (Some(1), Some("10.0.0.2"), Some("crit")));
        assert_eq!(doc["health"]["version"], "0.4.0");
        assert_eq!(doc["health"]["arrays"][0]["name"], "md2");
        assert_eq!(doc["summary"]["degraded_arrays"], 1);
        assert_eq!(doc["summary"]["failed_disks"], 1);
        assert_eq!(doc["summary"]["services_down"], 1);
        assert_eq!(doc["summary"]["bullets"][0]["level"], "crit");
        assert_eq!(doc["web_ui"], "https://10.0.0.2:8443");
    }

    fn status_for(nas: Option<&str>, bundled: &str) -> ManageStatus {
        let mut b = MgmtBundle::embedded();
        b.version = bundled.into();
        let installed = synofurb_core::manage::InstalledInfo { present: nas.is_some(), version: nas.map(Into::into), script_sha256: None };
        ManageStatus {
            plan: plan_update(&installed, &b, false),
            relation: nas.map(|n| manage::compare(n, bundled)),
            bundled_version: bundled.into(),
            summary: None,
            health: None,
            health_error: None,
            installed,
        }
    }

    #[test]
    fn version_output_names_both_versions_and_the_next_step() {
        let st = status_for(Some("0.2.0"), "0.4.0");
        let l = version_lines("nas", &st);
        assert_eq!(l[0], "On nas: management plane 0.2.0");
        assert_eq!(l[1], "This program ships: 0.4.0");
        assert_eq!(l[2], "The unit is older.");
        assert!(l.last().unwrap().contains("manage update --host nas"));
        let d = version_doc("nas", &st);
        assert_eq!(
            (d["update_available"].as_bool(), d["decision"].as_str(), d["relation"].as_str()),
            (Some(true), Some("upgrade"), Some("nas_older"))
        );
        // equal: nothing to do and no hint
        let st = status_for(Some("0.4.0"), "0.4.0");
        let l = version_lines("nas", &st);
        assert!(l.iter().any(|x| x.contains("up to date")) && !l.iter().any(|x| x.contains("To update")), "{l:?}");
        assert_eq!(version_doc("nas", &st)["update_available"], false);
        // newer on the unit: no update offered
        let st = status_for(Some("0.9.0"), "0.4.0");
        assert_eq!(version_doc("nas", &st)["update_available"], false);
        assert!(version_lines("nas", &st).iter().any(|x| x.contains("--force")));
        // not installed
        let st = status_for(None, "0.4.0");
        assert!(version_lines("nas", &st)[0].contains("not installed"));
    }

    #[test]
    fn open_prints_only_the_address() {
        assert!(open(&Out::plain(), "192.168.1.20").is_ok());
        for bad in ["", "  ", "nas/../x", "a b", "u@h", "h?x", "h'h"] {
            assert!(matches!(open(&Out::plain(), bad), Err(CliError::Usage(_))), "{bad:?}");
        }
    }
}
