//! The choices that make an install: turned from flags (or wizard answers) into a validated [`InstallConfig`], a boot method and an
//! [`InstallJob`]. One implementation for `install` and `wizard`.

use super::args::{BootOpts, MethodArg, SourceArg, SourceOpts, SystemOpts};
use super::exit::{CliError, CliResult};
use super::facts::{Inspection, config_dir, method_available, resolve_auto_method};
use super::out::Out;
use super::prompt::Prompter;
use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use synofurb_core::config::*;
use synofurb_core::model::*;
use synofurb_core::plan::Plan;
use synofurb_core::session::{BootMethod, InstallJob, SshTarget};

#[derive(Clone)]
pub enum PasswordChoice {
    /// A strong random password, shown at the end.
    Generate,
    /// The user's own password. Never echoed back in results.
    Given(String),
}

impl std::fmt::Debug for PasswordChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasswordChoice::Generate => f.write_str("Generate"),
            PasswordChoice::Given(_) => f.write_str("Given(<hidden>)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyChoice {
    Generate,
    None,
    /// Path of a public key file.
    File(PathBuf),
    /// A public key line (wizard).
    Line(String),
}

#[derive(Clone, Debug)]
pub struct InstallSpec {
    pub hostname: String,
    pub username: String,
    pub password: PasswordChoice,
    pub key: KeyChoice,
    pub timezone: String,
    pub locale: String,
    pub keymap: String,
    pub mirror_host: String,
    pub mirror_dir: String,
    pub packages: Vec<String>,
    pub serial: Option<SerialConsole>,
    pub dtb: Option<String>,
    pub mgmt: bool,
    pub kernel: KernelChannel,
    pub method: MethodArg,
    pub agent_dir: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub kexec: Option<PathBuf>,
    pub kit_dir: PathBuf,
    pub firmware: bool,
    pub verbose_installer: bool,
    pub bundle: Option<PathBuf>,
}

impl InstallSpec {
    /// Defaults identical to the flag defaults.
    pub fn defaults() -> InstallSpec {
        InstallSpec {
            hostname: "nas".into(),
            username: "nasadmin".into(),
            password: PasswordChoice::Generate,
            key: KeyChoice::Generate,
            timezone: "UTC".into(),
            locale: "en_US.UTF-8".into(),
            keymap: "us".into(),
            mirror_host: "deb.debian.org".into(),
            mirror_dir: "/debian".into(),
            packages: vec![],
            serial: None,
            dtb: None,
            mgmt: true,
            kernel: KernelChannel::Newest,
            method: MethodArg::Auto,
            agent_dir: None,
            backup_dir: None,
            kexec: None,
            kit_dir: PathBuf::from("synofurb-kit"),
            firmware: true,
            verbose_installer: true,
            bundle: None,
        }
    }

    pub fn from_args(s: &SystemOpts, b: &BootOpts, src: &SourceOpts, out: &Out, prompt: &mut dyn Prompter) -> CliResult<InstallSpec> {
        let password = resolve_password(s, out, prompt)?;
        let key = parse_key_choice(&s.authorize_key)?;
        let serial = match (&s.serial, s.serial_speed) {
            (Some(t), speed) => Some(parse_serial(t, speed).map_err(|e| CliError::usage(e.to_string()))?),
            (None, Some(_)) => return Err(CliError::usage("--serial-speed needs --serial (by default the speed DSM uses is kept)")),
            (None, None) => None,
        };
        let bundle = resolve_bundle(src)?;
        Ok(InstallSpec {
            hostname: s.hostname.clone(),
            username: s.username.clone(),
            password,
            key,
            timezone: s.timezone.clone(),
            locale: s.locale.clone(),
            keymap: s.keymap.clone(),
            mirror_host: s.mirror.clone(),
            mirror_dir: s.mirror_dir.clone(),
            packages: s.packages.clone(),
            serial,
            dtb: s.dtb.clone(),
            mgmt: !s.no_mgmt,
            kernel: s.kernel.channel(),
            method: b.method,
            agent_dir: b.agent_dir.clone(),
            backup_dir: b.backup_dir.clone(),
            kexec: b.kexec.clone(),
            kit_dir: b.kit_dir.clone(),
            firmware: !b.no_firmware,
            verbose_installer: !b.no_verbose_installer,
            bundle,
        })
    }
}

/// `--source offline` needs `--bundle`; `--bundle` implies offline; `--source online` with a bundle is contradictory.
pub fn resolve_bundle(src: &SourceOpts) -> CliResult<Option<PathBuf>> {
    match (src.source, &src.bundle) {
        (Some(SourceArg::Offline), None) => Err(CliError::usage("--source offline needs --bundle DIR (made by `bundle create`)")),
        (Some(SourceArg::Online), Some(_)) => Err(CliError::usage("--bundle and --source online contradict each other")),
        (_, Some(dir)) => {
            if !dir.is_dir() {
                return Err(CliError::usage(format!("--bundle {}: not a directory", dir.display())));
            }
            Ok(Some(dir.clone()))
        }
        (_, None) => Ok(None),
    }
}

fn read_first_line(path: &Path) -> Result<String> {
    let t = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(t.lines().next().unwrap_or("").trim_end_matches('\r').to_string())
}

fn resolve_password(s: &SystemOpts, out: &Out, prompt: &mut dyn Prompter) -> CliResult<PasswordChoice> {
    let non_empty = |p: String, what: &str| -> CliResult<PasswordChoice> {
        if p.is_empty() { Err(CliError::usage(format!("the password from {what} is empty"))) } else { Ok(PasswordChoice::Given(p)) }
    };
    if let Some(f) = &s.password_file {
        return non_empty(read_first_line(f)?, &f.display().to_string());
    }
    if let Some(v) = &s.password_env {
        return match std::env::var(v) {
            Ok(p) => non_empty(p, &format!("${v}")),
            Err(_) => Err(CliError::usage(format!("--password-env {v}: that environment variable is not set"))),
        };
    }
    if s.prompt_password {
        if !out.interactive {
            return Err(CliError::usage("--prompt-password needs a terminal; use --password-file or --password-env"));
        }
        let a = prompt.secret("Password for the new account")?;
        let b = prompt.secret("Repeat the password")?;
        if a != b {
            return Err(CliError::usage("the two passwords differ"));
        }
        return non_empty(a, "the prompt");
    }
    Ok(PasswordChoice::Generate)
}

pub fn parse_key_choice(v: &str) -> CliResult<KeyChoice> {
    Ok(match v.trim() {
        "generate" => KeyChoice::Generate,
        "none" => KeyChoice::None,
        "" => return Err(CliError::usage("--authorize-key needs generate, none or the path of a public key file")),
        p => KeyChoice::File(PathBuf::from(p)),
    })
}

/// A serial console from user input: `ttyS0`, or a raw `uart8250,mmio32,0xfedc9000,115200n8` console (AMD boards have no legacy UART).
pub fn parse_serial(text: &str, speed: Option<u32>) -> Result<SerialConsole> {
    let t = text.trim();
    if t.is_empty() {
        return Err(anyhow!("the serial console is empty"));
    }
    let speed_ok = |s: u32| (1200..=4_000_000).contains(&s);
    if t.contains(',') || t.contains("0x") {
        let c = SerialConsole::from_cmdline(&format!("console={t}"))
            .ok_or_else(|| anyhow!("'{t}' is not a valid raw serial console (example: uart8250,mmio32,0xfedc9000,115200n8)"))?;
        return match speed {
            Some(s) if speed_ok(s) => Ok(SerialConsole { speed: s, ..c }),
            Some(s) => Err(anyhow!("serial speed {s} is out of range (1200-4000000)")),
            None => Ok(c),
        };
    }
    if !t.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(anyhow!("'{t}' is not a valid tty name (example: ttyS0)"));
    }
    let s = speed.unwrap_or(115_200);
    if !speed_ok(s) {
        return Err(anyhow!("serial speed {s} is out of range (1200-4000000)"));
    }
    Ok(SerialConsole { tty: t.to_string(), speed: s, raw: None })
}

/// Text shown in the serial field: the raw console if there is one, else the tty name.
pub fn serial_text(c: &SerialConsole) -> String {
    c.raw.clone().unwrap_or_else(|| c.tty.clone())
}

/// What `install` builds from a spec.
pub struct Material {
    pub cfg: InstallConfig,
    pub password: String,
    pub password_generated: bool,
    /// Where a generated private key was saved, and its fingerprint.
    pub key_path: Option<PathBuf>,
    pub key_fingerprint: Option<String>,
}

/// The Debian suite installs use (not a flag: it is the engine's default).
pub fn default_suite() -> String {
    InstallConfig::new("nas", "nasadmin", DRY_RUN_HASH.into()).suite
}

/// A hash that passes validation but belongs to no password: dry runs never touch the real one.
pub const DRY_RUN_HASH: &str = "$6$DRYRUNSALT000000$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

/// Private key file name that does not overwrite an earlier install's key.
fn free_key_path(dir: &Path, hostname: &str) -> PathBuf {
    let first = dir.join(format!("{hostname}_ed25519"));
    if !first.exists() {
        return first;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    dir.join(format!("{hostname}-{stamp}_ed25519"))
}

fn write_private(path: &Path, content: &str) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Validated configuration. `dry`: no key file is written and the password hash is a placeholder.
pub fn build_material(spec: &InstallSpec, facts: &DeviceFacts, dry: bool, out: &Out) -> CliResult<Material> {
    let (password, generated) = match &spec.password {
        PasswordChoice::Generate => (generate_password(20), true),
        PasswordChoice::Given(p) => (p.clone(), false),
    };
    if !generated && password.chars().count() < 8 {
        out.warn("the new account's password is shorter than 8 characters");
    }
    if password.contains(['\n', '\r']) {
        return Err(CliError::usage("the password must be a single line"));
    }
    let hash = if dry { DRY_RUN_HASH.to_string() } else { hash_password(&password)? };
    let mut cfg = InstallConfig::new(&spec.hostname, &spec.username, hash);
    cfg.timezone = spec.timezone.clone();
    cfg.locale = spec.locale.clone();
    cfg.keymap = spec.keymap.clone();
    cfg.mirror_host = spec.mirror_host.clone();
    cfg.mirror_dir = spec.mirror_dir.clone();
    cfg.enable_mgmt = spec.mgmt;
    cfg.kernel = spec.kernel;
    cfg.extra_packages = spec.packages.clone();
    cfg.serial = spec.serial.clone().unwrap_or_else(|| facts.serial_console());
    cfg.dtb = spec.dtb.clone().or_else(|| {
        synofurb_core::platform::mainline_dtb(&facts.model)
            .or_else(|| synofurb_core::platform::mainline_dtb(&facts.unique))
            .map(String::from)
    });
    let mut key_path = None;
    let mut key_fingerprint = None;
    match &spec.key {
        KeyChoice::None => {}
        KeyChoice::File(p) => {
            let line = std::fs::read_to_string(p).with_context(|| format!("reading the public key {}", p.display()))?;
            cfg.ssh_authorized_keys.push(line.trim().to_string());
        }
        KeyChoice::Line(l) => cfg.ssh_authorized_keys.push(l.trim().to_string()),
        KeyChoice::Generate => {
            let kp = generate_ssh_keypair(&format!("synofurb-{}", spec.hostname))?;
            key_fingerprint = Some(kp.fingerprint.clone());
            if !dry {
                let path = free_key_path(&config_dir().join("keys"), &spec.hostname);
                write_private(&path, &kp.private_openssh)?;
                key_path = Some(path);
            }
            cfg.ssh_authorized_keys.push(kp.public_openssh);
        }
    }
    cfg.validate().map_err(|e| CliError::usage(format!("{e:#}")))?;
    Ok(Material { cfg, password, password_generated: generated, key_path, key_fingerprint })
}

/// Resolve `--method`. Returns the method name and the core value; refuses what the unit cannot do (exit 2).
pub fn resolve_method(spec: &InstallSpec, facts: &DeviceFacts) -> CliResult<(&'static str, BootMethod)> {
    let name = match spec.method {
        MethodArg::Auto => resolve_auto_method(facts),
        MethodArg::Dom => "dom",
        MethodArg::Kexec => "kexec",
        MethodArg::Kit => "kit",
    };
    if let Err(why) = method_available(name, facts) {
        return Err(CliError::usage(format!("--method {name}: {why} Use --method auto or kit.")));
    }
    let method = match name {
        "dom" => BootMethod::Dom {
            agent_dir: spec.agent_dir.clone(),
            backup_dir: spec.backup_dir.clone().unwrap_or_else(|| config_dir().join("dom-backups")),
        },
        "kexec" => BootMethod::Kexec { kexec_bin: spec.kexec.clone() },
        _ => BootMethod::Kit { dir: spec.kit_dir.clone() },
    };
    Ok((name, method))
}

#[allow(clippy::too_many_arguments)]
pub fn build_job(spec: &InstallSpec, insp: &Inspection, plan: Plan, target: SshTarget, mat: &Material, method: BootMethod) -> InstallJob {
    InstallJob {
        target,
        facts: insp.facts.clone(),
        plan,
        cfg: mat.cfg.clone(),
        method,
        cache_dir: config_dir().join("cache"),
        with_firmware: spec.firmware,
        verbose_installer: spec.verbose_installer,
        display_password: mat.password.clone(),
        offline_bundle: spec.bundle.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::facts::demo_facts;

    #[test]
    fn serial_parsing_matches_the_app() {
        let c = parse_serial("ttyS0", Some(115_200)).unwrap();
        assert_eq!((c.tty.as_str(), c.speed, c.raw.is_none()), ("ttyS0", 115_200, true));
        let amd = parse_serial("uart8250,mmio32,0xfedc9000,115200n8", None).unwrap();
        assert_eq!(amd.raw.as_deref(), Some("uart8250,mmio32,0xfedc9000,115200n8"));
        assert_eq!(amd.tty, "");
        assert_eq!(amd.kernel_arg(), "console=uart8250,mmio32,0xfedc9000,115200n8");
        assert!(parse_serial("ttyS0;rm -rf /", None).is_err());
        assert!(parse_serial("", None).is_err());
        assert!(parse_serial("ttyS0", Some(5)).is_err());
        assert_eq!(parse_serial("ttyS1", Some(9600)).unwrap().speed, 9600);
        assert_eq!(serial_text(&amd), "uart8250,mmio32,0xfedc9000,115200n8");
        assert_eq!(serial_text(&SerialConsole::default()), "ttyS0");
        assert!(parse_serial("uart8250,mmio32,0xfe;rm,9600", None).is_err());
    }

    #[test]
    fn key_choices() {
        assert_eq!(parse_key_choice("generate").unwrap(), KeyChoice::Generate);
        assert_eq!(parse_key_choice("none").unwrap(), KeyChoice::None);
        assert_eq!(parse_key_choice("/home/me/id.pub").unwrap(), KeyChoice::File("/home/me/id.pub".into()));
        assert!(parse_key_choice(" ").is_err());
    }

    #[test]
    fn dry_run_material_has_no_secrets_and_writes_nothing() {
        let (facts, _) = demo_facts();
        let mat = build_material(&InstallSpec::defaults(), &facts, true, &Out::plain()).unwrap();
        assert_eq!(mat.cfg.password_hash, DRY_RUN_HASH);
        assert!(mat.key_path.is_none(), "a dry run must not write key files");
        assert_eq!(mat.cfg.ssh_authorized_keys.len(), 1);
        assert_eq!(mat.cfg.serial.kernel_arg(), "console=ttyS0,115200n8", "serial console comes from DSM");
        assert!(mat.password_generated && mat.password.len() == 20);
    }

    #[test]
    fn spec_overrides_reach_the_config() {
        let (facts, _) = demo_facts();
        let mut spec = InstallSpec::defaults();
        spec.hostname = "vault".into();
        spec.key = KeyChoice::None;
        spec.packages = vec!["htop".into()];
        spec.serial = Some(parse_serial("uart8250,mmio32,0xfedc9000,115200n8", None).unwrap());
        spec.mgmt = false;
        spec.kernel = KernelChannel::Stable;
        spec.password = PasswordChoice::Given("correct horse battery".into());
        let mat = build_material(&spec, &facts, true, &Out::plain()).unwrap();
        assert_eq!(mat.cfg.hostname, "vault");
        assert!(mat.cfg.ssh_authorized_keys.is_empty() && !mat.cfg.enable_mgmt && mat.cfg.kernel == KernelChannel::Stable);
        assert_eq!(mat.cfg.extra_packages, vec!["htop"]);
        assert_eq!(mat.cfg.serial.raw.as_deref(), Some("uart8250,mmio32,0xfedc9000,115200n8"));
        assert!(!mat.password_generated);
        spec.hostname = "bad host".into();
        let e = build_material(&spec, &facts, true, &Out::plain()).err().unwrap();
        assert_eq!(e.code(), 2, "invalid input is a usage error");
        spec.hostname = "ok".into();
        spec.packages = vec!["x;rm".into()];
        assert!(build_material(&spec, &facts, true, &Out::plain()).is_err());
    }

    #[test]
    fn method_resolution() {
        let (mut facts, _) = demo_facts();
        let mut spec = InstallSpec::defaults();
        assert_eq!(resolve_method(&spec, &facts).unwrap().0, "kexec");
        spec.method = MethodArg::Dom;
        assert_eq!(resolve_method(&spec, &facts).err().unwrap().code(), 2, "dom on a unit without a boot flash");
        spec.method = MethodArg::Kit;
        assert_eq!(resolve_method(&spec, &facts).unwrap().0, "kit");
        spec.method = MethodArg::Auto;
        facts.kexec = false;
        assert_eq!(resolve_method(&spec, &facts).unwrap().0, "kit");
    }

    #[test]
    fn offline_needs_a_bundle() {
        let src = SourceOpts { source: Some(SourceArg::Offline), bundle: None };
        assert_eq!(resolve_bundle(&src).err().unwrap().code(), 2);
        let src = SourceOpts { source: None, bundle: Some("/definitely/not/here".into()) };
        assert_eq!(resolve_bundle(&src).err().unwrap().code(), 2);
        let tmp = std::env::temp_dir();
        let src = SourceOpts { source: Some(SourceArg::Offline), bundle: Some(tmp.clone()) };
        assert_eq!(resolve_bundle(&src).unwrap(), Some(tmp));
        assert_eq!(resolve_bundle(&SourceOpts { source: Some(SourceArg::Online), bundle: None }).unwrap(), None);
        let tmp2 = std::env::temp_dir();
        assert_eq!(resolve_bundle(&SourceOpts { source: Some(SourceArg::Online), bundle: Some(tmp2) }).err().unwrap().code(), 2);
    }
}
