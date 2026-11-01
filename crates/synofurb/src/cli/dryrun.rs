//! `install --dry-run`: do everything up to rendering and show it, without staging, serving or booting anything.
//! Secrets never appear: the password hash is a placeholder and the SSH key is a throw-away public key.

use super::exit::{CliError, CliResult};
use super::facts::SCHEMA;
use super::out::Out;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use synofurb_core::config::InstallConfig;
use synofurb_core::debian::{self, PreseedParams};
use synofurb_core::model::DeviceFacts;
use synofurb_core::offline::{Bundle, MIRROR_NAME};
use synofurb_core::payload::{OfflineEnv, build_payload, build_payload_offline};
use synofurb_core::plan::Plan;
use synofurb_core::script::{ScriptParams, render_stage2};
use synofurb_core::server::OFFLINE_MIRROR_ROUTE;
use synofurb_core::session::confirm_phrase;

pub const PLACEHOLDER_HTTP_PORT: u16 = 18080;
pub const PLACEHOLDER_TCP_PORT: u16 = 18081;
pub const PLACEHOLDER_UDP_PORT: u16 = 18082;
pub const PLACEHOLDER_TOKEN: &str = "DRYRUN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub mode: u32,
    pub size: u64,
    pub sha256: String,
}

pub struct Rendered {
    pub stage2: String,
    pub preseed: String,
    pub plan_json: String,
    pub cfg_json: Value,
    pub payload: Vec<u8>,
    pub payload_sha256: String,
    pub manifest: Vec<ManifestEntry>,
    pub cmdline: String,
    pub http_base: String,
    pub bind_ip: IpAddr,
    /// The offline bundle the script and payload were rendered for, if any.
    pub bundle: Option<String>,
}

/// The address the generated script would call back to: this computer's address towards the NAS when known, else a documentation address.
pub fn bind_ip_for(host: Option<&str>) -> IpAddr {
    let placeholder = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    let Some(host) = host else { return placeholder };
    use std::net::ToSocketAddrs;
    let peer = host.parse::<IpAddr>().ok().or_else(|| (host, 22).to_socket_addrs().ok()?.next().map(|a| a.ip()));
    peer.and_then(|p| synofurb_core::server::local_ip_towards(p).ok()).unwrap_or(placeholder)
}

/// Render everything a real run would generate. With `offline_bundle` the mirror settings and the payload are the offline ones
/// (exactly what `session::stage` does): the NAS then fetches packages from the bundle served by this computer.
pub fn render(
    cfg: &InstallConfig,
    plan: &Plan,
    facts: &DeviceFacts,
    raw: &str,
    bind_ip: IpAddr,
    offline_bundle: Option<&Path>,
) -> Result<Rendered> {
    if !plan.is_ok() {
        bail!("the plan has errors: {}", plan.errors.join("; "));
    }
    cfg.validate()?;
    let http_base = format!("http://{bind_ip}:{PLACEHOLDER_HTTP_PORT}/{PLACEHOLDER_TOKEN}");
    let mut cfg = cfg.clone();
    let payload = match offline_bundle {
        Some(dir) => {
            let b = Bundle::open(dir).with_context(|| format!("opening the offline bundle {}", dir.display()))?;
            if !bind_ip.is_ipv4() {
                bail!("offline installs need an IPv4 address of this computer (the installer fetches packages from {bind_ip})");
            }
            cfg.mirror_host = format!("{bind_ip}:{PLACEHOLDER_HTTP_PORT}");
            cfg.mirror_dir = format!("/{PLACEHOLDER_TOKEN}/{OFFLINE_MIRROR_ROUTE}/{MIRROR_NAME}");
            cfg.validate()?;
            build_payload_offline(&cfg, plan, facts, raw, &OfflineEnv { suites: b.suites().to_vec() })?
        }
        None => build_payload(&cfg, plan, facts, raw)?,
    };
    let cfg = &cfg;
    let stage2 = render_stage2(&ScriptParams {
        plan,
        facts,
        cfg,
        run_id: "dryrun",
        http_base: &http_base,
        payload_sha256: &payload.sha256,
        log_host: &bind_ip.to_string(),
        log_tcp_port: PLACEHOLDER_TCP_PORT,
        dry_run: false,
    })?;
    let preseed = debian::render_preseed(&PreseedParams { cfg, http_base: &http_base });
    let cmdline = debian::installer_cmdline(&cfg.serial, &bind_ip.to_string(), PLACEHOLDER_UDP_PORT, true, None);
    let manifest = manifest_of(&payload.bytes)?;
    Ok(Rendered {
        stage2,
        preseed,
        plan_json: serde_json::to_string_pretty(plan)?,
        cfg_json: serde_json::to_value(cfg)?,
        payload_sha256: payload.sha256,
        payload: payload.bytes,
        manifest,
        cmdline,
        http_base,
        bind_ip,
        bundle: offline_bundle.map(|d| d.display().to_string()),
    })
}

/// List the files inside the generated payload archive with their hashes.
pub fn manifest_of(tar_gz: &[u8]) -> Result<Vec<ManifestEntry>> {
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(tar_gz));
    let mut v = vec![];
    for e in ar.entries()? {
        let mut e = e?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let path = e.path()?.to_string_lossy().to_string();
        let mode = e.header().mode().unwrap_or(0);
        let mut data = vec![];
        e.read_to_end(&mut data)?;
        v.push(ManifestEntry { path, mode, size: data.len() as u64, sha256: hex::encode(Sha256::digest(&data)) });
    }
    v.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(v)
}

fn manifest_json(m: &[ManifestEntry]) -> Value {
    Value::Array(
        m.iter().map(|e| json!({"path": e.path, "mode": format!("{:04o}", e.mode & 0o7777), "size": e.size, "sha256": e.sha256})).collect(),
    )
}

pub fn placeholders_note(r: &Rendered) -> Vec<String> {
    let mut v = vec![
        format!(
            "The script calls back to {} (port {}, log port {}, syslog {}); a real run picks this computer's address towards the NAS and free ports.",
            r.bind_ip, PLACEHOLDER_HTTP_PORT, PLACEHOLDER_TCP_PORT, PLACEHOLDER_UDP_PORT
        ),
        "The password hash and the SSH key are placeholders in this output; a real run uses yours.".into(),
    ];
    if let Some(b) = &r.bundle {
        v.push(format!("OFFLINE: packages and installer files come from the bundle {b}, served by this computer at {} (mirror host/dir are rewritten accordingly).", r.http_base));
    }
    v
}

/// The whole dry run as one JSON document.
pub fn to_json(facts: &DeviceFacts, plan: &Plan, method: &str, r: &Rendered) -> Value {
    json!({
        "schema": SCHEMA,
        "dry_run": true,
        "model": facts.model,
        "confirm_phrase": confirm_phrase(facts),
        "method": method,
        "placeholders": placeholders_note(r),
        "plan": plan,
        "install_config": r.cfg_json,
        "installer_cmdline": r.cmdline,
        "offline_bundle": r.bundle,
        "payload": {"sha256": r.payload_sha256, "bytes": r.payload.len(), "files": manifest_json(&r.manifest)},
        "preseed": r.preseed,
        "stage2": r.stage2,
    })
}

/// The dry run as printable sections (stdout).
pub fn to_text(plan_summary: &[String], r: &Rendered, method: &str) -> Vec<String> {
    let mut v = vec!["===== DRY RUN: nothing was changed on the NAS and nothing was staged =====".to_string()];
    v.extend(placeholders_note(r));
    v.push(String::new());
    v.push(format!("----- install method: {method} -----"));
    v.push(String::new());
    v.push("----- plan -----".into());
    v.extend(plan_summary.iter().cloned());
    v.push(String::new());
    v.push(format!("----- installer kernel command line -----\n{}", r.cmdline));
    v.push(String::new());
    v.push(format!("----- payload manifest (payload.tar.gz, {} bytes, sha256 {}) -----", r.payload.len(), r.payload_sha256));
    for e in &r.manifest {
        v.push(format!("{:04o} {:>8}  {}  {}", e.mode & 0o7777, e.size, &e.sha256[..16], e.path));
    }
    v.push(String::new());
    v.push("----- plan.json -----".into());
    v.push(r.plan_json.clone());
    v.push(String::new());
    v.push("----- preseed.cfg -----".into());
    v.push(r.preseed.trim_end().to_string());
    v.push(String::new());
    v.push("----- stage2.sh -----".into());
    v.push(r.stage2.trim_end().to_string());
    v.push(String::new());
    v.push("===== end of dry run =====".into());
    v
}

/// Write the artefacts below `dir`: stage2.sh, preseed.cfg, plan.json, install-config.json, payload.tar.gz, payload-manifest.json and the
/// extracted payload/ tree. Refuses a non-empty directory unless `overwrite`.
pub fn write_to(dir: &Path, r: &Rendered, overwrite: bool) -> CliResult<Vec<String>> {
    if dir.exists() {
        let non_empty = std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true);
        if non_empty && !overwrite {
            return Err(CliError::usage(format!(
                "{} is not empty; choose another --out directory or pass --yes to overwrite",
                dir.display()
            )));
        }
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut written = vec![];
    let mut put = |name: &str, data: &[u8]| -> Result<()> {
        std::fs::write(dir.join(name), data).with_context(|| format!("writing {name}"))?;
        written.push(name.to_string());
        Ok(())
    };
    put("stage2.sh", r.stage2.as_bytes())?;
    put("preseed.cfg", r.preseed.as_bytes())?;
    put("plan.json", r.plan_json.as_bytes())?;
    put("install-config.json", serde_json::to_string_pretty(&r.cfg_json)?.as_bytes())?;
    put("payload.tar.gz", &r.payload)?;
    put("payload-manifest.json", serde_json::to_string_pretty(&manifest_json(&r.manifest))?.as_bytes())?;
    put("installer-cmdline.txt", format!("{}\n", r.cmdline).as_bytes())?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&r.payload[..]));
    let root = dir.join("payload");
    for e in ar.entries().map_err(anyhow::Error::from)? {
        let mut e = e.map_err(anyhow::Error::from)?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let rel = e.path().map_err(anyhow::Error::from)?.to_path_buf();
        if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return Err(anyhow::anyhow!("unsafe path in the generated payload: {}", rel.display()).into());
        }
        let dest = root.join(&rel);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p).map_err(anyhow::Error::from)?;
        }
        e.unpack(&dest).map_err(anyhow::Error::from)?;
    }
    written.push("payload/ (extracted)".into());
    Ok(written)
}

/// How the dry run is delivered: a JSON document (`--json`) and/or a directory (`--out`).
#[derive(Clone, Copy, Default)]
pub struct Delivery<'a> {
    pub json_doc: bool,
    pub dir: Option<&'a Path>,
}

/// Print or write the dry run, as the flags ask.
pub fn deliver(
    out: &Out,
    how: Delivery<'_>,
    facts: &DeviceFacts,
    plan: &Plan,
    plan_lines: &[String],
    method: &str,
    r: &Rendered,
) -> CliResult<()> {
    let json_doc = how.json_doc;
    if let Some(d) = how.dir {
        let files = write_to(d, r, out.yes)?;
        if json_doc || out.json_events() {
            let doc = json!({"schema": SCHEMA, "dry_run": true, "out_dir": d.display().to_string(), "files": files, "payload_sha256": r.payload_sha256});
            if out.json_events() {
                out.result_event("install", true, doc);
            } else {
                out.json_doc(&doc);
            }
        } else {
            out.data(&format!("Dry run written to {}", d.display()));
            for f in files {
                out.data(&format!("  {f}"));
            }
            out.data("Nothing was changed on the NAS.");
        }
        return Ok(());
    }
    if out.json_events() {
        out.result_event("install", true, to_json(facts, plan, method, r));
    } else if json_doc {
        out.json_doc(&to_json(facts, plan, method, r));
    } else {
        out.data_lines(to_text(plan_lines, r, method));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::facts::demo_facts;
    use super::super::out::Out;
    use super::super::spec::{InstallSpec, build_material};
    use super::*;
    use synofurb_core::plan::{PlanRequest, build_plan};

    fn rendered() -> (DeviceFacts, Plan, Rendered) {
        let (facts, raw) = demo_facts();
        let plan = build_plan(&facts, &PlanRequest::default());
        let mat = build_material(&InstallSpec::defaults(), &facts, true, &Out::plain()).unwrap();
        let r = render(&mat.cfg, &plan, &facts, &raw, bind_ip_for(None), None).unwrap();
        (facts, plan, r)
    }

    #[test]
    fn renders_script_preseed_payload_and_manifest() {
        let (_, _, r) = rendered();
        assert!(r.stage2.starts_with("#!/bin/sh") && r.stage2.contains("POINT_OF_NO_RETURN"));
        assert!(r.preseed.contains("d-i partman/early_command string wget") && r.preseed.contains("192.0.2.10:18080/DRYRUN/stage2.sh"));
        assert!(r.manifest.iter().any(|e| e.path == "env.sh" && e.mode == 0o600));
        assert!(r.manifest.iter().any(|e| e.path == "plan.json") && r.manifest.iter().any(|e| e.path == "chroot-setup.sh"));
        assert!(r.manifest.iter().all(|e| e.sha256.len() == 64));
        assert_eq!(r.payload_sha256.len(), 64);
        assert!(r.cmdline.contains("console=ttyS0,115200n8") && r.cmdline.contains("log_port=18082"));
    }

    #[test]
    fn no_secret_material_in_a_dry_run() {
        let (_, _, r) = rendered();
        let env_text = {
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&r.payload[..]));
            let mut s = String::new();
            for e in ar.entries().unwrap() {
                let mut e = e.unwrap();
                if e.path().unwrap().to_string_lossy() == "env.sh" {
                    e.read_to_string(&mut s).unwrap();
                }
            }
            s
        };
        assert!(env_text.contains("DRYRUNSALT"), "placeholder hash only: {env_text}");
        assert!(!r.stage2.contains("PRIVATE KEY") && !r.cfg_json.to_string().contains("PRIVATE KEY"));
    }

    #[test]
    fn plan_errors_cannot_be_rendered() {
        let (facts, raw) = demo_facts();
        let mut plan = build_plan(&facts, &PlanRequest::default());
        plan.errors.push("boom".into());
        let mat = build_material(&InstallSpec::defaults(), &facts, true, &Out::plain()).unwrap();
        assert!(render(&mat.cfg, &plan, &facts, &raw, bind_ip_for(None), None).is_err());
    }

    #[test]
    fn writes_a_reviewable_tree_and_refuses_to_clobber() {
        let (_, _, r) = rendered();
        let dir = std::env::temp_dir().join(format!("sf-dry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = write_to(&dir, &r, false).unwrap();
        assert!(files.contains(&"stage2.sh".to_string()));
        for f in [
            "stage2.sh",
            "preseed.cfg",
            "plan.json",
            "payload.tar.gz",
            "payload-manifest.json",
            "payload/env.sh",
            "payload/chroot-setup.sh",
        ] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        assert_eq!(write_to(&dir, &r, false).err().unwrap().code(), 2, "non-empty directory is refused");
        assert!(write_to(&dir, &r, true).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_and_text_forms() {
        let (facts, plan, r) = rendered();
        let v = to_json(&facts, &plan, "kexec", &r);
        assert_eq!((v["dry_run"].as_bool(), v["schema"].as_u64(), v["method"].as_str()), (Some(true), Some(1), Some("kexec")));
        assert!(v["stage2"].as_str().unwrap().starts_with("#!/bin/sh"));
        assert!(v["payload"]["files"].as_array().unwrap().len() > 10);
        let t = to_text(&["x".into()], &r, "kexec").join("\n");
        for s in ["DRY RUN", "payload manifest", "preseed.cfg", "stage2.sh", "plan.json"] {
            assert!(t.contains(s), "{s}");
        }
    }
}
