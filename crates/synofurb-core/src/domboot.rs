//! Booting Debian (or any OS) on x86 Synology units through the internal boot flash (DOM).
//!
//! Flow (everything over SSH to DSM, nothing is opened or plugged in):
//! 1. back up the whole DOM to the user's computer (streamed, hashed, verified against the NAS),
//! 2. upload `synofurb-dom` + the small boot **agent** (kernel + initrd + per-unit overlay) and add the
//!    agent as a boot entry, armed for the next boot only,
//! 3. reboot DSM; the agent resets the default to DSM as its first action (fail-safe), brings up SSH,
//! 4. connect to the agent and `kexec` the full Debian installer (or any other OS) from this computer.

use crate::config::{generate_ssh_keypair, random_token};
use crate::debian::{cpio_gz, sha256_hex};
use crate::hostkey::HostKeyPolicy;
use crate::logs::Logger;
use crate::model::Arch;
use crate::remote::{Auth, Privileged, RemoteShell, SshSession};
use crate::session::SessionError;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::net::{IpAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// UDP port on which the agent announces itself.
pub const BEACON_PORT: u16 = 9988;

/// Writer that hashes everything that passes through.
struct HashWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    bytes: u64,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.hasher.update(&b[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Debug, Clone)]
pub struct DomBackup {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

/// Stream the whole DOM (`/dev/synoboot`) into `dest_dir` and verify it against a hash computed on the NAS.
/// The backup is what makes every later step reversible, so a mismatch is fatal.
///
/// Any failure is a [`SessionError::FlashBackupFailed`] (find it with `SessionError::find`): the flash has not been written to at
/// that point, which is different from every later failure.
pub fn backup_dom(
    sh: &mut dyn RemoteShell,
    is_root: bool,
    sudo_pw: Option<String>,
    dest_dir: &Path,
    unit: &str,
    expected_bytes: u64,
    log: &Logger,
) -> Result<DomBackup> {
    backup_dom_inner(sh, is_root, sudo_pw, dest_dir, unit, expected_bytes, log)
        .map_err(|e| anyhow::Error::new(SessionError::FlashBackupFailed { reason: format!("{e:#}") }))
}

fn backup_dom_inner(
    sh: &mut dyn RemoteShell,
    is_root: bool,
    sudo_pw: Option<String>,
    dest_dir: &Path,
    unit: &str,
    expected_bytes: u64,
    log: &Logger,
) -> Result<DomBackup> {
    std::fs::create_dir_all(dest_dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let safe: String = unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    let path = dest_dir.join(format!("synofurb-dom-backup-{safe}-{stamp}.img"));
    log.info(format!("backing up the internal boot flash to {} ({} MiB)", path.display(), expected_bytes / 1048576));
    let file = std::fs::File::create(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    let mut w = HashWriter { inner: std::io::BufWriter::new(file), hasher: Sha256::new(), bytes: 0 };
    let mut p = Privileged { sh, is_root, sudo_password: sudo_pw };
    let (code, err) = p.run_to("dd if=/dev/synoboot bs=64k 2>/dev/null", &mut w)?;
    w.flush()?;
    if code != 0 {
        let _ = std::fs::remove_file(&path);
        bail!("reading /dev/synoboot failed (exit {code}): {}", err.trim());
    }
    if expected_bytes > 0 && w.bytes != expected_bytes {
        let _ = std::fs::remove_file(&path);
        bail!("DOM backup is {} bytes but the device is {} bytes", w.bytes, expected_bytes);
    }
    let (bytes, local) = (w.bytes, hex::encode(w.hasher.finalize()));
    // Independent check: hash computed on the NAS itself.
    let r = p.run("sha256sum /dev/synoboot 2>/dev/null || openssl dgst -sha256 /dev/synoboot 2>/dev/null")?;
    let remote = r.stdout.split_whitespace().find(|t| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_string);
    match remote {
        Some(h) if h == local => log.info(format!("DOM backup verified: sha256 {local}")),
        Some(h) => {
            let _ = std::fs::remove_file(&path);
            bail!("DOM backup does not match the device (local {local}, NAS {h}); nothing was changed");
        }
        None => log.warn("the NAS has no sha256 tool; the DOM backup was verified by size only"),
    }
    std::fs::write(path.with_extension("img.sha256"), format!("{local}  {}\n", path.file_name().unwrap().to_string_lossy()))?;
    Ok(DomBackup { path, sha256: local, bytes })
}

/// Files of the boot agent for one architecture.
#[derive(Debug, Clone)]
pub struct AgentFiles {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
}

impl AgentFiles {
    /// `agent-<arch>.vmlinuz` / `agent-<arch>.initrd` in `dir`, `assets/agent/` or `$SYNOFURB_AGENT_DIR`.
    pub fn locate(arch: Arch, dir: Option<&Path>) -> Result<AgentFiles> {
        let mut dirs: Vec<PathBuf> = vec![];
        if let Some(d) = dir {
            dirs.push(d.to_path_buf());
        }
        if let Ok(d) = std::env::var("SYNOFURB_AGENT_DIR") {
            dirs.push(d.into());
        }
        dirs.push("assets/agent".into());
        if let Some(d) = std::env::current_exe().ok().as_deref().and_then(Path::parent) {
            dirs.push(d.join("assets/agent"));
        }
        let a = arch.debian();
        for d in &dirs {
            let (k, i) = (d.join(format!("agent-{a}.vmlinuz")), d.join(format!("agent-{a}.initrd")));
            if k.is_file() && i.is_file() {
                return Ok(AgentFiles { kernel: k, initrd: i });
            }
        }
        bail!(
            "boot agent files for {a} not found (looked in {}). Build them with boot/agent/build.sh {a} or download the release assets 'synofurb-agent-{a}-<version>.vmlinuz/.initrd'.",
            dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
        )
    }
}

impl AgentFiles {
    /// Release asset names for one architecture and version (the release workflow's naming contract).
    pub fn release_asset_names(arch: Arch, version: &str) -> (String, String) {
        let a = arch.debian();
        (format!("synofurb-agent-{a}-{version}.vmlinuz"), format!("synofurb-agent-{a}-{version}.initrd"))
    }

    /// `UpdateInfo` that points the verified downloader at one asset of the release `v<version>`.
    fn release_info(repo: &str, version: &str, asset: &str) -> crate::updater::UpdateInfo {
        use crate::updater::{AssetInfo, SIGNATURE_ASSET, SUMS_ASSET};
        let at = |name: &str| AssetInfo {
            name: name.to_string(),
            url: format!("https://github.com/{repo}/releases/download/v{version}/{name}"),
            size: 0,
        };
        crate::updater::UpdateInfo {
            current: version.to_string(),
            latest: version.to_string(),
            tag: format!("v{version}"),
            update_available: false,
            channel: crate::updater::Channel::Stable,
            prerelease: false,
            published_at: None,
            notes: String::new(),
            html_url: None,
            asset: Some(at(asset)),
            checksums: Some(at(SUMS_ASSET)),
            signature: Some(at(SIGNATURE_ASSET)),
            problems: vec![],
        }
    }

    /// Download the agent for `arch` from the GitHub release matching this program's version. Both files are verified
    /// (minisign signature of `SHA256SUMS`, then SHA-256) exactly like a program update, then stored as
    /// `agent-<arch>.vmlinuz` / `agent-<arch>.initrd` in `dest_dir`.
    pub fn fetch_release(
        arch: Arch,
        version: &str,
        dest_dir: &Path,
        log: &Logger,
        progress: &mut dyn FnMut(&str, f32),
    ) -> Result<AgentFiles> {
        let (k_name, i_name) = Self::release_asset_names(arch, version);
        let a = arch.debian();
        std::fs::create_dir_all(dest_dir)?;
        let mut out = vec![];
        for (name, local) in [(k_name, format!("agent-{a}.vmlinuz")), (i_name, format!("agent-{a}.initrd"))] {
            log.info(format!("downloading {name} from the synofurb release v{version}"));
            let info = Self::release_info(crate::updater::DEFAULT_REPO, version, &name);
            let dl = crate::updater::download_verified(&info, dest_dir, &mut |p| {
                if let Some(t) = p.total {
                    progress(&name, p.downloaded as f32 / t.max(1) as f32);
                }
            })
            .map_err(|e| anyhow::anyhow!("could not fetch {name}: {e}"))?;
            std::fs::rename(&dl.path, dest_dir.join(&local))?;
            out.push(dest_dir.join(local));
        }
        Ok(AgentFiles { kernel: out.remove(0), initrd: out.remove(0) })
    }

    /// Use local files when present, otherwise download the matching release into `cache_dir`.
    pub fn locate_or_fetch(
        arch: Arch,
        dir: Option<&Path>,
        cache_dir: &Path,
        log: &Logger,
        progress: &mut dyn FnMut(&str, f32),
    ) -> Result<AgentFiles> {
        match Self::locate(arch, dir) {
            Ok(f) => Ok(f),
            Err(local_err) => Self::fetch_release(arch, env!("CARGO_PKG_VERSION"), &cache_dir.join("agent"), log, progress)
                .map_err(|net_err| anyhow::anyhow!("{local_err}\nAnd downloading it failed: {net_err}")),
        }
    }
}

/// Per-unit settings baked into the agent (appended to its initrd as a cpio overlay).
pub struct AgentOverlay {
    pub hostname: String,
    pub timeout_min: u32,
    pub authorized_keys: Vec<String>,
}

impl AgentOverlay {
    pub fn render_conf(&self) -> String {
        format!("AGENT_HOSTNAME='{}'\nAGENT_TIMEOUT_MIN={}\n", self.hostname.replace('\'', ""), self.timeout_min)
    }
    pub fn cpio(&self) -> Vec<u8> {
        let keys = self.authorized_keys.join("\n") + "\n";
        cpio_gz(&[
            ("etc/synofurb-agent.conf", self.render_conf().as_bytes(), 0o100600),
            ("etc/synofurb-authorized_keys", keys.as_bytes(), 0o100600),
        ])
    }
}

/// Upload the DOM manager, the agent and its overlay to DSM and register the agent entry.
/// `arm = true` makes the next boot go to the agent (it resets the default to DSM itself).
#[allow(clippy::too_many_arguments)]
pub fn install_agent(
    sh: &mut dyn RemoteShell,
    is_root: bool,
    sudo_pw: Option<String>,
    dom_script: &str,
    files: &AgentFiles,
    overlay: &AgentOverlay,
    cmdline: &str,
    arm: bool,
    log: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<()> {
    if cmdline.contains(['\'', '\n']) {
        bail!("unsafe kernel command line");
    }
    let mut initrd = std::fs::read(&files.initrd).with_context(|| format!("reading {}", files.initrd.display()))?;
    initrd.extend(overlay.cpio());
    let kernel = std::fs::read(&files.kernel).with_context(|| format!("reading {}", files.kernel.display()))?;
    let dir = format!("/tmp/.synofurb-dom-{}", random_token(4));
    sh.exec(&format!("mkdir -p '{dir}' && chmod 700 '{dir}'"), None)?;
    for (name, data, mode) in
        [("synofurb-dom", dom_script.as_bytes(), 0o700u32), ("agent.vmlinuz", &kernel[..], 0o600), ("agent.initrd", &initrd[..], 0o600)]
    {
        let remote = format!("{dir}/{name}");
        let mut cur = std::io::Cursor::new(data.to_vec());
        sh.upload(&remote, &mut cur, data.len() as u64, mode, &mut |sent| progress(name, sent as f32 / data.len().max(1) as f32))?;
        let r = sh.exec(&format!("sha256sum '{remote}' 2>/dev/null"), None)?;
        let got = r.stdout.split_whitespace().next().unwrap_or("");
        if !got.is_empty() && got != sha256_hex(data) {
            bail!("checksum mismatch after uploading {name}");
        }
    }
    let mut p = Privileged { sh, is_root, sudo_password: sudo_pw };
    let st = p.run(&format!("/bin/sh '{dir}/synofurb-dom' status"))?;
    log.info(format!("boot flash status:\n{}", st.stdout.trim()));
    if !st.ok() {
        bail!("synofurb-dom refuses this unit: {} {}", st.stdout.trim(), st.stderr.trim());
    }
    let r = p.run(&format!(
        "/bin/sh '{dir}/synofurb-dom' install-agent '{dir}/agent.vmlinuz' '{dir}/agent.initrd' --cmdline '{cmdline}'{} --yes",
        if arm { " --arm" } else { "" }
    ))?;
    if !r.ok() {
        bail!("installing the boot agent failed: {} {}", r.stdout.trim(), r.stderr.trim());
    }
    log.info(r.stdout.trim());
    // Keep a copy of the manager on DSM for later recovery.
    let _ = p.run(&format!("mkdir -p /usr/local/bin 2>/dev/null; cp '{dir}/synofurb-dom' /usr/local/bin/synofurb-dom 2>/dev/null; rm -rf '{dir}/agent.vmlinuz' '{dir}/agent.initrd'"));
    Ok(())
}

/// How long to wait for the agent, and the watchdog the agent was armed with (for the text of the timeout error).
#[derive(Debug, Clone, Copy)]
pub struct AgentWait {
    pub timeout: Duration,
    pub watchdog_min: u32,
}

impl AgentWait {
    /// The standard install wait ([`crate::timing::AGENT_WAIT_MIN`]) for an agent armed with `watchdog_min`.
    pub fn standard(watchdog_min: u32) -> AgentWait {
        AgentWait { timeout: Duration::from_secs(crate::timing::AGENT_WAIT_MIN as u64 * 60), watchdog_min }
    }
}

/// Wait until the agent answers over SSH. DSM's own sshd also listens on port 22, so a successful key login with
/// the agent's per-unit key (DSM does not know it) and a changed host key identify the agent.
///
/// The agent's host key is new at every boot and the login is by our per-unit key (no password is sent), so this connection uses
/// [`HostKeyPolicy::KeyAuthOnly`]. On timeout the error is [`SessionError::AgentTimeout`].
pub fn wait_for_agent(
    candidates: &[IpAddr],
    agent_key_pem: &str,
    dsm_fingerprint: Option<&str>,
    wait: AgentWait,
    cancel: &AtomicBool,
    log: &Logger,
) -> Result<(SshSession, IpAddr)> {
    let timeout = wait.timeout;
    let deadline = Instant::now() + timeout;
    let mut last_log = Instant::now() - Duration::from_secs(60);
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled while waiting for the boot agent");
        }
        for ip in candidates {
            if TcpStream::connect_timeout(&std::net::SocketAddr::new(*ip, 22), Duration::from_secs(2)).is_err() {
                continue;
            }
            if let Ok(fp) = crate::remote::fetch_host_fingerprint(&ip.to_string(), 22, Duration::from_secs(6)) {
                if dsm_fingerprint == Some(fp.as_str()) {
                    continue; // still DSM (reboot not started yet)
                }
                if let Ok(s) = SshSession::connect_with(
                    &ip.to_string(),
                    22,
                    "root",
                    &Auth::Key(agent_key_pem.to_string()),
                    log,
                    &HostKeyPolicy::KeyAuthOnly,
                ) {
                    log.info(format!("boot agent is up at {ip} (host key {fp})"));
                    return Ok((s, *ip));
                }
            }
        }
        if last_log.elapsed() > Duration::from_secs(30) {
            log.info("waiting for the NAS to reboot into the boot agent (usually 1-3 minutes)...");
            last_log = Instant::now();
        }
        std::thread::sleep(Duration::from_secs(4));
    }
    Err(anyhow::Error::new(SessionError::AgentTimeout { waited_min: (timeout.as_secs() / 60) as u32, watchdog_min: wait.watchdog_min }))
}

/// A throw-away key pair for the agent session of this unit.
pub fn agent_session_key(unit: &str) -> Result<crate::config::SshKeypair> {
    generate_ssh_keypair(&format!("synofurb-agent-{unit}"))
}

/// `sf-boot kexec` command line on the agent for a kernel/initrd served by this computer.
///
/// The agent's busybox `wget` does not verify TLS and the installer is fetched over plain HTTP from this computer, so the
/// agent REFUSES to load URLs without checksums: the hashes of both files are passed and checked there. `exec` is detached
/// because the SSH connection dies the moment the new kernel starts.
pub fn agent_kexec_command(kernel_url: &str, initrd_url: &str, cmdline: &str, kernel_sha256: &str, initrd_sha256: &str) -> Result<String> {
    for v in [kernel_url, initrd_url, cmdline] {
        if v.contains(['\'', '\n']) {
            bail!("unsafe characters in kexec arguments");
        }
    }
    for h in [kernel_sha256, initrd_sha256] {
        if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("kexec arguments need SHA-256 hex digests");
        }
    }
    Ok(format!(
        "sf-boot kexec '{kernel_url}' '{initrd_url}' --cmdline '{cmdline}' --sha256 kernel={kernel_sha256},initrd={initrd_sha256} && sf-boot exec --detach --delay 3"
    ))
}

/// Ask DSM (or Debian) to boot DSM next time: the recovery command.
pub fn default_to_dsm(sh: &mut dyn RemoteShell, is_root: bool, sudo_pw: Option<String>) -> Result<String> {
    let mut p = Privileged { sh, is_root, sudo_password: sudo_pw };
    let r = p.run("synofurb-dom default dsm --yes || /usr/local/bin/synofurb-dom default dsm --yes")?;
    if !r.ok() {
        bail!("could not set the default boot entry to DSM: {} {}", r.stdout.trim(), r.stderr.trim());
    }
    Ok(r.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::mock::MockShell;

    #[test]
    fn backup_streams_binary_and_verifies_against_the_nas_hash() {
        let dir = std::env::temp_dir().join(format!("sf-domb-{}", std::process::id()));
        let mut m = MockShell::default();
        let data = "hello dom";
        m.push_ok(data); // dd stream
        m.push_ok(&format!("{}  /dev/synoboot\n", sha256_hex(data.as_bytes())));
        let b = backup_dom(&mut m, true, None, &dir, "DS923+", data.len() as u64, &Logger::memory(None)).unwrap();
        assert_eq!(std::fs::read(&b.path).unwrap(), data.as_bytes());
        assert!(b.path.with_extension("img.sha256").exists());
        assert!(m.commands[0].contains("dd if=/dev/synoboot"));
    }

    #[test]
    fn backup_mismatch_deletes_the_file_and_fails() {
        let dir = std::env::temp_dir().join(format!("sf-domb2-{}", std::process::id()));
        let mut m = MockShell::default();
        m.push_ok("hello dom");
        m.push_ok(&format!("{}  /dev/synoboot\n", "0".repeat(64)));
        let e = backup_dom(&mut m, true, None, &dir, "x", 9, &Logger::memory(None)).unwrap_err();
        assert!(e.to_string().contains("does not match"), "{e}");
        assert!(std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) == 0, "bad backup must not be left behind");
    }

    #[test]
    fn every_backup_failure_is_the_typed_flash_untouched_error() {
        let dir = std::env::temp_dir().join(format!("sf-domb4-{}", std::process::id()));
        let mut m = MockShell::default();
        m.push_ok("hello dom");
        m.push_ok(&format!("{}  /dev/synoboot\n", "0".repeat(64))); // NAS hash differs
        let e = backup_dom(&mut m, true, None, &dir, "x", 9, &Logger::memory(None)).unwrap_err();
        let t = SessionError::find(&e).expect("typed");
        assert!(matches!(t, SessionError::FlashBackupFailed { reason } if reason.contains("does not match")), "{e}");
        assert!(t.flash_untouched() && !t.boot_default_unconfirmed());
        assert!(e.to_string().contains("nothing was written to it"), "{e}");
        let mut m = MockShell::default();
        m.push_ok("short");
        let e = backup_dom(&mut m, true, None, &dir, "x", 1000, &Logger::memory(None)).unwrap_err();
        assert!(SessionError::find(&e).is_some_and(SessionError::flash_untouched), "{e}");
    }

    #[test]
    fn waiting_for_the_agent_times_out_with_the_typed_error_carrying_the_watchdog() {
        let w = AgentWait { timeout: Duration::from_millis(1), watchdog_min: crate::timing::INSTALL_WATCHDOG_MIN };
        let Err(e) = wait_for_agent(&[], "", None, w, &AtomicBool::new(false), &Logger::memory(None)) else {
            panic!("no agent was started")
        };
        assert_eq!(SessionError::find(&e), Some(&SessionError::AgentTimeout { waited_min: 0, watchdog_min: 30 }));
        assert_eq!(AgentWait::standard(60).timeout, Duration::from_secs(600));
    }

    #[test]
    fn short_backup_is_rejected() {
        let dir = std::env::temp_dir().join(format!("sf-domb3-{}", std::process::id()));
        let mut m = MockShell::default();
        m.push_ok("short");
        assert!(backup_dom(&mut m, true, None, &dir, "x", 1000, &Logger::memory(None)).is_err());
    }

    #[test]
    fn install_agent_uploads_verifies_and_arms() {
        let dir = std::env::temp_dir().join(format!("sf-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agent-amd64.vmlinuz"), b"KERNEL").unwrap();
        std::fs::write(dir.join("agent-amd64.initrd"), b"INITRD").unwrap();
        let files = AgentFiles::locate(Arch::X86_64, Some(&dir)).unwrap();
        let ov = AgentOverlay {
            hostname: "nas".into(),
            timeout_min: crate::timing::AGENT_DEFAULT_WATCHDOG_MIN,
            authorized_keys: vec!["ssh-ed25519 AAAA test".into()],
        };
        let mut m = MockShell::default();
        m.push_ok(""); // mkdir
        // three uploads each followed by a sha256sum that we let succeed with an empty answer (no tool)
        m.push_ok("");
        m.push_ok("");
        m.push_ok("");
        m.push_ok("device /dev/synoboot ok\n"); // status
        m.push_ok("agent installed\n"); // install-agent
        install_agent(
            &mut m,
            true,
            None,
            "#!/bin/sh\n",
            &files,
            &ov,
            "console=ttyS0,115200n8",
            true,
            &Logger::memory(None),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(m.uploads.len(), 3);
        assert_eq!(m.uploads[2].0.split('/').next_back(), Some("agent.initrd"));
        assert!(m.uploads[2].1.starts_with(b"INITRD"), "overlay is appended after the base initrd");
        assert!(m.commands.iter().any(|c| c.contains("install-agent") && c.contains("--arm") && c.contains("--yes")));
        assert!(AgentFiles::locate(Arch::Armhf, Some(&dir)).is_err());
    }

    #[test]
    fn agent_release_assets_follow_the_release_workflow_contract() {
        let (k, i) = AgentFiles::release_asset_names(Arch::Armhf, "0.2.0");
        assert_eq!((k.as_str(), i.as_str()), ("synofurb-agent-armhf-0.2.0.vmlinuz", "synofurb-agent-armhf-0.2.0.initrd"));
        let info = AgentFiles::release_info("andreaswiren/synofurb", "0.2.0", &k);
        assert_eq!(
            info.asset.as_ref().unwrap().url,
            "https://github.com/andreaswiren/synofurb/releases/download/v0.2.0/synofurb-agent-armhf-0.2.0.vmlinuz"
        );
        assert!(info.installable(), "needs SHA256SUMS and its signature next to the asset");
        // the downloader must refuse anything but the release host: policy is exercised in updater's own tests
    }

    #[test]
    fn unsafe_arguments_are_refused() {
        let h = "a".repeat(64);
        let c = agent_kexec_command("http://a/k", "http://a/i", "auto=true console=ttyS0", &h, &h).unwrap();
        assert!(c.contains(&format!("--sha256 kernel={h},initrd={h}")) && c.ends_with("sf-boot exec --detach --delay 3"));
        assert!(agent_kexec_command("http://a/k'; reboot", "http://a/i", "x", &h, &h).is_err());
        assert!(
            agent_kexec_command("http://a/k", "http://a/i", "x", "short", &h).is_err(),
            "the agent refuses unverified downloads, so do not even build the command"
        );
        let ov = AgentOverlay { hostname: "n'x".into(), timeout_min: 5, authorized_keys: vec![] };
        assert!(!ov.render_conf().contains("n'x"));
    }
}
