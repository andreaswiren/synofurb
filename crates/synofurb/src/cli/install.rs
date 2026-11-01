//! `inspect`, `plan` and `install`.

use super::args::{InspectArgs, InstallArgs, PlanArgs};
use super::bundle;
use super::dryrun;
use super::exit::{CliError, CliResult};
use super::facts::{self, assess, inspect_doc, load, load_compat_db, plan_doc};
use super::gate;
use super::out::Out;
use super::prompt::StdPrompter;
use super::render;
use super::runner::{console_logger, progress_fn, run_job};
use super::spec::{self, InstallSpec, Material, build_job, build_material, resolve_method, serial_text};
use serde_json::json;
use std::path::Path;
use synofurb_core::plan::{Plan, build_plan};
use synofurb_core::session::{self, InstallJob, Outcome};

// ---------------------------------------------------------------------------------------------- inspect

pub fn inspect(out: &Out, a: InspectArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let log = console_logger(out);
    let insp = load(&a.src, out, &mut prompt, &log)?;
    let db = load_compat_db();
    let asm = assess(&db, &insp.facts);
    if let Some(p) = &a.save {
        std::fs::write(p, serde_json::to_string_pretty(&insp.facts)?).map_err(|e| anyhow::anyhow!("writing {}: {e}", p.display()))?;
        out.info(&format!("facts saved to {} (use them later with --facts)", p.display()));
    }
    if a.json {
        out.json_doc(&inspect_doc(&insp.facts, &asm));
    } else {
        out.data_lines(render::inspect_lines(&insp.facts, &asm));
    }
    match asm.blocking() {
        Some(why) => Err(CliError::Unsupported(why)),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------- plan

pub fn plan(out: &Out, a: PlanArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    let log = console_logger(out);
    let insp = load(&a.src, out, &mut prompt, &log)?;
    let db = load_compat_db();
    let asm = assess(&db, &insp.facts);
    let plan = build_plan(&insp.facts, &a.opts.request());
    if a.json {
        let mut doc = plan_doc(&insp.facts, &plan, &asm);
        doc["kernel"] = facts::kernel_json(a.kernel.channel());
        out.json_doc(&doc);
    } else {
        out.data_lines(render::plan_report(&insp.facts, &plan, &asm));
        out.data(&format!("Kernel: {}", a.kernel.channel().label()));
    }
    verdict(&plan, &asm)
}

/// Exit-code decision shared by `plan` and `install`: unsupported hardware (5) before plan errors (4).
pub fn verdict(plan: &Plan, asm: &facts::Assessment<'_>) -> CliResult<()> {
    if let Some(why) = asm.blocking() {
        return Err(CliError::Unsupported(why));
    }
    if !plan.is_ok() {
        return Err(CliError::PlanErrors(format!("the plan has errors; nothing was changed ({})", plan.errors.join("; "))));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------- install

pub fn install(out: &Out, a: InstallArgs) -> CliResult<()> {
    let mut prompt = StdPrompter;
    if !a.dry_run && a.src.facts.is_some() {
        return Err(CliError::usage("--facts is only for --dry-run and planning: a real install always connects to the NAS (--host)"));
    }
    if a.out.is_some() && a.json {
        return Err(CliError::usage("--out writes files; --json prints a document: choose one"));
    }
    // Nobody to ask and the gate flags are plainly absent: refuse before touching the network.
    if !a.dry_run {
        gate::precheck(&a.gate, out)?;
    }
    let spec = InstallSpec::from_args(&a.system, &a.boot, &a.source, out, &mut prompt)?;
    // An offline install: the bundle must be a finished bundle before anything is contacted.
    if let Some(dir) = &spec.bundle {
        bundle::open_bundle(dir)?;
    }
    let log = console_logger(out);
    let insp = load(&a.src, out, &mut prompt, &log)?;
    let db = load_compat_db();
    let asm = assess(&db, &insp.facts);
    let plan = build_plan(&insp.facts, &a.plan.request());

    let lines = render::plan_report(&insp.facts, &plan, &asm);
    if !a.dry_run {
        for l in &lines {
            out.info(l);
        }
    }
    verdict(&plan, &asm)?;
    let (method_name, method) = resolve_method(&spec, &insp.facts)?;
    if let Some(dir) = &spec.bundle {
        // does the bundle fit this unit (architecture) and these choices (suite, kernel channel, extra packages, firmware)?
        for w in bundle::check_for_install(dir, insp.facts.arch, &spec::default_suite(), spec.kernel, &spec.packages, spec.firmware)? {
            out.warn(&w);
        }
    }

    if a.dry_run {
        let mat = build_material(&spec, &insp.facts, true, out)?;
        let host = a.src.host.as_deref();
        let rendered = dryrun::render(&mat.cfg, &plan, &insp.facts, &insp.raw, dryrun::bind_ip_for(host), spec.bundle.as_deref())
            .map_err(CliError::Other)?;
        return dryrun::deliver(
            out,
            dryrun::Delivery { json_doc: a.json, dir: a.out.as_deref() },
            &insp.facts,
            &plan,
            &lines,
            method_name,
            &rendered,
        );
    }

    let target = insp.target().cloned().ok_or_else(|| CliError::usage("an install needs --host"))?;
    gate::enforce(&a.gate, &insp.facts, &target.host, out, &mut prompt)?;
    let mat = build_material(&spec, &insp.facts, false, out)?;
    let job = build_job(&spec, &insp, plan, target, &mat, method);
    execute(out, job, &mat, method_name, a.save_logins.as_deref())
}

/// Everything after the gate: run the job with progress, then show the logins. Shared with the wizard.
pub fn execute(out: &Out, job: InstallJob, mat: &Material, method_name: &str, save_logins: Option<&Path>) -> CliResult<()> {
    out.event(json!({"event": "start", "command": "install", "version": env!("CARGO_PKG_VERSION"), "model": job.facts.model, "method": method_name}));
    out.info(&format!(
        "Starting the install of {} ({}) with method {method_name}. Ctrl-C cancels until the point of no return.",
        job.facts.model, job.target.host
    ));
    let secrets = [job.display_password.clone()];
    let (serial, key_path, key_fp) = (job.cfg.serial.clone(), mat.key_path.clone(), mat.key_fingerprint.clone());
    let (password, generated, username) = (mat.password.clone(), mat.password_generated, job.cfg.username.clone());
    let rep = run_job(out, &secrets, move |lg, cancel| {
        let mut progress = progress_fn(lg);
        session::run_install(&job, lg, cancel, &mut progress)
    });
    if let Some(err) = rep.failure() {
        return Err(err);
    }
    let outcome = rep.result.expect("checked above");
    let logins =
        Logins::new(&outcome, &username, &password, generated, &serial, key_path.as_deref(), key_fp.as_deref(), rep.log_path.as_deref());
    if let Some(p) = save_logins {
        match write_secret_file(p, &logins.text(true).join("\n")) {
            Ok(()) => out.info(&format!("logins saved to {} (readable only by you)", p.display())),
            Err(e) => out.warn(&format!("could not save the logins to {}: {e}", p.display())),
        }
    }
    out.result_text(&logins.text(false));
    out.result_event("install", true, logins.json());
    Ok(())
}

pub fn write_secret_file(p: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(p, format!("{text}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// What the person needs after the install.
pub struct Logins {
    pub hostname: String,
    pub address: Option<String>,
    pub username: String,
    pub password: String,
    pub password_generated: bool,
    pub ssh_fingerprint: Option<String>,
    pub mgmt_url: Option<String>,
    pub serial: String,
    pub key_path: Option<String>,
    pub key_fingerprint: Option<String>,
    pub log_path: Option<String>,
}

impl Logins {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        o: &Outcome,
        username: &str,
        password: &str,
        generated: bool,
        serial: &synofurb_core::model::SerialConsole,
        key_path: Option<&Path>,
        key_fp: Option<&str>,
        log_path: Option<&Path>,
    ) -> Logins {
        Logins {
            hostname: o.hostname.clone(),
            address: o.ip.map(|i| i.to_string()),
            username: username.to_string(),
            password: password.to_string(),
            password_generated: generated,
            ssh_fingerprint: o.ssh_fingerprint.clone(),
            mgmt_url: o.mgmt_url.clone(),
            serial: format!("{} at {} baud", serial_text(serial), serial.speed),
            key_path: key_path.map(|p| p.display().to_string()),
            key_fingerprint: key_fp.map(String::from),
            log_path: o.log_path.as_deref().or(log_path).map(|p| p.display().to_string()),
        }
    }

    /// `with_password`: the file form always has the password; the screen form only prints a password this tool generated.
    pub fn text(&self, with_password: bool) -> Vec<String> {
        let addr = self.address.clone().unwrap_or_else(|| "see your DHCP server".into());
        let mut v =
            vec![String::new(), "=== INSTALLED ===".into(), format!("hostname:   {}", self.hostname), format!("address:    {addr}")];
        if self.password_generated || with_password {
            v.push(format!("login:      {} / {}", self.username, self.password));
        } else {
            v.push(format!("login:      {} / (the password you chose)", self.username));
        }
        v.push(format!(
            "ssh:        ssh {}@{}   (host key {})",
            self.username,
            self.address.clone().unwrap_or_default(),
            self.ssh_fingerprint.clone().unwrap_or_default()
        ));
        if let Some(k) = &self.key_path {
            v.push(format!(
                "ssh key:    {k}   ({})   use: ssh -i {k} {}@{}",
                self.key_fingerprint.clone().unwrap_or_default(),
                self.username,
                self.address.clone().unwrap_or_default()
            ));
        }
        if let Some(u) = &self.mgmt_url {
            v.push(format!("management: {u}  (self-signed certificate)"));
        }
        v.push(format!("serial console: {} (also on USB serial adapters)", self.serial));
        if let Some(p) = &self.log_path {
            v.push(format!("debug log:  {p}"));
        }
        v
    }

    pub fn json(&self) -> serde_json::Value {
        json!({
            "hostname": self.hostname,
            "address": self.address,
            "username": self.username,
            "password": self.password_generated.then_some(&self.password),
            "password_generated": self.password_generated,
            "ssh_command": self.address.as_ref().map(|a| format!("ssh {}@{a}", self.username)),
            "ssh_host_key": self.ssh_fingerprint,
            "ssh_private_key": self.key_path,
            "ssh_key_fingerprint": self.key_fingerprint,
            "management_url": self.mgmt_url,
            "serial_console": self.serial,
            "log_path": self.log_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::model::SerialConsole;

    fn outcome() -> Outcome {
        Outcome {
            hostname: "vault".into(),
            ip: Some("192.168.1.30".parse().unwrap()),
            ssh_fingerprint: Some("SHA256:abc".into()),
            mgmt_url: Some("https://192.168.1.30:8443/".into()),
            mgmt_reachable: true,
            username: "nasadmin".into(),
            log_path: None,
        }
    }

    #[test]
    fn logins_show_a_generated_password_but_never_echo_a_chosen_one() {
        let l = Logins::new(&outcome(), "nasadmin", "Gen3ratedPw", true, &SerialConsole::default(), None, None, None);
        assert!(l.text(false).join("\n").contains("nasadmin / Gen3ratedPw"));
        assert_eq!(l.json()["password"], "Gen3ratedPw");
        let l = Logins::new(&outcome(), "nasadmin", "my own secret", false, &SerialConsole::default(), None, None, None);
        let screen = l.text(false).join("\n");
        assert!(!screen.contains("my own secret") && screen.contains("the password you chose"));
        assert!(l.json()["password"].is_null());
        assert!(l.text(true).join("\n").contains("my own secret"), "the 0600 file has it");
    }

    #[test]
    fn logins_json_shape() {
        let l = Logins::new(
            &outcome(),
            "nasadmin",
            "x",
            true,
            &SerialConsole::default(),
            Some(Path::new("/k/vault_ed25519")),
            Some("SHA256:k"),
            None,
        );
        let j = l.json();
        for k in [
            "hostname",
            "address",
            "username",
            "password",
            "ssh_command",
            "ssh_host_key",
            "ssh_private_key",
            "management_url",
            "serial_console",
            "log_path",
        ] {
            assert!(j.get(k).is_some(), "{k}");
        }
        assert_eq!(j["ssh_command"], "ssh nasadmin@192.168.1.30");
        assert_eq!(j["serial_console"], "ttyS0 at 115200 baud");
    }
}
