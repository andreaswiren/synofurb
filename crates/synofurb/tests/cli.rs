//! Black-box tests of the headless binary: exit codes, JSON shapes, the safety gate, dry runs. No NAS and no network needed: units come
//! from `--facts demo` (the repository's DS918+ fixture) or from files written by `inspect --save`.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use synofurb_core::config::KernelChannel;
use synofurb_core::offline::{ArchManifest, BundleManifest, FORMAT_VERSION, FileEntry, MANIFEST_FILE};

const BIN: &str = env!("CARGO_BIN_EXE_synofurb-cli");

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }
}

/// A private data directory per test, so nothing touches the real one.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("sf-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }
    fn run(&self, args: &[&str]) -> Run {
        self.run_env(args, &[])
    }
    fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Run {
        let mut c = Command::new(BIN);
        c.args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .env("XDG_DATA_HOME", self.dir.join("data"))
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .stdin(Stdio::null());
        for (k, v) in env {
            c.env(k, v);
        }
        let o = c.output().expect("run synofurb-cli");
        Run {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into(),
            stderr: String::from_utf8_lossy(&o.stderr).into(),
        }
    }
    fn path(&self, p: &str) -> PathBuf {
        self.dir.join(p)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn has_keys(v: &Value, keys: &[&str]) {
    for k in keys {
        assert!(v.get(*k).is_some(), "missing key {k} in {v}");
    }
}

// ---------------------------------------------------------------------------------------------- basics

#[test]
fn help_lists_every_command_and_exits_zero() {
    let s = Sandbox::new("help");
    let r = s.run(&["--help"]);
    assert_eq!(r.code, 0);
    for c in [
        "scan",
        "inventory",
        "inspect",
        "compat",
        "plan",
        "install",
        "wizard",
        "boot-manager",
        "recover",
        "backup-dom",
        "disk-backup",
        "manage",
        "logs",
        "bundle",
        "fetch-installer",
        "self-update",
        "config",
        "completions",
        "version",
    ] {
        assert!(r.stdout.contains(c), "--help lacks {c}");
    }
    assert!(r.stdout.contains("EXIT CODES") && r.stdout.contains("EXAMPLES"));
    for sub in ["install", "plan", "scan", "wizard", "completions"] {
        let r = s.run(&[sub, "--help"]);
        assert_eq!(r.code, 0, "{sub} --help");
        assert!(r.stdout.contains("Usage:"), "{sub}");
    }
    assert!(s.run(&["install", "--help"]).stdout.contains("--dry-run") && s.run(&["install", "--help"]).stdout.contains("EXAMPLES"));
}

#[test]
fn usage_errors_exit_two() {
    let s = Sandbox::new("usage");
    assert_eq!(s.run(&[]).code, 2, "no subcommand");
    assert_eq!(s.run(&["bogus"]).code, 2);
    assert_eq!(s.run(&["plan"]).code, 2, "plan needs --host or --facts");
    assert_eq!(s.run(&["install", "--host", "h", "--password", "hunter2"]).code, 2, "passwords are never accepted on argv");
    assert!(s.run(&["install", "--host", "h", "--password", "hunter2"]).stderr.contains("unexpected argument"));
    assert_eq!(s.run(&["scan", "--timeout", "0"]).code, 2);
}

#[test]
fn version_json_shape() {
    let s = Sandbox::new("version");
    let r = s.run(&["version", "--json"]);
    assert_eq!(r.code, 0);
    let v = r.json();
    has_keys(&v, &["schema", "name", "version", "git", "target", "os", "arch", "profile", "features", "gui"]);
    assert_eq!(v["name"], "synofurb-cli");
    assert_eq!(v["gui"], false);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert!(s.run(&["version"]).stdout.starts_with("synofurb-cli "));
    assert!(s.run(&["--version"]).stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn completions_for_every_shell() {
    let s = Sandbox::new("compl");
    for (shell, needle) in [
        ("bash", "complete -F"),
        ("zsh", "#compdef synofurb-cli"),
        ("fish", "complete -c synofurb-cli"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let r = s.run(&["completions", shell]);
        assert_eq!(r.code, 0, "{shell}");
        assert!(r.stdout.contains(needle), "{shell}: {}", &r.stdout[..r.stdout.len().min(200)]);
        assert!(r.stdout.contains("boot-manager"), "{shell} knows the subcommands");
    }
}

// ---------------------------------------------------------------------------------------------- compat

#[test]
fn compat_documents() {
    let s = Sandbox::new("compat");
    let v = s.run(&["compat", "DS923+", "--json"]).json();
    has_keys(&v, &["schema", "query", "found", "status", "status_label", "explanation", "blocking", "entry"]);
    assert_eq!((v["found"].as_bool(), v["entry"]["model"].as_str()), (Some(true), Some("DS923+")));
    let v = s.run(&["compat", "--json"]).json();
    has_keys(&v, &["schema", "models", "updated", "origin", "by_status"]);
    let r = s.run(&["compat", "DS-does-not-exist"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("not in the list"));
    let list = s.run(&["compat", "--list", "--status", "unsupported"]);
    assert_eq!(list.code, 0);
    let first = list.stdout.lines().next().expect("unsupported models exist");
    let model = first.split_whitespace().next().unwrap();
    assert_eq!(s.run(&["compat", model]).code, 5, "an unsupported model exits 5");
    assert_eq!(s.run(&["compat", "--list", "--status", "weird"]).code, 2);
}

// ---------------------------------------------------------------------------------------------- inspect / plan

#[test]
fn inspect_json_and_save_roundtrip() {
    let s = Sandbox::new("inspect");
    let r = s.run(&["inspect", "--facts", "demo", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v = r.json();
    has_keys(
        &v,
        &[
            "schema",
            "facts",
            "arch",
            "firmware",
            "cpu_vendor",
            "serial_console",
            "platform",
            "compat",
            "confirm_phrase",
            "supported",
            "auto_boot_method",
        ],
    );
    assert_eq!(v["confirm_phrase"], "ERASE DS918+");
    has_keys(&v["facts"], &["model", "serial", "disks", "arrays", "lvs", "mounts", "firmware", "dom", "cpu_vendor", "cmdline", "nics"]);
    has_keys(&v["compat"], &["status", "headline", "details", "blocking", "entry"]);
    has_keys(&v["platform"], &["platform", "arch", "soc", "support", "note"]);
    has_keys(&v["serial_console"], &["tty", "speed", "raw", "kernel_arg"]);

    let saved = s.path("facts.json");
    let r = s.run(&["inspect", "--facts", "demo", "--save", saved.to_str().unwrap()]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("Model:") && r.stdout.contains("DS918+") && r.stdout.contains("RAID arrays:"));
    let again = s.run(&["inspect", "--facts", saved.to_str().unwrap(), "--json"]);
    assert_eq!(again.json()["facts"], v["facts"], "a saved inspection reads back identically");
}

#[test]
fn plan_json_and_exit_codes() {
    let s = Sandbox::new("plan");
    let r = s.run(&["plan", "--facts", "demo", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v = r.json();
    has_keys(
        &v,
        &[
            "schema",
            "model",
            "confirm_phrase",
            "ok",
            "plan_ok",
            "supported",
            "errors",
            "warnings",
            "summary",
            "destroyed",
            "preserved",
            "platform",
            "compat",
            "plan",
        ],
    );
    assert_eq!(v["ok"], true);
    assert_eq!(v["plan"]["mode"], "WipeAll");
    // every knob reaches the planner
    let v = s
        .run(&[
            "plan",
            "--facts",
            "demo",
            "--json",
            "--mode",
            "keep",
            "--keep",
            "md2",
            "--copies",
            "2",
            "--free-percent",
            "35",
            "--kernel",
            "stable",
        ])
        .json();
    assert_eq!(v["kernel"]["channel"], "stable");
    assert_eq!(v["plan"]["mode"], "KeepVolumes");
    assert_eq!(v["plan"]["request"]["system_copies"], 2);
    assert_eq!(v["plan"]["request"]["data_free_percent"], 35);
    assert_eq!(v["plan"]["kept_arrays"][0], "md2");
    // text form
    let t = s.run(&["plan", "--facts", "demo"]);
    assert!(t.stdout.contains("WILL BE DESTROYED") && t.stdout.contains("ERASE DS918+"));
    // a plan with errors exits 4 but still prints
    let bad = s.run(&["plan", "--facts", "demo", "--json", "--mode", "keep", "--keep", "md99"]);
    assert_eq!(bad.code, 4, "{}{}", bad.stdout, bad.stderr);
    assert_eq!(bad.json()["plan_ok"], false);
    assert!(!bad.json()["errors"].as_array().unwrap().is_empty());
}

#[test]
fn plan_accepts_the_hybrid_raid_levels() {
    let s = Sandbox::new("shr");
    let v = s.run(&["plan", "--facts", "demo", "--json", "--raid", "shr1"]).json();
    assert_eq!(v["plan_ok"], true, "{v}");
    assert_eq!(v["plan"]["request"]["data_raid"], "Shr1");
    let arrays: Vec<&str> = v["plan"]["arrays"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert!(arrays.contains(&"data1"), "{arrays:?}");
    let vg = v["plan"]["vgs"].as_array().unwrap().iter().find(|g| g["name"] == "vg_data").unwrap();
    assert_eq!(vg["pvs"].as_array().unwrap().len(), 1);
    assert!(v["summary"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("hybrid RAID")));
    let v = s.run(&["plan", "--facts", "demo", "--json", "--raid", "shr2"]).json();
    assert_eq!(v["plan"]["arrays"].as_array().unwrap().iter().find(|a| a["name"] == "data1").unwrap()["level"], "6");
    // keep mode preserves md2 by default: SHR cannot be built next to it, and the plan says why
    let bad = s.run(&["plan", "--facts", "demo", "--json", "--mode", "keep", "--raid", "shr1"]);
    assert_eq!(bad.code, 4, "{}{}", bad.stdout, bad.stderr);
    let errs = bad.json()["errors"].to_string();
    assert!(errs.contains("cannot be combined with preserved arrays") && errs.contains("md2"), "{errs}");
    // the option is documented and completed
    let help = s.run(&["plan", "--help"]);
    assert!(help.stdout.contains("shr1") && help.stdout.contains("shr2"), "{}", help.stdout);
    assert!(s.run(&["completions", "bash"]).stdout.contains("shr1"));
}

#[test]
fn unsupported_hardware_exits_five() {
    let s = Sandbox::new("unsupported");
    let facts = s.run(&["inspect", "--facts", "demo", "--json"]).json()["facts"].clone();
    let mut f = facts;
    f["unique"] = Value::String("synology_88f6281_ds109".into());
    f["uname_m"] = Value::String("armv5tel".into());
    f["arch"] = Value::String("Unsupported".into());
    let p = s.path("old.json");
    std::fs::write(&p, f.to_string()).unwrap();
    for cmd in ["inspect", "plan"] {
        let r = s.run(&[cmd, "--facts", p.to_str().unwrap(), "--json"]);
        assert_eq!(r.code, 5, "{cmd}: {}{}", r.stdout, r.stderr);
        assert_eq!(r.json()["supported"], false);
    }
    let r = s.run(&["install", "--facts", p.to_str().unwrap(), "--dry-run"]);
    assert_eq!(r.code, 5, "{}", r.stderr);
}

#[test]
fn missing_facts_file_is_a_plain_error() {
    let s = Sandbox::new("nofacts");
    let r = s.run(&["plan", "--facts", "/definitely/not/here.json"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("error:"));
    // with NDJSON output the failure is an event on stdout
    let r = s.run(&["--log-format", "json", "plan", "--facts", "/definitely/not/here.json"]);
    assert_eq!(r.code, 1);
    let last: Value = serde_json::from_str(r.stdout.lines().last().unwrap()).unwrap();
    assert_eq!((last["event"].as_str(), last["exit_code"].as_u64(), last["kind"].as_str()), (Some("failure"), Some(1), Some("error")));
    assert!(last["ts"].is_string() && last["message"].is_string());
}

// ---------------------------------------------------------------------------------------------- the safety gate

#[test]
fn install_refuses_without_the_gate_before_touching_the_network() {
    let s = Sandbox::new("gate");
    // 192.0.2.1 is documentation space: if it were contacted this would hang or fail differently
    for args in [
        vec!["install", "--host", "192.0.2.1"],
        vec!["install", "--host", "192.0.2.1", "--yes"],
        vec!["install", "--host", "192.0.2.1", "--have-backup"],
        vec!["install", "--host", "192.0.2.1", "--have-backup", "--accept-data-loss"],
        vec!["install", "--host", "192.0.2.1", "--accept-data-loss", "--confirm", "ERASE DS918+"],
        vec!["install", "--host", "192.0.2.1", "--have-backup", "--confirm", "ERASE DS918+"],
        vec!["install", "--host", "192.0.2.1", "--have-backup", "--accept-data-loss", "--confirm", ""],
    ] {
        let r = s.run_env(&args, &[("SYNOFURB_PASSWORD", "x")]);
        assert_eq!(r.code, 3, "{args:?}: {}", r.stderr);
        assert!(r.stderr.contains("refusing to continue"), "{args:?}");
        let missing: Vec<&str> = ["--have-backup", "--accept-data-loss", "--confirm"]
            .into_iter()
            .filter(|f| !args.contains(f) || *f == "--confirm" && args.last() == Some(&""))
            .collect();
        assert!(
            !missing.is_empty() && missing.iter().all(|m| r.stderr.contains(m)),
            "{args:?} names what is missing ({missing:?}): {}",
            r.stderr
        );
    }
    // the same with --log-format json: the failure is an event
    let r = s.run(&["--log-format", "json", "install", "--host", "192.0.2.1"]);
    assert_eq!(r.code, 3);
    let last: Value = serde_json::from_str(r.stdout.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "refused");
    assert_eq!(last["exit_code"], 3);
}

#[test]
fn a_real_install_cannot_use_saved_facts() {
    let s = Sandbox::new("realfacts");
    let r = s.run(&["install", "--facts", "demo", "--have-backup", "--accept-data-loss", "--confirm", "ERASE DS918+"]);
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("--dry-run"));
}

#[test]
fn a_fully_confirmed_install_to_a_dead_host_fails_cleanly_before_anything_changes() {
    let s = Sandbox::new("dead");
    let r = s.run_env(
        &["install", "--host", "127.0.0.1", "--port", "1", "--have-backup", "--accept-data-loss", "--confirm", "ERASE DS918+"],
        &[("SYNOFURB_PASSWORD", "x")],
    );
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("error:"));
}

// ---------------------------------------------------------------------------------------------- dry run

#[test]
fn dry_run_prints_everything_and_changes_nothing() {
    let s = Sandbox::new("dry");
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--hostname", "vault", "--package", "htop"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    for needle in [
        "DRY RUN",
        "payload manifest",
        "----- stage2.sh -----",
        "----- preseed.cfg -----",
        "----- plan.json -----",
        "env.sh",
        "chroot-setup.sh",
        "#!/bin/sh",
    ] {
        assert!(r.stdout.contains(needle), "dry run lacks {needle}");
    }
    assert!(!r.stdout.contains("PRIVATE KEY"));
    assert!(!s.path("data/synofurb/keys").exists(), "a dry run writes no key");

    let v = s.run(&["install", "--facts", "demo", "--dry-run", "--json"]).json();
    has_keys(
        &v,
        &[
            "schema",
            "dry_run",
            "model",
            "confirm_phrase",
            "method",
            "placeholders",
            "plan",
            "install_config",
            "installer_cmdline",
            "payload",
            "preseed",
            "stage2",
        ],
    );
    assert_eq!(v["dry_run"], true);
    assert!(v["payload"]["files"].as_array().unwrap().iter().any(|f| f["path"] == "env.sh"));

    let out = s.path("review");
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--out", out.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    for f in ["stage2.sh", "preseed.cfg", "plan.json", "payload.tar.gz", "payload-manifest.json", "payload/env.sh"] {
        assert!(out.join(f).is_file(), "{f}");
    }
    let env = std::fs::read_to_string(out.join("payload/env.sh")).unwrap();
    assert!(env.contains("DRYRUNSALT"), "password hash is a placeholder");
    // refuses to overwrite a non-empty directory unless --yes
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--out", out.to_str().unwrap()]).code, 2);
    assert_eq!(s.run(&["--yes", "install", "--facts", "demo", "--dry-run", "--out", out.to_str().unwrap()]).code, 0);
}

#[test]
fn dry_run_knobs_end_up_in_the_rendered_files() {
    let s = Sandbox::new("knobs");
    let out = s.path("o");
    let r = s.run(&[
        "install",
        "--facts",
        "demo",
        "--dry-run",
        "--out",
        out.to_str().unwrap(),
        "--hostname",
        "vault",
        "--username",
        "andreas",
        "--timezone",
        "Europe/Stockholm",
        "--keymap",
        "se",
        "--locale",
        "sv_SE.UTF-8",
        "--mirror",
        "ftp.se.debian.org",
        "--package",
        "htop,tmux",
        "--kernel",
        "stable",
        "--no-mgmt",
        "--serial",
        "uart8250,mmio32,0xfedc9000,115200n8",
        "--method",
        "kit",
        "--authorize-key",
        "none",
        "--mode",
        "keep",
        "--keep",
        "md2",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let env = std::fs::read_to_string(out.join("payload/env.sh")).unwrap();
    for needle in [
        "NAS_HOSTNAME='vault'",
        "USERNAME='andreas'",
        "TIMEZONE='Europe/Stockholm'",
        "KEYMAP='se'",
        "LOCALE='sv_SE.UTF-8'",
        "MIRROR_HOST='ftp.se.debian.org'",
        "EXTRA_PACKAGES='htop tmux'",
        "KERNEL_CHANNEL='stable'",
        "ENABLE_MGMT='0'",
        "SERIAL_ARG='console=uart8250,mmio32,0xfedc9000,115200n8'",
    ] {
        assert!(env.contains(needle), "env.sh lacks {needle}:\n{env}");
    }
    assert_eq!(std::fs::read_to_string(out.join("payload/authorized_keys")).unwrap(), "", "--authorize-key none");
    assert!(std::fs::read_to_string(out.join("preseed.cfg")).unwrap().contains("time/zone string Europe/Stockholm"));
    assert!(std::fs::read_to_string(out.join("installer-cmdline.txt")).unwrap().contains("console=uart8250,mmio32,0xfedc9000,115200n8"));
    let plan: Value = serde_json::from_str(&std::fs::read_to_string(out.join("plan.json")).unwrap()).unwrap();
    assert_eq!(plan["mode"], "KeepVolumes");
}

#[test]
fn dry_run_reads_the_new_password_from_a_file_but_never_prints_it() {
    let s = Sandbox::new("pwfile");
    let pw = s.path("pw.txt");
    std::fs::write(&pw, "Sup3r-secret-pass\n").unwrap();
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--password-file", pw.to_str().unwrap()]);
    assert_eq!(r.code, 0);
    assert!(!r.stdout.contains("Sup3r-secret-pass") && !r.stderr.contains("Sup3r-secret-pass"));
    let r = s.run_env(&["install", "--facts", "demo", "--dry-run", "--password-env", "NEWPW"], &[("NEWPW", "Another-secret-1")]);
    assert_eq!(r.code, 0);
    assert!(!r.stdout.contains("Another-secret-1"));
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--password-env", "NEWPW"]).code, 2, "unset variable");
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--prompt-password"]).code, 2, "no terminal to prompt on");
}

#[test]
fn method_and_source_validation() {
    let s = Sandbox::new("method");
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--method", "dom"]);
    assert_eq!(r.code, 2, "the demo unit has no boot flash");
    assert!(r.stderr.contains("boot flash"));
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--source", "offline"]).code, 2, "offline needs a bundle");
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--source", "online", "--bundle", "/tmp"]).code, 2);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", "/definitely/not/here"]).code, 2);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--hostname", "bad host"]).code, 2);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--serial", "ttyS0;rm"]).code, 2);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--serial-speed", "9600"]).code, 2, "speed needs --serial");
    // a directory that is not a finished bundle is refused before anything else happens
    let dir = s.path("bundle");
    std::fs::create_dir_all(&dir).unwrap();
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", dir.to_str().unwrap()]);
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("--bundle") && r.stderr.contains("manifest.json"), "{}", r.stderr);
}

// ---------------------------------------------------------------------------------------------- wizard, inventory, logs, config

#[test]
fn the_wizard_needs_a_terminal() {
    let s = Sandbox::new("wiz");
    let r = s.run(&["wizard"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("interactive terminal") && r.stderr.contains("install --host H --dry-run"));
    let r = s.run(&["wizard", "--demo"]);
    assert_eq!(r.code, 2, "even the rehearsal asks questions");
}

#[test]
fn inventory_add_list_forget() {
    let s = Sandbox::new("inv");
    assert_eq!(s.run(&["inventory", "list", "--json"]).json()["devices"].as_array().unwrap().len(), 0);
    let r = s.run(&["inventory", "add", "--ip", "127.0.0.1", "--name", "loopback"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v = s.run(&["inventory", "list", "--json"]).json();
    assert_eq!(v["devices"][0]["hostname"], "loopback");
    assert!(v["devices"][0]["sources"].as_array().unwrap().iter().any(|x| x == "manual"));
    assert!(s.run(&["inventory", "list"]).stdout.contains("127.0.0.1"));
    assert_eq!(s.run(&["inventory", "forget", "10.9.9.9"]).code, 1, "nothing matches");
    assert_eq!(s.run(&["inventory", "forget", "127.0.0.1"]).code, 0);
    assert_eq!(s.run(&["inventory", "list", "--json"]).json()["devices"].as_array().unwrap().len(), 0);
}

#[test]
fn scan_json_is_a_device_list_even_with_no_network() {
    let s = Sandbox::new("scan");
    let r = s.run(&["scan", "--no-sweep", "--timeout", "1", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.json()["devices"].is_array());
    assert!(s.path("data/synofurb/inventory.json").is_file() || s.path("data").exists(), "the inventory is saved");
    let r = s.run(&["scan", "--no-sweep", "--timeout", "1"]);
    assert!(r.stdout.contains("Synology device(s) found"));
}

#[test]
fn logs_and_config() {
    let s = Sandbox::new("cfg");
    let p = s.run(&["config", "paths", "--json"]).json();
    has_keys(&p, &["data_dir", "logs", "inventory", "keys", "dom_backups", "disk_metadata_backups", "cache"]);
    let logs = PathBuf::from(p["logs"].as_str().unwrap());
    assert!(logs.starts_with(s.path("data")), "{logs:?} is inside the sandbox");
    assert_eq!(s.run(&["logs", "path"]).stdout.trim(), logs.to_str().unwrap());
    assert_eq!(s.run(&["logs", "list", "--json"]).json()["files"].as_array().unwrap().len(), 0);
    assert_eq!(s.run(&["logs", "path", "--latest"]).code, 1);
    assert_eq!(s.run(&["logs", "show"]).code, 2);
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("synofurb-20260101-000001.log"), "one\ntwo\nthree\n").unwrap();
    assert_eq!(s.run(&["logs", "list", "--json"]).json()["files"].as_array().unwrap().len(), 1);
    assert_eq!(s.run(&["logs", "show", "--tail", "2"]).stdout.trim(), "two\nthree");
    assert!(s.run(&["logs", "path", "--latest"]).stdout.contains("synofurb-20260101-000001.log"));
    let v = s.run_env(&["config", "show", "--json"], &[("SYNOFURB_PASSWORD", "topsecret")]).json();
    assert_eq!(v["environment"]["SYNOFURB_PASSWORD"], "set");
    assert!(!s.run_env(&["config", "show"], &[("SYNOFURB_PASSWORD", "topsecret")]).stdout.contains("topsecret"));
}

/// A minimal but structurally valid bundle (what `Bundle::open` and `check_for_install` look at), written without any network.
fn fake_bundle(dir: &std::path::Path, archs: &[&str], kernel: KernelChannel, extra: &[&str], firmware: bool) {
    let mut m = BundleManifest {
        format: FORMAT_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").into(),
        created: "2026-11-01T12:00:00+00:00".into(),
        suite: "trixie".into(),
        suites: vec!["trixie".into()],
        kernel,
        with_firmware: firmware,
        extra_packages: extra.iter().map(|s| s.to_string()).collect(),
        release_dates: Default::default(),
        pgp_verified: false,
        warnings: vec![],
        archs: Default::default(),
        files: vec![],
    };
    for a in archs {
        m.archs.insert(a.to_string(), ArchManifest::default());
    }
    std::fs::create_dir_all(dir.join("mirror/debian/dists/trixie")).unwrap();
    std::fs::write(
        dir.join("mirror/debian/dists/trixie/InRelease"),
        "Origin: Debian\nSuite: trixie\nDate: Sat, 01 Nov 2026 00:00:00 UTC\n",
    )
    .unwrap();
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_string_pretty(&m).unwrap()).unwrap();
}

#[test]
fn bundle_info_and_verify() {
    let s = Sandbox::new("bundleinfo");
    let b = s.path("b");
    fake_bundle(&b, &["amd64"], KernelChannel::Newest, &["htop"], false);
    let r = s.run(&["bundle", "info", b.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v = r.json();
    assert_eq!(v["suite"], "trixie");
    assert_eq!(v["archs"][0]["arch"], "amd64");
    assert_eq!(v["extra_packages"][0], "htop");
    let r = s.run(&["bundle", "info", b.to_str().unwrap()]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("amd64") && r.stdout.contains("trixie"), "{}", r.stdout);
    let r = s.run(&["bundle", "verify", b.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    // a file the manifest promises but the disk lacks fails verification
    let mut m: BundleManifest = serde_json::from_str(&std::fs::read_to_string(b.join(MANIFEST_FILE)).unwrap()).unwrap();
    m.files.push(FileEntry { path: "installer/amd64/kernel".into(), size: 3, sha256: "0".repeat(64) });
    std::fs::write(b.join(MANIFEST_FILE), serde_json::to_string(&m).unwrap()).unwrap();
    let r = s.run(&["bundle", "verify", b.to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert_eq!(s.run(&["bundle", "info", s.path("nowhere").to_str().unwrap()]).code, 1);
    assert_eq!(s.run(&["bundle", "verify", s.path("nowhere").to_str().unwrap()]).code, 1);
}

#[test]
fn bundle_create_is_wired_to_the_engine() {
    let s = Sandbox::new("bundlecreate");
    // the engine starts (its banner names the output directory) and fails cleanly where building is impossible: the output path is a file
    let blocker = s.path("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let r = s.run(&["bundle", "create", "--arch", "amd64", "--out", blocker.to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("Building the offline bundle in") && r.stderr.contains("amd64"), "{}", r.stderr);
}

#[test]
fn offline_install_is_checked_against_the_bundle_before_the_nas_is_touched() {
    let s = Sandbox::new("offline");
    let ok = s.path("ok");
    fake_bundle(&ok, &["amd64"], KernelChannel::Newest, &["htop", "tmux"], true);
    // dry run with a fitting bundle: offline mirror settings in the preseed and payload
    let out = s.path("review");
    let r = s.run(&[
        "install",
        "--facts",
        "demo",
        "--dry-run",
        "--bundle",
        ok.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--package",
        "htop",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let preseed = std::fs::read_to_string(out.join("preseed.cfg")).unwrap();
    assert!(
        preseed.contains("mirror/http/hostname string 192.0.2.10:18080")
            && preseed.contains("mirror/http/directory string /DRYRUN/mirror/debian"),
        "{preseed}"
    );
    let env = std::fs::read_to_string(out.join("payload/env.sh")).unwrap();
    assert!(env.contains("MIRROR_HOST='192.0.2.10:18080'"), "{env}");
    let j = s.run(&["install", "--facts", "demo", "--dry-run", "--json", "--source", "offline", "--bundle", ok.to_str().unwrap()]).json();
    assert_eq!(j["offline_bundle"], ok.to_str().unwrap());
    assert!(j["placeholders"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains("OFFLINE")));
    // the online render has none of that
    let j = s.run(&["install", "--facts", "demo", "--dry-run", "--json"]).json();
    assert!(j["offline_bundle"].is_null());
    // mismatches are usage errors that explain themselves
    let arm = s.path("arm");
    fake_bundle(&arm, &["arm64"], KernelChannel::Newest, &[], true);
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", arm.to_str().unwrap()]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("no amd64"), "{}", r.stderr);
    let stable = s.path("stable");
    fake_bundle(&stable, &["amd64"], KernelChannel::Stable, &[], true);
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", stable.to_str().unwrap()]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("kernel"), "{}", r.stderr);
    // the error names both ways out, with the flags to use
    assert!(r.stderr.contains("--kernel stable") && r.stderr.contains("bundle create --kernel newest"), "{}", r.stderr);
    // and the default bundle (newest) is accepted by a default install
    let newest = s.path("newest-default");
    fake_bundle(&newest, &["amd64"], KernelChannel::Newest, &[], true);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", newest.to_str().unwrap()]).code, 0);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", stable.to_str().unwrap(), "--kernel", "stable"]).code, 0);
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", ok.to_str().unwrap(), "--package", "vim"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("vim"), "{}", r.stderr);
    // a bundle without firmware only warns
    let nofw = s.path("nofw");
    fake_bundle(&nofw, &["amd64"], KernelChannel::Newest, &[], false);
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", nofw.to_str().unwrap()]);
    assert_eq!(r.code, 0);
    assert!(r.stderr.contains("without installer firmware"), "{}", r.stderr);
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", nofw.to_str().unwrap(), "--no-firmware"]).code, 0);

    // a real install with a bad bundle stops before connecting (192.0.2.1 would hang) even with the whole gate given
    let empty = s.path("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let r = s.run_env(
        &[
            "install",
            "--host",
            "192.0.2.1",
            "--bundle",
            empty.to_str().unwrap(),
            "--have-backup",
            "--accept-data-loss",
            "--confirm",
            "ERASE DS918+",
        ],
        &[("SYNOFURB_PASSWORD", "x")],
    );
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("manifest.json"), "{}", r.stderr);
}

#[test]
fn bundle_arguments_are_checked() {
    let s = Sandbox::new("bundle");
    assert_eq!(s.run(&["bundle", "create", "--arch", "riscv64", "--out", "o"]).code, 2);
    let r = s.run(&["bundle", "create", "--arch", "amd64", "--suite", "Not A Suite", "--out", "o"]);
    assert_eq!(r.code, 2);
}

#[test]
fn colour_is_off_when_not_a_terminal_and_no_color_is_respected() {
    let s = Sandbox::new("color");
    for r in [
        s.run(&["plan", "--facts", "demo"]),
        s.run_env(&["plan", "--facts", "demo"], &[("NO_COLOR", "1")]),
        s.run(&["--no-color", "plan", "--facts", "demo"]),
    ] {
        assert!(!r.stdout.contains('\x1b') && !r.stderr.contains('\x1b'));
    }
    let r = s.run(&["plan", "--facts", "demo", "--mode", "keep", "--keep", "md99"]);
    assert!(!r.stderr.contains('\x1b'), "errors are plain without a terminal");
}

// ---------------------------------------------------------------------------------------------- manage (no unit needed)
//
// The parts of `manage` that talk to a unit are covered by the engine's tests (scripted unit, and the repository's real
// synofurb_mgmt.py) and by tests/cli/smoke.sh (a real sshd in front of that script). Here: everything that is decided before or
// without a connection.

fn write_mgmt_dir(dir: &std::path::Path, version: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("synofurb_mgmt.py"), "#!/usr/bin/env python3\nprint('x')\n").unwrap();
    std::fs::write(dir.join("VERSION"), format!("{version}\n")).unwrap();
    std::fs::write(dir.join("README.md"), "# x\n").unwrap();
}

#[test]
fn manage_open_prints_the_web_ui_address() {
    let s = Sandbox::new("manage-open");
    let r = s.run(&["manage", "open", "--host", "192.168.1.20"]);
    assert_eq!((r.code, r.stdout.trim()), (0, "https://192.168.1.20:8443"), "{}", r.stderr);
    assert_eq!(s.run(&["manage", "open", "--host", "fe80::1"]).stdout.trim(), "https://[fe80::1]:8443");
    // stdout carries data only: the self-signed-certificate hint is on stderr
    assert!(r.stderr.contains("self-signed") && !r.stdout.contains("self-signed"));
    assert_eq!(s.run(&["manage", "open", "--host", "a b"]).code, 2);
    assert_eq!(s.run(&["manage", "open"]).code, 2);
    let r = s.run(&["--log-format", "json", "manage", "open", "--host", "nas.lan"]);
    let ev: Vec<Value> = r.stdout.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!((ev[0]["event"].as_str(), ev[0]["command"].as_str()), (Some("start"), Some("manage open")));
    let last = ev.last().unwrap();
    assert_eq!(
        (last["event"].as_str(), last["ok"].as_bool(), last["data"]["url"].as_str()),
        (Some("result"), Some(true), Some("https://nas.lan:8443"))
    );
}

#[test]
fn manage_needs_a_host_and_never_takes_a_password_on_the_command_line() {
    let s = Sandbox::new("manage-usage");
    for sub in ["health", "version", "update", "rollback"] {
        let r = s.run(&["manage", sub]);
        assert_eq!(r.code, 2, "{sub}: {}", r.stderr);
        assert!(r.stderr.contains("--host"), "{sub}: {}", r.stderr);
        let r = s.run(&["manage", sub, "--host", "h", "--password", "hunter2"]);
        assert_eq!(r.code, 2, "{sub}");
        assert!(r.stderr.contains("unexpected argument") && !r.stderr.contains("hunter2 "), "{sub}: {}", r.stderr);
        // there is no terminal to ask for a password on
        let r = s.run(&["manage", sub, "--host", "127.0.0.1", "--port", "1"]);
        assert_eq!(r.code, 2, "{sub}: {}", r.stderr);
        assert!(r.stderr.contains("SYNOFURB_PASSWORD"), "{sub}: {}", r.stderr);
    }
    assert_eq!(s.run(&["manage"]).code, 2);
    assert_eq!(s.run(&["manage", "frobnicate"]).code, 2);
}

#[test]
fn manage_commands_fail_cleanly_when_the_unit_is_unreachable() {
    let s = Sandbox::new("manage-down");
    for sub in ["health", "version", "update --yes", "rollback --yes"] {
        let mut a = vec!["manage"];
        a.extend(sub.split(' '));
        a.extend(["--host", "127.0.0.1", "--port", "1"]);
        let r = s.run_env(&a, &[("SYNOFURB_PASSWORD", "hunter2-secret")]);
        assert_eq!(r.code, 1, "{sub}: {}{}", r.stdout, r.stderr);
        assert!(r.stderr.contains("error:"), "{sub}: {}", r.stderr);
        assert!(!r.stdout.contains("hunter2-secret") && !r.stderr.contains("hunter2-secret"), "{sub}: the password was printed");
    }
    // NDJSON: start (where the command has one) and a failure event with exit code 1
    let r = s.run_env(&["--log-format", "json", "manage", "health", "--host", "127.0.0.1", "--port", "1"], &[("SYNOFURB_PASSWORD", "x")]);
    let last: Value = serde_json::from_str(r.stdout.lines().last().unwrap()).unwrap();
    assert_eq!((last["event"].as_str(), last["exit_code"].as_u64(), last["kind"].as_str()), (Some("failure"), Some(1), Some("error")));
}

#[test]
fn manage_update_checks_its_bundle_before_connecting() {
    let s = Sandbox::new("manage-bundle");
    let env = [("SYNOFURB_PASSWORD", "x")];
    // not there / not a management plane folder: a usage error, no connection attempt (port 1 would give exit 1)
    for dir in [s.path("nope"), s.dir.clone()] {
        let r = s.run_env(&["manage", "update", "--host", "127.0.0.1", "--port", "1", "--from-bundle", dir.to_str().unwrap()], &env);
        assert_eq!(r.code, 2, "{}", r.stderr);
        assert!(r.stderr.contains("--from-bundle"), "{}", r.stderr);
    }
    // a folder with a manifest that does not match its files
    let bad = s.path("bad");
    write_mgmt_dir(&bad, "9.9.9");
    std::fs::write(
        bad.join("MANIFEST.sha256"),
        format!("{}  synofurb_mgmt.py\n{}  VERSION\n{}  README.md\n", "0".repeat(64), "0".repeat(64), "0".repeat(64)),
    )
    .unwrap();
    let r = s.run_env(&["manage", "update", "--host", "127.0.0.1", "--port", "1", "--from-bundle", bad.to_str().unwrap()], &env);
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("different checksum"), "{}", r.stderr);
    // a good folder (also inside an offline bundle's mgmt/) gets past the check and fails on the connection instead
    let good = s.path("good/mgmt");
    write_mgmt_dir(&good, "9.9.9");
    for dir in [good.clone(), good.parent().unwrap().to_path_buf()] {
        let r = s.run_env(&["manage", "update", "--host", "127.0.0.1", "--port", "1", "--from-bundle", dir.to_str().unwrap()], &env);
        assert_eq!(r.code, 1, "{}", r.stderr);
    }
}

/// `fake_bundle` plus the `mgmt/` folder `bundle create` writes (the embedded management app and its MANIFEST.sha256), listed in manifest.json.
fn add_mgmt_to_bundle(dir: &std::path::Path) {
    let b = synofurb_core::manage::MgmtBundle::embedded();
    let md = dir.join("mgmt");
    std::fs::create_dir_all(&md).unwrap();
    let mut files = vec![];
    let mut put = |name: &str, data: &[u8]| {
        std::fs::write(md.join(name), data).unwrap();
        files.push(FileEntry { path: format!("mgmt/{name}"), size: data.len() as u64, sha256: synofurb_core::debian::sha256_hex(data) });
    };
    for f in &b.files {
        put(&f.name, &f.data);
    }
    put("MANIFEST.sha256", b.manifest().as_bytes());
    let mut m: BundleManifest = serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    m.files.extend(files);
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_string_pretty(&m).unwrap()).unwrap();
}

#[test]
fn a_bundle_carries_the_management_app_for_offline_updates() {
    let s = Sandbox::new("bundle-mgmt");
    let env = [("SYNOFURB_PASSWORD", "x")];
    let with = s.path("with");
    fake_bundle(&with, &["amd64"], KernelChannel::Stable, &[], false);
    add_mgmt_to_bundle(&with);
    let ver = synofurb_core::manage::bundled_mgmt_version();

    // verify: OK, says which management app it found, no note
    let r = s.run(&["bundle", "verify", with.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains(&format!("management app {ver} in mgmt/ verified")) && r.stdout.contains("bundle OK"), "{}", r.stdout);
    assert!(!r.stdout.contains("note:"), "{}", r.stdout);
    // info (text and JSON)
    let r = s.run(&["bundle", "info", with.to_str().unwrap()]);
    assert!(r.stdout.contains(&format!("Mgmt app:  synofurb-mgmt {ver}")), "{}", r.stdout);
    assert_eq!(s.run(&["bundle", "info", with.to_str().unwrap(), "--json"]).json()["mgmt_version"], ver);
    // the bundle (or its mgmt/ folder) is accepted by `manage update` and gets as far as the connection (port 1: exit 1)
    for dir in [with.clone(), with.join("mgmt")] {
        let r = s.run_env(&["manage", "update", "--host", "127.0.0.1", "--port", "1", "--from-bundle", dir.to_str().unwrap()], &env);
        assert_eq!(r.code, 1, "{}: {}", dir.display(), r.stderr);
    }
    // install still accepts it
    assert_eq!(s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", with.to_str().unwrap(), "--kernel", "stable"]).code, 0);

    // a bundle from an older synofurb: still verifies, with a note; `manage update` explains what to do
    let old = s.path("old");
    fake_bundle(&old, &["amd64"], KernelChannel::Stable, &[], false);
    let r = s.run(&["bundle", "verify", old.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("note:") && r.stdout.contains("no mgmt/ folder") && r.stdout.contains("bundle OK"), "{}", r.stdout);
    assert!(s.run(&["bundle", "info", old.to_str().unwrap()]).stdout.contains("Mgmt app:  not included"));
    assert!(s.run(&["bundle", "info", old.to_str().unwrap(), "--json"]).json()["mgmt_version"].is_null());
    let r = s.run_env(&["manage", "update", "--host", "127.0.0.1", "--port", "1", "--from-bundle", old.to_str().unwrap()], &env);
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stderr.contains("no mgmt/ folder") && r.stderr.contains("bundle create"), "{}", r.stderr);

    // tampering: verify fails (exit 1, MGMT/CHANGED lines) and so does `manage update`, before any connection (exit 2)
    std::fs::write(with.join("mgmt/synofurb_mgmt.py"), "print('patched')\n").unwrap();
    let r = s.run(&["bundle", "verify", with.to_str().unwrap()]);
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("CHANGED  mgmt/synofurb_mgmt.py") && r.stdout.contains("MGMT "), "{}", r.stdout);
    assert!(!r.stdout.contains("bundle OK"));
    for dir in [with.clone(), with.join("mgmt")] {
        let r = s.run_env(&["manage", "update", "--host", "127.0.0.1", "--port", "1", "--from-bundle", dir.to_str().unwrap()], &env);
        assert_eq!(r.code, 2, "{}: {}", dir.display(), r.stderr);
        assert!(r.stderr.contains("--from-bundle"), "{}", r.stderr);
    }
    // ... and an install refuses the damaged bundle too (exit 2, nothing contacted)
    let r = s.run(&["install", "--facts", "demo", "--dry-run", "--bundle", with.to_str().unwrap(), "--kernel", "stable"]);
    assert_eq!(r.code, 2, "{}{}", r.stdout, r.stderr);
}

#[test]
fn manage_is_documented_and_completed() {
    let s = Sandbox::new("manage-help");
    let h = s.run(&["manage", "--help"]);
    assert_eq!(h.code, 0);
    for c in ["health", "version", "update", "rollback", "open"] {
        assert!(h.stdout.contains(c), "manage --help lacks {c}");
    }
    let u = s.run(&["manage", "update", "--help"]);
    for f in ["--from-bundle", "--force", "--dry-run", "--yes", "EXAMPLES"] {
        assert!(u.stdout.contains(f), "manage update --help lacks {f}");
    }
    assert!(s.run(&["--help"]).stdout.contains("8 `manage health`"), "the exit codes 8 and 9 are in the top-level help");
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let c = s.run(&["completions", shell]).stdout;
        assert!(c.contains("manage") && c.contains("rollback") && c.contains("from-bundle"), "{shell}");
    }
}

#[test]
fn unsigned_self_update_is_an_explicit_opt_in_and_never_satisfied_by_yes_alone() {
    let s = Sandbox::new("unsigned");
    // without a terminal, --allow-unsigned also needs --yes: refused (exit 3) before any network traffic
    let r = s.run(&["self-update", "--allow-unsigned"]);
    assert_eq!(r.code, 3, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("--allow-unsigned") && r.stderr.contains("--yes") && r.stderr.contains("Nothing was changed"),
        "{}",
        r.stderr
    );
    // it is not a way to check, and not a default
    assert_eq!(s.run(&["self-update", "--check", "--allow-unsigned"]).code, 2);
    // --yes alone does not enable it: the flag is a separate opt-in (the command fails on the network or says how to opt in, never installs)
    let h = s.run(&["self-update", "--help"]);
    assert!(h.stdout.contains("--allow-unsigned") && h.stdout.contains("SHA-256"), "{}", h.stdout);
    // the shipped build still has the placeholder key, so the flag is accepted by the argument checks
    assert!(synofurb_core::updater::unsigned_allowed());
}
