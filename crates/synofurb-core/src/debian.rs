//! Debian installer assets: download + verify netboot kernel/initrd, build the preseed, and bake
//! the preseed (and optionally non-free firmware) into the initrd.
//!
//! Verification chain: `InRelease` (fetched over HTTPS) lists the SHA-256 of
//! `installer-<arch>/current/images/SHA256SUMS`, which lists the SHA-256 of each file. If `gpgv` and
//! Debian's archive keyring exist on this machine, `InRelease` is also PGP-verified.

use crate::config::InstallConfig;
use crate::logs::Logger;
use crate::model::{Arch, SerialConsole};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DEBIAN_BASE: &str = "https://deb.debian.org/debian";
pub const FIRMWARE_BASE: &str = "https://cdimage.debian.org/cdimage/firmware";

/// Paths (relative to `.../images/`) of the netboot kernel and initrd.
pub fn netboot_paths(arch: Arch) -> Option<(&'static str, &'static str)> {
    match arch {
        Arch::X86_64 => Some(("./netboot/debian-installer/amd64/linux", "./netboot/debian-installer/amd64/initrd.gz")),
        Arch::Arm64 => Some(("./netboot/debian-installer/arm64/linux", "./netboot/debian-installer/arm64/initrd.gz")),
        Arch::Armhf => Some(("./netboot/vmlinuz", "./netboot/initrd.gz")),
        Arch::Unsupported => None,
    }
}

/// Find `path`'s hash in a `Release`/`InRelease` file's SHA256 section.
pub fn release_sha256(release: &str, path: &str) -> Option<String> {
    let mut in_sha256 = false;
    for line in release.lines() {
        if line.starts_with("SHA256:") {
            in_sha256 = true;
            continue;
        }
        if !line.starts_with(' ') {
            in_sha256 = false;
            continue;
        }
        if in_sha256 {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() == 3 && f[2] == path && f[0].len() == 64 && f[0].chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(f[0].to_string());
            }
        }
    }
    None
}

/// Find a file's hash in a `sha256sum`-style list.
pub fn sums_sha256(sums: &str, path: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (h, p) = l.split_once("  ")?;
        (p.trim() == path).then(|| h.to_string())
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent("synofurb")
        .build()
}

fn get_text(url: &str) -> Result<String> {
    Ok(agent().get(url).call().with_context(|| format!("GET {url}"))?.into_string()?)
}

/// Download `url` into `dest`, verifying `sha256`. Re-uses a cached file with the right hash.
pub fn download_verified(url: &str, dest: &Path, sha256: &str, log: &Logger, progress: &mut dyn FnMut(f32)) -> Result<()> {
    if let Ok(existing) = std::fs::read(dest)
        && sha256_hex(&existing) == sha256
    {
        log.info(format!("cached and verified: {}", dest.display()));
        progress(1.0);
        return Ok(());
    }
    log.info(format!("downloading {url}"));
    let resp = agent().get(url).call().with_context(|| format!("GET {url}"))?;
    let total: u64 = resp.header("content-length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut r = resp.into_reader();
    let mut hasher = Sha256::new();
    let tmp = dest.with_extension("part");
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut f = std::fs::File::create(&tmp)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        f.write_all(&buf[..n])?;
        got += n as u64;
        if total > 0 {
            progress(got as f32 / total as f32);
        }
    }
    f.flush()?;
    let h = hex::encode(hasher.finalize());
    if h != sha256 {
        let _ = std::fs::remove_file(&tmp);
        bail!("checksum mismatch for {url}: expected {sha256}, got {h}");
    }
    std::fs::rename(&tmp, dest)?;
    log.info(format!("verified sha256 {h} ({got} bytes)"));
    Ok(())
}

/// Best-effort PGP verification of `InRelease` with the local `gpgv` and Debian's keyring.
pub fn gpg_verify_inrelease(text: &str, log: &Logger) -> bool {
    let keyring = ["/usr/share/keyrings/debian-archive-keyring.pgp", "/usr/share/keyrings/debian-archive-keyring.gpg"]
        .into_iter()
        .find(|p| Path::new(p).exists());
    let Some(keyring) = keyring else {
        log.warn("PGP check of the Debian Release file skipped: debian-archive-keyring is not installed on this machine (HTTPS + SHA-256 chain only)");
        return false;
    };
    let dir = std::env::temp_dir().join(format!("sf-gpg-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let f = dir.join("InRelease");
    if std::fs::write(&f, text).is_err() {
        return false;
    }
    match std::process::Command::new("gpgv").args(["--keyring", keyring]).arg(&f).output() {
        Ok(o) if o.status.success() => {
            log.info("PGP signature of the Debian Release file verified");
            true
        }
        Ok(o) => {
            log.warn(format!("PGP verification of the Debian Release file did not pass ({}). Most often this computer's debian-archive-keyring is older than the trixie signing key; continuing on the HTTPS + SHA-256 chain only. Update debian-archive-keyring to get the signature check.", String::from_utf8_lossy(&o.stderr).trim()));
            false
        }
        Err(_) => {
            log.warn("PGP check skipped: gpgv is not installed on this machine");
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallerFiles {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub firmware: Option<PathBuf>,
    pub dtb: Option<PathBuf>,
}

pub fn fetch_installer(
    arch: Arch,
    suite: &str,
    cache: &Path,
    with_firmware: bool,
    dtb: Option<&str>,
    log: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<InstallerFiles> {
    let (kpath, ipath) = netboot_paths(arch).context("no installer for this architecture")?;
    let da = arch.debian();
    log.info(format!("fetching the Debian {suite} installer for {da}"));
    let inrelease = get_text(&format!("{DEBIAN_BASE}/dists/{suite}/InRelease"))?;
    gpg_verify_inrelease(&inrelease, log);
    let rel_path = format!("main/installer-{da}/current/images/SHA256SUMS");
    let want = release_sha256(&inrelease, &rel_path).with_context(|| format!("{rel_path} not listed in InRelease"))?;
    let sums = get_text(&format!("{DEBIAN_BASE}/dists/{suite}/main/installer-{da}/current/images/SHA256SUMS"))?;
    if sha256_hex(sums.as_bytes()) != want {
        bail!("installer SHA256SUMS does not match the hash in the signed Release file");
    }
    log.info("installer SHA256SUMS matches the Release file");
    let dir = cache.join(format!("debian-{suite}-{da}"));
    let base = format!("{DEBIAN_BASE}/dists/{suite}/main/installer-{da}/current/images");
    let get = |rel: &str, name: &str, progress: &mut dyn FnMut(&str, f32)| -> Result<PathBuf> {
        let h = sums_sha256(&sums, rel).with_context(|| format!("{rel} not in SHA256SUMS"))?;
        let dest = dir.join(name);
        download_verified(&format!("{base}/{}", rel.trim_start_matches("./")), &dest, &h, log, &mut |f| progress(name, f))?;
        Ok(dest)
    };
    let kernel = get(kpath, "kernel", progress)?;
    let initrd = get(ipath, "initrd.gz", progress)?;
    let dtb = match (arch, dtb) {
        (Arch::Armhf, Some(d)) => {
            let rel = format!("./device-tree/{d}");
            match sums_sha256(&sums, &rel) {
                Some(_) => Some(get(&rel, d, progress)?),
                None => {
                    log.warn(format!("{d} is not shipped with Debian's installer; supply your own DTB"));
                    None
                }
            }
        }
        _ => None,
    };
    let firmware = if with_firmware {
        let url = format!("{FIRMWARE_BASE}/{suite}/current");
        let sums = get_text(&format!("{url}/SHA256SUMS"))?;
        let h = sums_sha256(&sums, "firmware.cpio.gz").context("firmware.cpio.gz not listed")?;
        let dest = dir.join("firmware.cpio.gz");
        download_verified(&format!("{url}/firmware.cpio.gz"), &dest, &h, log, &mut |f| progress("firmware.cpio.gz", f))?;
        Some(dest)
    } else {
        None
    };
    Ok(InstallerFiles { kernel, initrd, firmware, dtb })
}

// --------------------------------------------------------------------------------------------
// preseed + initrd
// --------------------------------------------------------------------------------------------

pub struct PreseedParams<'a> {
    pub cfg: &'a InstallConfig,
    /// `http://host:port/token`
    pub http_base: &'a str,
    /// Password of the installer's own SSH server (Debian's `network-console`, `ssh installer@<NAS>`), so a stuck install can be looked
    /// into without a screen. `None` leaves it off (the offline mirror may not carry the component). The password lives only for the
    /// installer session, in RAM on the NAS.
    pub debug_ssh_password: Option<&'a str>,
}

pub fn render_preseed(p: &PreseedParams) -> String {
    let c = p.cfg;
    format!(
        "# synofurb preseed - generated. It only configures the installer environment (network, mirror);\n\
         # disks, packages and users are handled by the stage-2 script it downloads.\n\
         d-i debian-installer/locale string {locale}\n\
         d-i debian-installer/language string en\n\
         d-i debian-installer/country string US\n\
         d-i keyboard-configuration/xkb-keymap select {keymap}\n\
         d-i netcfg/choose_interface select auto\n\
         d-i netcfg/disable_autoconfig boolean false\n\
         d-i netcfg/dhcp_timeout string 60\n\
         d-i netcfg/link_wait_timeout string 30\n\
         d-i netcfg/get_hostname string {host}\n\
         d-i netcfg/get_domain string local\n\
         d-i netcfg/hostname string {host}\n\
         d-i hw-detect/load_firmware boolean true\n\
         d-i mirror/country string manual\n\
         d-i mirror/protocol string http\n\
         d-i mirror/http/hostname string {mirror}\n\
         d-i mirror/http/directory string {dir}\n\
         d-i mirror/http/proxy string\n\
         d-i mirror/suite string {suite}\n\
         d-i clock-setup/utc boolean true\n\
         d-i clock-setup/ntp boolean false\n\
         d-i time/zone string {tz}\n\
         d-i preseed/early_command string logger -t synofurb \"preseed loaded; stage 2 starts at partman\"\n\
         d-i partman/early_command string wget -q -O /tmp/synofurb-stage2.sh {base}/stage2.sh && sh /tmp/synofurb-stage2.sh\n{debug}",
        locale = c.locale,
        keymap = c.keymap,
        host = c.hostname,
        mirror = c.mirror_host,
        dir = c.mirror_dir,
        suite = c.suite,
        tz = c.timezone,
        base = p.http_base,
        debug = match p.debug_ssh_password {
            Some(pw) => format!(
                "# the installer's SSH server (network-console), loaded from the mirror once the network is up: ssh installer@<NAS>\n\
                 d-i anna/choose_modules string network-console\n\
                 d-i network-console/password password {pw}\n\
                 d-i network-console/password-again password {pw}\n"
            ),
            None => String::new(),
        },
    )
}

/// Kernel command line for the installer.
pub fn installer_cmdline(serial: &SerialConsole, log_host: &str, log_udp_port: u16, verbose: bool, preseed_url: Option<&str>) -> String {
    let mut a = vec![
        "auto=true".to_string(),
        "priority=critical".into(),
        "DEBIAN_FRONTEND=text".into(),
        format!("log_host={log_host}"),
        format!("log_port={log_udp_port}"),
        "console=tty0".into(),
        serial.kernel_arg(),
    ];
    if let Some(u) = preseed_url {
        a.push(format!("url={u}"));
    }
    if verbose {
        a.push("DEBCONF_DEBUG=5".into());
        a.push("BOOT_DEBUG=2".into());
    }
    a.join(" ")
}

/// A `newc` cpio archive containing the given files, gzip-compressed (appendable to an initrd).
pub fn cpio_gz(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ino = 1u32;
    let mut put = |name: &str, data: &[u8], mode: u32, out: &mut Vec<u8>| {
        let namesz = name.len() + 1;
        out.extend_from_slice(
            format!(
                "070701{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}",
                ino,
                mode,
                0,
                0,
                1,
                0,
                data.len(),
                0,
                0,
                0,
                0,
                namesz,
                0
            )
            .as_bytes(),
        );
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        let pad = |n: usize| (4 - n % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad(110 + namesz)));
        out.extend_from_slice(data);
        out.extend(std::iter::repeat_n(0u8, pad(data.len())));
        ino += 1;
    };
    for (n, d, m) in files {
        put(n, d, *m, &mut out);
    }
    put("TRAILER!!!", &[], 0, &mut out);
    use flate2::{Compression, write::GzEncoder};
    let mut gz = GzEncoder::new(Vec::new(), Compression::new(6));
    gz.write_all(&out).unwrap();
    gz.finish().unwrap()
}

/// Where the installer image gets `synofurb-dom` (relative to its root). The generated installer script looks for it by absolute path in
/// `/usr/local/sbin` and the other usual directories, **not** only on PATH.
pub const INSTALLER_DOM_TOOL: &str = "usr/local/sbin/synofurb-dom";

/// The installer images built by [`build_initrd`] always carry `synofurb-dom` (it is what lets the failure handler reboot to DSM).
pub fn installer_has_dom_tool() -> bool {
    INSTALLER_DOM_TOOL.starts_with("usr/local/sbin/") && !crate::payload::DOM_SCRIPT.is_empty()
}

/// Concatenate `initrd.gz` + preseed cpio (+ firmware cpio): the kernel unpacks all of them.
pub fn build_initrd(files: &InstallerFiles, preseed: &str, out: &Path) -> Result<()> {
    let mut data = std::fs::read(&files.initrd)?;
    // The DOM manager rides along so the installer can fall back to DSM on failure and register Debian on success.
    data.extend(cpio_gz(&[
        ("preseed.cfg", preseed.as_bytes(), 0o100600),
        (INSTALLER_DOM_TOOL, crate::payload::DOM_SCRIPT.as_bytes(), 0o100755),
    ]));
    if let Some(fw) = &files.firmware {
        data.extend(std::fs::read(fw)?);
    }
    std::fs::write(out, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = "Origin: Debian\nSHA256:\n f41c75389c2c011b8c4b6205ea63c53c    80699 main/installer-amd64/current/images/SHA256SUMS\n 31a1a4fc99c5b0a729d261f137cd7e1599bfa6f214f83fe4e596543d8ccf2dec    80699 main/installer-amd64/current/images/SHA256SUMS\nMD5Sum:\n abc 1 other\n";

    #[test]
    fn release_lookup_uses_only_sha256_section() {
        assert_eq!(
            release_sha256(RELEASE, "main/installer-amd64/current/images/SHA256SUMS").as_deref(),
            Some("31a1a4fc99c5b0a729d261f137cd7e1599bfa6f214f83fe4e596543d8ccf2dec")
        );
        assert!(release_sha256(RELEASE, "other").is_none(), "MD5 section must be ignored");
    }

    #[test]
    fn sums_lookup() {
        let s = "57303d157cffce3fa402301667fbc9b2280aa7372082b081e777375ae49da5f2  ./netboot/debian-installer/amd64/initrd.gz\n";
        assert_eq!(sums_sha256(s, "./netboot/debian-installer/amd64/initrd.gz").unwrap().len(), 64);
        assert!(sums_sha256(s, "./netboot/x").is_none());
    }

    #[test]
    fn netboot_paths_per_arch() {
        assert!(netboot_paths(Arch::Armhf).unwrap().0.ends_with("vmlinuz"));
        assert!(netboot_paths(Arch::Arm64).unwrap().1.contains("arm64"));
        assert!(netboot_paths(Arch::Unsupported).is_none());
    }

    #[test]
    fn preseed_runs_stage2_at_partman() {
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let p = render_preseed(&PreseedParams { cfg: &cfg, http_base: "http://10.0.0.2:8800/tok", debug_ssh_password: Some("Abc123xyz789") });
        assert!(p.contains("d-i partman/early_command string wget -q -O /tmp/synofurb-stage2.sh http://10.0.0.2:8800/tok/stage2.sh"));
        assert!(p.contains("netcfg/get_hostname string nas"));
        assert!(!p.contains("$6$"), "no password material in the preseed");
    }

    #[test]
    fn preseed_enables_the_installer_ssh_unless_switched_off() {
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let on = render_preseed(&PreseedParams { cfg: &cfg, http_base: "http://h/t", debug_ssh_password: Some("Abc123xyz789") });
        assert!(on.contains("d-i anna/choose_modules string network-console"));
        assert!(on.contains("d-i network-console/password password Abc123xyz789"));
        assert!(on.contains("d-i network-console/password-again password Abc123xyz789"));
        let off = render_preseed(&PreseedParams { cfg: &cfg, http_base: "http://h/t", debug_ssh_password: None });
        assert!(!off.contains("network-console"));
    }

    #[test]
    fn cmdline_has_logging_and_serial() {
        let c = installer_cmdline(&SerialConsole::default(), "10.0.0.2", 8802, true, None);
        assert!(c.contains("log_host=10.0.0.2") && c.contains("log_port=8802"));
        assert!(c.contains("console=tty0 console=ttyS0,115200n8"), "serial must come last so it is /dev/console");
        assert!(c.contains("DEBCONF_DEBUG=5"));
        assert!(!installer_cmdline(&SerialConsole::default(), "h", 1, false, None).contains("DEBCONF_DEBUG"));
    }

    #[test]
    fn cpio_is_readable_by_system_cpio() {
        let gz = cpio_gz(&[("preseed.cfg", b"d-i foo bar\n", 0o100600), ("a/b.txt", b"hello", 0o100644)]);
        let dir = std::env::temp_dir().join(format!("sf-cpio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.cpio.gz");
        std::fs::write(&f, &gz).unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && gzip -dc x.cpio.gz | cpio -it 2>&1", dir.display()))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.contains("not found") {
            eprintln!("cpio not installed; skipping format check");
            return;
        }
        assert!(out.status.success(), "cpio failed: {text}");
        assert!(text.contains("preseed.cfg") && text.contains("a/b.txt"), "{text}");
    }
}
