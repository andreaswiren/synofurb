//! `--addon ID[:KEY=VALUE,...]` on `plan` and `install`, and `addons list`.
//!
//! Secret settings never travel on the command line: `KEY=@env:VAR` reads an environment variable, `KEY=@file:PATH` the first line of a
//! file (like `--password-env` / `--password-file`). A literal value for a secret setting is refused, because it would sit in the
//! process list and the shell history.

use super::args::AddonsCmd;
use super::exit::{CliError, CliResult};
use super::facts::SCHEMA;
use super::out::Out;
use serde_json::json;
use std::collections::BTreeMap;
use synofurb_core::addons::{self, AddonChoice, AddonContext, SettingKind};

/// Where a secret value is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    Env(String),
    File(String),
}

/// One `--addon` argument, parsed but with secrets not yet read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAddon {
    pub id: String,
    pub settings: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, SecretSource>,
}

fn key_ok(k: &str) -> bool {
    k.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `ID` or `ID:KEY=VALUE,KEY=VALUE`. Values cannot contain a comma.
pub fn parse_addon_spec(text: &str) -> CliResult<ParsedAddon> {
    let t = text.trim();
    let (id, rest) = match t.split_once(':') {
        Some((i, r)) => (i, Some(r)),
        None => (t, None),
    };
    if id.is_empty() {
        return Err(CliError::usage("--addon needs an add-on id (see `addons list`), for example --addon portainer"));
    }
    let mut p = ParsedAddon { id: id.to_string(), settings: BTreeMap::new(), secrets: BTreeMap::new() };
    let Some(rest) = rest else { return Ok(p) };
    if rest.is_empty() {
        return Err(CliError::usage(format!("--addon {t}: nothing after ':' (write {id}:KEY=VALUE)")));
    }
    for pair in rest.split(',') {
        let Some((k, v)) = pair.split_once('=') else {
            return Err(CliError::usage(format!("--addon {id}: '{pair}' is not KEY=VALUE (values cannot contain a comma)")));
        };
        if !key_ok(k) {
            return Err(CliError::usage(format!("--addon {id}: '{k}' is not a setting name (lowercase letters, digits, '_')")));
        }
        if p.settings.contains_key(k) || p.secrets.contains_key(k) {
            return Err(CliError::usage(format!("--addon {id}: '{k}' is given twice")));
        }
        if let Some(var) = v.strip_prefix("@env:") {
            if var.is_empty() {
                return Err(CliError::usage(format!("--addon {id}: '{k}=@env:' needs a variable name")));
            }
            p.secrets.insert(k.to_string(), SecretSource::Env(var.to_string()));
        } else if let Some(file) = v.strip_prefix("@file:") {
            if file.is_empty() {
                return Err(CliError::usage(format!("--addon {id}: '{k}=@file:' needs a file name")));
            }
            p.secrets.insert(k.to_string(), SecretSource::File(file.to_string()));
        } else {
            p.settings.insert(k.to_string(), v.to_string());
        }
    }
    Ok(p)
}

fn is_secret_key(id: &str, key: &str) -> Option<bool> {
    let e = addons::entry(id)?;
    e.settings.iter().find(|s| s.key == key).map(|s| matches!(s.kind, SettingKind::Secret { .. }))
}

fn read_secret(id: &str, key: &str, src: &SecretSource) -> CliResult<String> {
    let v = match src {
        SecretSource::Env(var) => std::env::var(var)
            .map_err(|_| CliError::usage(format!("--addon {id}:{key}=@env:{var}: that environment variable is not set")))?,
        SecretSource::File(path) => {
            let t = std::fs::read_to_string(path).map_err(|e| CliError::usage(format!("--addon {id}:{key}=@file:{path}: {e}")))?;
            t.lines().next().unwrap_or("").trim_end_matches('\r').to_string()
        }
    };
    let v = v.trim().to_string();
    if v.is_empty() {
        return Err(CliError::usage(format!("--addon {id}:{key}: the secret is empty")));
    }
    Ok(v)
}

/// Parse and check the `--addon` arguments: secrets are read from their environment variable or file, and the whole choice is validated
/// against the catalog (unknown add-on or setting, bad value, secret given inline). Dependencies are not added here.
pub fn addons_from_args(specs: &[String]) -> CliResult<Vec<AddonChoice>> {
    let mut v = vec![];
    for s in specs {
        let p = parse_addon_spec(s)?;
        let mut c = AddonChoice::new(&p.id);
        c.settings = p.settings.clone();
        for (k, src) in &p.secrets {
            match is_secret_key(&p.id, k) {
                Some(true) => {
                    c.secrets.insert(k.clone(), read_secret(&p.id, k, src)?);
                }
                Some(false) => {
                    return Err(CliError::usage(format!(
                        "--addon {}: '{k}' is not a secret setting; give its value directly ({k}=VALUE)",
                        p.id
                    )));
                }
                None => {} // unknown id or key: the catalog check below says which
            }
        }
        for k in p.settings.keys() {
            if is_secret_key(&p.id, k) == Some(true) {
                return Err(CliError::usage(format!(
                    "--addon {}: '{k}' is a secret and cannot be given on the command line (it would show in the process list and shell history). Use {k}=@env:VARIABLE or {k}=@file:PATH",
                    p.id
                )));
            }
        }
        v.push(c);
    }
    addons::validate(&v).map_err(|e| CliError::usage(format!("{e:#}")))?;
    Ok(v)
}

/// The lines the plan and the dry run add for the chosen add-ons (what is installed, then the things to know).
pub fn plan_text(choices: &[AddonChoice], ctx: &AddonContext) -> Vec<String> {
    let mut v = addons::plan_lines(choices);
    if v.is_empty() {
        return v;
    }
    v.extend(addons::warnings(choices, ctx).into_iter().map(|w| format!("Note: {w}")));
    v
}

pub fn addons(out: &Out, action: AddonsCmd) -> CliResult<()> {
    match action {
        AddonsCmd::List { json } => {
            if json {
                out.json_doc(&list_json());
            } else {
                out.data_lines(list_text());
            }
            Ok(())
        }
    }
}

pub fn list_json() -> serde_json::Value {
    json!({
        "schema": SCHEMA,
        "addons": addons::catalog(),
        "presets": addons::PRESETS,
        "secret_syntax": "KEY=@env:VARIABLE or KEY=@file:PATH (a secret is never written on the command line)",
        "flag": "--addon ID[:KEY=VALUE,...] (repeatable)",
    })
}

pub fn list_text() -> Vec<String> {
    let mut v = vec![
        "Add-ons the installer can set up after the first boot (Docker is added when another add-on needs it):".to_string(),
        String::new(),
    ];
    for e in addons::catalog() {
        v.push(format!("  {:<10} {}", e.id, e.name));
        v.push(format!("             {}", e.summary));
        let mut facts = vec![];
        if !e.requires.is_empty() {
            facts.push(format!("needs {}", e.requires.join(", ")));
        }
        if !e.privileges.is_empty() {
            facts.push(format!("privileges {}", e.privileges.join(", ")));
        }
        if e.needs_internet {
            facts.push("Internet at first boot".into());
        }
        facts.push(format!("about {:.1} GiB", e.approx_size_mib as f64 / 1024.0));
        if e.data_gib > 0 {
            facts.push(format!("{} GiB free on the data volume", e.data_gib));
        }
        v.push(format!("             {}", facts.join("; ")));
        for s in e.settings {
            let how = match s.kind {
                SettingKind::Int { min, max } => format!("whole number {min}-{max}, default {}", s.default),
                SettingKind::Text { max_len } => format!("text up to {max_len} characters"),
                SettingKind::Secret { .. } => format!("secret: {}=@env:VARIABLE or {}=@file:PATH", s.key, s.key),
            };
            v.push(format!("             setting {}: {} ({how})", s.key, s.label));
        }
    }
    v.push(String::new());
    v.push("Examples:".into());
    v.push("  --addon portainer                       Docker and Portainer".into());
    v.push("  --addon frigate:retain_days=14          Frigate NVR (Docker is added)".into());
    v.push("  --addon plex:plex_claim=@env:PLEX_CLAIM Plex with a claim token read from $PLEX_CLAIM".into());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &[&str]) -> Vec<String> {
        x.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn spec_syntax() {
        let p = parse_addon_spec("frigate:retain_days=14").unwrap();
        assert_eq!((p.id.as_str(), p.settings["retain_days"].as_str()), ("frigate", "14"));
        assert_eq!(parse_addon_spec("docker").unwrap().settings.len(), 0);
        let p = parse_addon_spec("plex:plex_claim=@env:PLEX_CLAIM").unwrap();
        assert_eq!(p.secrets["plex_claim"], SecretSource::Env("PLEX_CLAIM".into()));
        let p = parse_addon_spec("plex:plex_claim=@file:/run/secrets/claim,x=1").unwrap();
        assert_eq!(p.secrets["plex_claim"], SecretSource::File("/run/secrets/claim".into()));
        assert_eq!(p.settings["x"], "1");
        for bad in [
            "",
            ":x=1",
            "frigate:",
            "frigate:retain_days",
            "frigate:Retain=1",
            "frigate:a=1,a=2",
            "plex:k=@env:",
            "plex:k=@file:",
            "a:b=1,,c=2",
        ] {
            assert_eq!(parse_addon_spec(bad).err().unwrap().code(), 2, "{bad:?}");
        }
    }

    #[test]
    fn resolution_and_validation() {
        let c = addons_from_args(&s(&["portainer", "frigate:retain_days=14"])).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].settings["retain_days"], "14");
        // unknown add-on / setting, bad value: usage errors that list the choices
        let e = addons_from_args(&s(&["nope"])).err().unwrap();
        assert_eq!(e.code(), 2);
        assert!(e.to_string().contains("available: docker"), "{e}");
        assert!(addons_from_args(&s(&["frigate:detector=x"])).err().unwrap().to_string().contains("available: retain_days"));
        assert_eq!(addons_from_args(&s(&["frigate:retain_days=0"])).err().unwrap().code(), 2);
        assert_eq!(addons_from_args(&s(&["docker", "docker"])).err().unwrap().code(), 2);
    }

    #[test]
    fn secrets_only_via_env_or_file_never_inline() {
        let inline = addons_from_args(&s(&["plex:plex_claim=claim-abcdefgh12345"])).err().unwrap();
        assert_eq!(inline.code(), 2);
        let msg = inline.to_string();
        assert!(msg.contains("@env:") && msg.contains("process list") && !msg.contains("abcdefgh12345"), "{msg}");
        let e = addons_from_args(&s(&["plex:plex_claim=@env:SYNOFURB_TEST_UNSET_CLAIM_VAR"])).err().unwrap();
        assert!(e.to_string().contains("not set"));
        let e = addons_from_args(&s(&["frigate:retain_days=@env:X"])).err().unwrap();
        assert!(e.to_string().contains("not a secret"));
        // from a file (first line)
        let f = std::env::temp_dir().join(format!("sf-claim-{}", std::process::id()));
        std::fs::write(&f, "claim-FromFile12345\nsecond line\n").unwrap();
        let c = addons_from_args(&s(&[&format!("plex:plex_claim=@file:{}", f.display())])).unwrap();
        assert_eq!(c[0].secrets["plex_claim"], "claim-FromFile12345");
        assert!(c[0].settings.is_empty());
        // an invalid secret is refused without echoing it
        std::fs::write(&f, "not-a-claim\n").unwrap();
        let e = addons_from_args(&s(&[&format!("plex:plex_claim=@file:{}", f.display())])).err().unwrap();
        assert_eq!(e.code(), 2);
        assert!(!e.to_string().contains("not-a-claim"), "{e}");
        std::fs::write(&f, "\n").unwrap();
        assert!(addons_from_args(&s(&[&format!("plex:plex_claim=@file:{}", f.display())])).err().unwrap().to_string().contains("empty"));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn list_outputs() {
        let t = list_text().join("\n");
        for need in [
            "docker",
            "portainer",
            "frigate",
            "plex",
            "retain_days",
            "@env:VARIABLE",
            "docker-socket",
            "host-network",
            "Internet at first boot",
        ] {
            assert!(t.contains(need), "{need}\n{t}");
        }
        let j = list_json();
        assert_eq!(j["addons"].as_array().unwrap().len(), addons::catalog().len());
        assert_eq!(j["addons"][0]["id"], "docker");
        assert!(j["presets"].as_array().unwrap().iter().any(|p| p["label"] == "Frigate NVR"));
        assert!(j["schema"].is_number());
    }

    #[test]
    fn plan_text_lists_add_ons_and_notes() {
        let c = addons_from_args(&s(&["portainer"])).unwrap();
        let t = plan_text(&c, &AddonContext::default()).join("\n");
        assert!(
            t.contains("Add-ons: Docker (required by Portainer), Portainer") && t.contains("Note:") && t.contains("docker-socket"),
            "{t}"
        );
        assert!(plan_text(&[], &AddonContext::default()).is_empty());
    }
}
