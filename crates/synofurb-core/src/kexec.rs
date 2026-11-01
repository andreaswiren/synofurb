//! Boot the Debian installer from a running DSM with `kexec`, and export an "installer kit" for
//! boxes where that is not possible (no kexec in the DSM kernel, U-Boot ARM boards, USB boot).

use crate::debian::sha256_hex;
use crate::logs::Logger;
use crate::model::{Arch, DeviceFacts, Firmware, SerialConsole};
use crate::remote::{Privileged, RemoteShell};
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct KexecJob<'a> {
    pub arch: Arch,
    pub kexec_bin: &'a Path,
    pub kernel: &'a Path,
    pub initrd: &'a Path,
    pub dtb: Option<&'a Path>,
    pub cmdline: &'a str,
}

/// Look for a statically linked `kexec` for `arch`: explicit path, `$SYNOFURB_KEXEC`, then `assets/kexec/<arch>/kexec`
/// next to the executable or in the working directory.
pub fn find_kexec_binary(arch: Arch, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Ok(p.to_path_buf());
        }
        bail!("kexec binary {} does not exist", p.display());
    }
    if let Ok(p) = std::env::var("SYNOFURB_KEXEC")
        && Path::new(&p).is_file()
    {
        return Ok(p.into());
    }
    let mut dirs = vec![PathBuf::from("assets/kexec")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(d) = exe.parent()
    {
        dirs.push(d.join("assets/kexec"));
    }
    for d in dirs {
        let p = d.join(arch.debian()).join("kexec");
        if p.is_file() {
            return Ok(p);
        }
    }
    bail!(
        "no static kexec binary for {} found. Build one with scripts/build-kexec.sh (see docs/kexec.md) \
         and pass it with --kexec, or use the installer kit instead.",
        arch.debian()
    )
}

/// Parse `df -P -k` output into (mountpoint, available KiB).
pub fn parse_df(out: &str) -> Vec<(String, u64)> {
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() >= 6 { Some((f[5..].join(" "), f[3].parse().ok()?)) } else { None }
        })
        .collect()
}

fn file_sha(path: &Path) -> Result<(String, u64)> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok((sha256_hex(&data), data.len() as u64))
}

fn upload_checked(
    sh: &mut dyn RemoteShell,
    local: &Path,
    remote: &str,
    mode: u32,
    log: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<()> {
    let (sha, size) = file_sha(local)?;
    let mut f = File::open(local)?;
    let name = local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    sh.upload(remote, &mut f, size, mode, &mut |sent| progress(&name, sent as f32 / size.max(1) as f32))?;
    let r = sh.exec(&format!("sha256sum '{remote}' 2>/dev/null || openssl dgst -sha256 '{remote}' 2>/dev/null"), None)?;
    let got = r.stdout.split_whitespace().find(|t| t.len() == 64).unwrap_or("").to_string();
    if got.is_empty() {
        let r = sh.exec(&format!("wc -c < '{remote}'"), None)?;
        if r.stdout.trim().parse::<u64>().ok() != Some(size) {
            bail!("size check failed for {remote} (no sha256 tool on the NAS)");
        }
        log.warn(format!("no sha256 tool on the NAS; verified {remote} by size only"));
    } else if got != sha {
        bail!("checksum mismatch after uploading {remote}: expected {sha}, NAS has {got}");
    } else {
        log.info(format!("{remote}: sha256 verified on the NAS"));
    }
    Ok(())
}

/// Upload everything and `kexec -l` it. Does NOT execute the kernel; see [`execute`].
pub fn stage_and_load(
    sh: &mut dyn RemoteShell,
    privileged_root: bool,
    sudo_pw: Option<String>,
    job: &KexecJob,
    log: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<String> {
    let need: u64 = [job.kexec_bin, job.kernel, job.initrd]
        .iter()
        .chain(job.dtb.iter())
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    let need_kib = need / 1024 * 12 / 10 + 1024;
    let df = sh.exec("df -P -k /tmp /var/tmp 2>/dev/null", None)?;
    let dir = parse_df(&df.stdout)
        .into_iter()
        .find(|(_, avail)| *avail >= need_kib)
        .map(|(m, _)| m)
        .with_context(|| format!("neither /tmp nor /var/tmp on the NAS has {} MiB free for the installer files", need_kib / 1024))?;
    let stage = format!("{}/.synofurb-{}", dir.trim_end_matches('/'), crate::config::random_token(4));
    log.info(format!("staging installer files in {stage} on the NAS"));
    sh.exec(&format!("mkdir -p '{stage}' && chmod 700 '{stage}'"), None)?;
    let (kx, kr, ir) = (format!("{stage}/kexec"), format!("{stage}/kernel"), format!("{stage}/initrd"));
    upload_checked(sh, job.kexec_bin, &kx, 0o700, log, progress)?;
    upload_checked(sh, job.kernel, &kr, 0o600, log, progress)?;
    upload_checked(sh, job.initrd, &ir, 0o600, log, progress)?;
    let mut dtb_arg = String::new();
    if let Some(d) = job.dtb {
        let dr = format!("{stage}/board.dtb");
        upload_checked(sh, d, &dr, 0o600, log, progress)?;
        dtb_arg = format!(" --dtb='{dr}'");
    }
    let v = sh.exec(&format!("'{kx}' --version"), None)?;
    if !v.ok() {
        bail!(
            "the uploaded kexec binary does not run on this NAS ({}); it must be statically linked for {}",
            v.stderr.trim(),
            job.arch.debian()
        );
    }
    log.info(format!("kexec on the NAS: {}", v.stdout.trim()));
    if job.cmdline.contains('\'') {
        bail!("unsafe kernel command line");
    }
    let mut p = Privileged { sh, is_root: privileged_root, sudo_password: sudo_pw };
    let r = p.run(&format!("'{kx}' -l '{kr}' --initrd='{ir}'{dtb_arg} --command-line='{}'", job.cmdline))?;
    if !r.ok() {
        bail!("kexec -l failed: {} {}", r.stdout.trim(), r.stderr.trim());
    }
    let loaded = p.sh.exec("cat /sys/kernel/kexec_loaded", None)?;
    if loaded.stdout.trim() != "1" {
        bail!("kexec did not report a loaded kernel (kexec_loaded = '{}')", loaded.stdout.trim());
    }
    log.info("installer kernel is loaded into memory; the files can be deleted");
    p.run(&format!("rm -f '{kr}' '{ir}'"))?;
    Ok(stage)
}

/// Sync, remount read-only and jump into the installer. The SSH connection drops; that is expected.
pub fn execute(sh: &mut dyn RemoteShell, privileged_root: bool, sudo_pw: Option<String>, stage: &str, log: &Logger) -> Result<()> {
    log.warn("executing kexec: DSM is stopping now and the connection will drop");
    let mut p = Privileged { sh, is_root: privileged_root, sudo_password: sudo_pw };
    let script = format!(
        "nohup /bin/sh -c 'sleep 2; sync; echo s > /proc/sysrq-trigger 2>/dev/null; sleep 1; echo u > /proc/sysrq-trigger 2>/dev/null; sleep 1; sync; \"{stage}/kexec\" -e' >/dev/null 2>&1 &"
    );
    p.run(&script)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Installer kit
// ---------------------------------------------------------------------------------------------

pub struct KitParams<'a> {
    pub facts: &'a DeviceFacts,
    pub cmdline: &'a str,
    pub serial: &'a SerialConsole,
    pub desktop_ip: &'a str,
    pub preseed_url: &'a str,
    pub dtb_name: Option<&'a str>,
}

pub fn render_kit_readme(p: &KitParams) -> String {
    let f = p.facts;
    let mut s = format!(
        "synofurb installer kit for {model} ({arch}, {fw})\n\
         ==============================================================\n\n\
         WARNING: installing wipes or repartitions the disks. Keep a backup of anything you care about.\n\n\
         Files\n  vmlinuz / linux      Debian installer kernel (checksum-verified by synofurb)\n  \
         initrd-synofurb.gz   Debian installer initrd with the preseed (and firmware, if selected) appended\n  \
         preseed.cfg          what was appended to the initrd\n\n\
         Kernel command line (serial console last so it becomes /dev/console):\n\n  {cmd}\n\n\
         synofurb must stay running on {ip} until the NAS reboots into Debian: the installer fetches its\n\
         script from it and streams its log back.\n\n",
        model = f.model,
        arch = f.arch.debian(),
        fw = f.firmware.label(),
        cmd = p.cmdline,
        ip = p.desktop_ip
    );
    match f.firmware {
        Firmware::Bios | Firmware::Uefi => {
            s.push_str(&format!(
                "x86 / UEFI boot options\n-----------------------\n\
                 A) GRUB (USB stick with GRUB, or any existing GRUB menu):\n\n\
                 \x20  menuentry 'Debian installer (synofurb)' {{\n\
                 \x20    linux  /synofurb/vmlinuz {cmd}\n\
                 \x20    initrd /synofurb/initrd-synofurb.gz\n\
                 \x20  }}\n\n\
                 B) Stock Debian netinst/netboot media: boot it, press 'e' (GRUB) or Tab (syslinux), append\n\
                 \x20  these parameters, and boot:\n\n\
                 \x20  auto=true priority=critical url={url} log_host={ip} {ser}\n\n\
                 Serial console: {tty} at {speed} baud, 8N1. Synology boards usually have no VGA output; use the\n\
                 serial header or a USB serial adapter, and set the BIOS to boot from USB/network.\n",
                cmd = p.cmdline,
                url = p.preseed_url,
                ip = p.desktop_ip,
                ser = p.serial.kernel_arg(),
                tty = p.serial.tty,
                speed = p.serial.speed
            ));
        }
        Firmware::UBoot => {
            let dtb = p.dtb_name.unwrap_or("<your-board>.dtb");
            s.push_str(&format!(
                "ARM / U-Boot boot (needs a TTL serial adapter: {tty}, {speed} baud, 8N1)\n\
                 -----------------------------------------------------------------------\n\
                 Reference: https://wiki.debian.org/InstallingDebianOn/Synology\n\
                 Serve this directory by TFTP from {ip} (for example: dnsmasq --enable-tftp --tftp-root=<dir> or tftpd-hpa),\n\
                 interrupt U-Boot with Ctrl+C and try (addresses vary by board; this is a generic recipe, NOT verified\n\
                 on every Synology model):\n\n\
                 \x20  setenv serverip {ip}\n\
                 \x20  dhcp                       (or: setenv ipaddr <free-ip>)\n\
                 \x20  tftpboot ${{fdt_addr_r}} {dtb}\n\
                 \x20  tftpboot ${{kernel_addr_r}} vmlinuz\n\
                 \x20  tftpboot ${{ramdisk_addr_r}} initrd-synofurb.gz\n\
                 \x20  setenv bootargs '{cmd}'\n\
                 \x20  bootz ${{kernel_addr_r}} ${{ramdisk_addr_r}}:${{filesize}} ${{fdt_addr_r}}\n\n\
                 If the variables above are not defined on your U-Boot, take the load addresses from `printenv`.\n\
                 Mainline Debian ships DTBs only for DS213j (armada-370-synology-ds213j.dtb), DS116\n\
                 (armada-385-synology-ds116.dtb) and DS414 (armada-xp-synology-ds414.dtb); other models need a DTB\n\
                 you supply. After the install finishes, /boot/uboot-env.txt on the NAS contains the exact U-Boot\n\
                 commands that boot Debian from the disk.\n",
                tty = p.serial.tty,
                speed = p.serial.speed,
                ip = p.desktop_ip,
                dtb = dtb,
                cmd = p.cmdline
            ));
        }
    }
    s
}

pub fn export_kit(
    dir: &Path,
    files: &crate::debian::InstallerFiles,
    initrd_with_preseed: &Path,
    preseed: &str,
    p: &KitParams,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::copy(&files.kernel, dir.join("vmlinuz"))?;
    std::fs::copy(initrd_with_preseed, dir.join("initrd-synofurb.gz"))?;
    std::fs::write(dir.join("preseed.cfg"), preseed)?;
    std::fs::write(dir.join("kernel-cmdline.txt"), format!("{}\n", p.cmdline))?;
    if let Some(d) = &files.dtb {
        std::fs::copy(d, dir.join(d.file_name().unwrap()))?;
    }
    std::fs::write(dir.join("README.txt"), render_kit_readme(p))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{fixtures, parse_inspect};
    use crate::remote::mock::MockShell;

    #[test]
    fn df_parsing() {
        let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\ntmpfs 100 1 51200 1% /tmp\n/dev/md0 8000 1 4096 50% /\n";
        assert_eq!(parse_df(out), vec![("/tmp".into(), 51200), ("/".into(), 4096)]);
    }

    fn files(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        std::fs::create_dir_all(dir).unwrap();
        let (a, b, c) = (dir.join("kexec"), dir.join("kernel"), dir.join("initrd"));
        std::fs::write(&a, b"KEXEC").unwrap();
        std::fs::write(&b, b"KERNEL").unwrap();
        std::fs::write(&c, b"INITRD").unwrap();
        (a, b, c)
    }

    #[test]
    fn stage_and_load_sequence_and_checks() {
        let dir = std::env::temp_dir().join(format!("sf-kx-{}", std::process::id()));
        let (kx, kr, ir) = files(&dir);
        let mut m = MockShell::default();
        m.push_ok("Filesystem 1024-blocks Used Available Capacity Mounted on\ntmpfs 900000 0 800000 0% /tmp\n"); // df
        m.push_ok(""); // mkdir
        for data in [&b"KEXEC"[..], b"KERNEL", b"INITRD"] {
            m.push_ok(&format!("{}  /x\n", sha256_hex(data))); // sha256sum after each upload
        }
        m.push_ok("kexec-tools 2.0.31\n"); // --version
        m.push_ok(""); // kexec -l
        m.push_ok("1\n"); // kexec_loaded
        let log = Logger::memory(None);
        let job = KexecJob {
            arch: Arch::X86_64,
            kexec_bin: &kx,
            kernel: &kr,
            initrd: &ir,
            dtb: None,
            cmdline: "auto=true console=ttyS0,115200n8",
        };
        let stage = stage_and_load(&mut m, true, None, &job, &log, &mut |_, _| {}).unwrap();
        assert!(stage.starts_with("/tmp/.synofurb-"));
        assert_eq!(m.uploads.len(), 3);
        assert!(m.commands.iter().any(|c| c.contains("kexec' -l") && c.contains("--initrd=") && c.contains("--command-line='auto=true")));
        assert!(m.commands.last().unwrap().starts_with("rm -f"), "kernel/initrd deleted once loaded");
    }

    #[test]
    fn checksum_mismatch_aborts() {
        let dir = std::env::temp_dir().join(format!("sf-kx2-{}", std::process::id()));
        let (kx, kr, ir) = files(&dir);
        let mut m = MockShell::default();
        m.push_ok("h\ntmpfs 1 0 800000 0% /tmp\n");
        m.push_ok("");
        m.push_ok(&format!("{}  /x\n", "0".repeat(64)));
        let job = KexecJob { arch: Arch::X86_64, kexec_bin: &kx, kernel: &kr, initrd: &ir, dtb: None, cmdline: "x" };
        let err = stage_and_load(&mut m, true, None, &job, &Logger::memory(None), &mut |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
    }

    #[test]
    fn not_enough_space_is_reported() {
        let dir = std::env::temp_dir().join(format!("sf-kx3-{}", std::process::id()));
        let (kx, kr, ir) = files(&dir);
        let mut m = MockShell::default();
        m.push_ok("h\ntmpfs 1 0 10 0% /tmp\n");
        let job = KexecJob { arch: Arch::X86_64, kexec_bin: &kx, kernel: &kr, initrd: &ir, dtb: None, cmdline: "x" };
        assert!(stage_and_load(&mut m, true, None, &job, &Logger::memory(None), &mut |_, _| {}).is_err());
    }

    #[test]
    fn execute_syncs_before_kexec() {
        let mut m = MockShell::default();
        execute(&mut m, true, None, "/tmp/.synofurb-1", &Logger::memory(None)).unwrap();
        let c = &m.commands[0];
        assert!(c.contains("sysrq-trigger") && c.contains("kexec\" -e") && c.starts_with("nohup"));
        assert!(c.find("echo u").unwrap() < c.find("-e").unwrap());
    }

    #[test]
    fn kit_readme_variants() {
        let facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let serial = SerialConsole::default();
        let p = KitParams {
            facts: &facts,
            cmdline: "auto=true x",
            serial: &serial,
            desktop_ip: "10.0.0.2",
            preseed_url: "http://10.0.0.2:1/t/preseed.cfg",
            dtb_name: None,
        };
        let r = render_kit_readme(&p);
        assert!(r.contains("menuentry") && r.contains("url=http://10.0.0.2:1/t/preseed.cfg") && r.contains("backup"));
        let mut arm = facts.clone();
        arm.firmware = Firmware::UBoot;
        arm.arch = Arch::Armhf;
        let p = KitParams { facts: &arm, dtb_name: Some("armada-385-synology-ds116.dtb"), ..p };
        let r = render_kit_readme(&p);
        assert!(
            r.contains("bootz") && r.contains("armada-385-synology-ds116.dtb") && r.contains("wiki.debian.org/InstallingDebianOn/Synology")
        );
    }

    #[test]
    fn missing_kexec_binary_explains_how_to_get_one() {
        let e = find_kexec_binary(Arch::Armhf, None).unwrap_err().to_string();
        assert!(e.contains("build-kexec.sh"));
    }
}
