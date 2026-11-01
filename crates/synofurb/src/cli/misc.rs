//! `config`, `version`, `completions`.

use super::Program;
use super::args::{ConfigCmd, ShellArg};
use super::exit::CliResult;
use super::facts::{SCHEMA, config_dir, load_compat_db};
use super::logs_cmd::logs_dir;
use super::out::Out;
use super::scan::inventory_path;
use serde_json::{Value, json};
use synofurb_core::compat::DbOrigin;
use synofurb_core::discovery::Inventory;
use synofurb_core::session::{config_keys_dir, data_dir};

pub fn paths_json() -> Value {
    let data = data_dir();
    let update_cache = directories::ProjectDirs::from("", "", "synofurb").map(|d| d.cache_dir().join("updates"));
    json!({
        "schema": SCHEMA,
        "data_dir": data.display().to_string(),
        "logs": logs_dir().display().to_string(),
        "inventory": inventory_path().display().to_string(),
        "keys": config_keys_dir().display().to_string(),
        "dom_backups": data.join("dom-backups").display().to_string(),
        "disk_metadata_backups": data.join("backups").display().to_string(),
        "cache": data.join("cache").display().to_string(),
        "compat_cache": data.join("compat-models.json").display().to_string(),
        "ui_settings": data.join("ui-settings.json").display().to_string(),
        "update_cache": update_cache.map(|p| p.display().to_string()),
    })
}

fn env_state(name: &str, secret: bool) -> Value {
    match std::env::var(name) {
        Ok(v) if secret => json!(if v.is_empty() { "set (empty)" } else { "set" }),
        Ok(v) => json!(v),
        Err(_) => Value::Null,
    }
}

/// The update channel the desktop app saved (`stable` when it never ran or the file is unreadable).
pub fn saved_update_channel() -> String {
    std::fs::read_to_string(config_dir().join("ui-settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["update_channel"].as_str().map(String::from))
        .unwrap_or_else(|| "stable".into())
}

pub fn show_json(out: &Out, p: &Program) -> Value {
    let db = load_compat_db();
    let inv = Inventory::load(&inventory_path());
    json!({
        "schema": SCHEMA,
        "program": p.name,
        "version": env!("CARGO_PKG_VERSION"),
        "executable": std::env::current_exe().ok().map(|e| e.display().to_string()),
        "paths": paths_json(),
        "update_channel": saved_update_channel(),
        "compat_list": {"models": db.len(), "updated": db.updated(), "origin": if db.origin() == DbOrigin::Cache { "cache" } else { "embedded" }},
        "inventory_devices": inv.devices.len(),
        "terminal": {"stdin_tty": std::io::IsTerminal::is_terminal(&std::io::stdin()), "stderr_tty": out.tty_err, "color": out.color_err, "interactive": out.interactive},
        "environment": {
            "SYNOFURB_PASSWORD": env_state("SYNOFURB_PASSWORD", true),
            "SYNOFURB_SUDO_PASSWORD": env_state("SYNOFURB_SUDO_PASSWORD", true),
            "SYNOFURB_AGENT_DIR": env_state("SYNOFURB_AGENT_DIR", false),
            "SYNOFURB_LOG_FORMAT": env_state("SYNOFURB_LOG_FORMAT", false),
            "NO_COLOR": env_state("NO_COLOR", false),
            "CI": env_state("CI", false),
            "TERM": env_state("TERM", false),
            "XDG_DATA_HOME": env_state("XDG_DATA_HOME", false),
        },
    })
}

pub fn config(out: &Out, p: &Program, action: ConfigCmd) -> CliResult<()> {
    match action {
        ConfigCmd::Paths { json } => {
            let v = paths_json();
            if json {
                out.json_doc(&v);
            } else {
                for (k, val) in v.as_object().into_iter().flatten().filter(|(k, _)| *k != "schema") {
                    out.data(&format!("{k:<22} {}", val.as_str().unwrap_or("-")));
                }
            }
        }
        ConfigCmd::Show { json } => {
            let v = show_json(out, p);
            if json {
                out.json_doc(&v);
            } else {
                out.data(&format!("{} {}  ({})", p.name, env!("CARGO_PKG_VERSION"), v["executable"].as_str().unwrap_or("?")));
                out.data(&format!("update channel          {}", v["update_channel"].as_str().unwrap_or("stable")));
                out.data(&format!(
                    "compatibility list      {} models, dated {}, {} copy",
                    v["compat_list"]["models"],
                    v["compat_list"]["updated"].as_str().unwrap_or(""),
                    v["compat_list"]["origin"].as_str().unwrap_or("")
                ));
                out.data(&format!("remembered units        {}", v["inventory_devices"]));
                out.data("");
                out.data("Paths");
                for (k, val) in v["paths"].as_object().into_iter().flatten().filter(|(k, _)| *k != "schema") {
                    out.data(&format!("  {k:<22} {}", val.as_str().unwrap_or("-")));
                }
                out.data("");
                out.data("Environment (secrets are only reported as set/unset)");
                for (k, val) in v["environment"].as_object().into_iter().flatten() {
                    out.data(&format!("  {k:<24} {}", val.as_str().unwrap_or("(unset)")));
                }
                out.data("");
                out.data(&format!(
                    "Terminal: stdin tty {}, stderr tty {}, colour {}, may prompt {}",
                    v["terminal"]["stdin_tty"], v["terminal"]["stderr_tty"], v["terminal"]["color"], v["terminal"]["interactive"]
                ));
            }
        }
    }
    Ok(())
}

pub fn version_json(p: &Program) -> Value {
    let git = env!("SYNOFURB_GIT_DESCRIBE_BUILD");
    let mut features = vec!["cli", "bundle"];
    if p.gui {
        features.push("gui");
    }
    json!({
        "schema": SCHEMA,
        "name": p.name,
        "version": env!("CARGO_PKG_VERSION"),
        "git": if git.is_empty() { Value::Null } else { json!(git) },
        "target": env!("SYNOFURB_TARGET"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "profile": env!("SYNOFURB_PROFILE"),
        "features": features,
        "gui": p.gui,
    })
}

pub fn version_line(p: &Program) -> String {
    let v = version_json(p);
    format!(
        "{} {}{} ({}) features: {}",
        p.name,
        env!("CARGO_PKG_VERSION"),
        v["git"].as_str().map(|g| format!(" [{g}]")).unwrap_or_default(),
        env!("SYNOFURB_TARGET"),
        v["features"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(",")
    )
}

pub fn version(out: &Out, p: &Program, json: bool) -> CliResult<()> {
    if json {
        out.json_doc(&version_json(p));
    } else {
        out.data(&version_line(p));
    }
    Ok(())
}

pub fn completions(p: &Program, shell: ShellArg) -> CliResult<()> {
    use clap_complete::Shell;
    let sh = match shell {
        ShellArg::Bash => Shell::Bash,
        ShellArg::Zsh => Shell::Zsh,
        ShellArg::Fish => Shell::Fish,
        ShellArg::Powershell => Shell::PowerShell,
    };
    let mut cmd = (p.command)();
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::generate(sh, &mut cmd, p.name, &mut buf);
    use std::io::Write;
    let mut so = std::io::stdout().lock();
    if so.write_all(&buf).and_then(|_| so.flush()).is_err() {
        std::process::exit(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn prog() -> Program {
        Program { name: "synofurb-cli", gui: false, command: super::super::args::Cli::command }
    }

    #[test]
    fn version_document() {
        let v = version_json(&prog());
        assert_eq!(v["name"], "synofurb-cli");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert!(v["target"].as_str().unwrap().contains('-'), "{v}");
        assert_eq!(v["gui"], false);
        assert!(v["features"].as_array().unwrap().iter().any(|f| f == "cli"));
        assert!(!v["features"].as_array().unwrap().iter().any(|f| f == "gui"));
        assert!(version_line(&prog()).starts_with("synofurb-cli "));
    }

    #[test]
    fn paths_document_has_every_location() {
        let v = paths_json();
        for k in ["data_dir", "logs", "inventory", "keys", "dom_backups", "disk_metadata_backups", "cache", "compat_cache", "ui_settings"] {
            assert!(v[k].as_str().is_some_and(|s| !s.is_empty()), "{k}");
        }
        assert!(v["logs"].as_str().unwrap().ends_with("logs"));
    }

    #[test]
    fn secrets_are_never_reported_by_value() {
        assert_eq!(env_state("SYNOFURB_SURELY_UNSET_VAR", true), Value::Null);
        // PATH is always set; as a "secret" it must not leak
        assert_eq!(env_state("PATH", true), json!("set"));
    }
}
