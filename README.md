# synofurb

A desktop tool (Rust, Windows / macOS / Linux) that finds Synology NAS units on your network and reinstalls
them with **plain Debian 13 "trixie"** on **LVM + xfs**, with software RAID (mdadm), SSH, a serial console and a
small web management plane.

> **Read this first.** Reinstalling a NAS repartitions its disks. **There is no guarantee that any data on the NAS
> survives**, including volumes the tool marks as "preserved". Make a complete, verified backup somewhere else
> before you start. The tool makes you confirm this (two checkboxes and a typed phrase) before it changes anything.
>
> **Hardware status.** Everything here was developed and tested without a Synology unit. What *was* verified for
> real is listed under [What is verified](#what-is-verified). The first install on your hardware is the real test:
> have a serial console ready and keep your backup.

## What it does

1. **Scans** the network the way Synology's own tools do (see [Discovery](#discovery)) and keeps an inventory.
2. **Inspects** the chosen NAS over SSH, read-only (disks, partitions, md arrays, LVM, mounts, RAM, firmware, CPU).
3. **Plans** the new storage layout and shows exactly what will be destroyed and what will be kept.
4. **Boots the Debian installer** on the NAS (automatically with `kexec`, or from an installer kit you boot yourself).
5. **Installs** Debian with a generated, reviewable shell script, streaming a detailed debug log back to you.
6. **Reports** the result: addresses, logins, SSH host key fingerprint, management URL.

```
synofurb gui                 # wizard (default); `synofurb gui --demo` shows it with a built-in sample NAS
synofurb scan                # discovery only
synofurb plan    --host 192.168.1.20 --user admin      # read-only: show the plan
synofurb install --host ... --mode keep ...            # headless install; needs --have-backup --accept-data-loss --confirm
synofurb fetch-installer amd64|arm64|armhf             # download + verify the Debian installer files
```

## The resulting system

| | |
|---|---|
| OS | Debian 13 trixie, `amd64`, `arm64` or `armhf` (ARMv7) |
| System | **LVM** `vg_sys` on md RAID1 (swap + **xfs** root). Size selectable; recommended 32 GiB (16 / 8 GiB on small disks) |
| Redundancy | With **more than two disks** the system is *always* mirrored (default 3 copies). With two disks it is mirrored by default; one disk = no mirror |
| Boot | BIOS: GRUB on every system disk (`bios_grub`); UEFI: mirrored ESP (md metadata 1.0) with `--removable` GRUB; ARM/U-Boot: raw sectors, see [docs/ARM.md](docs/ARM.md) |
| Data | md RAID (auto: none / 1 / 5 / 6, or choose 0/1/5/6/10) -> LVM `vg_data` -> **xfs** `/srv/data`; 20 % of the VG is left free by default for new volumes and snapshots |
| Filesystems | xfs, btrfs, ext4, exFAT, FAT32, NTFS (ntfs-3g, full read/write + `mkntfs`/`ntfsfix`) and mdadm tools |
| Default user | your choice of name, generated strong password; in `sudo`. Root login is locked, and SSH root login is off |
| SSH | enabled. Key login preferred (a key pair is generated for you, or paste yours); password login stays on |
| Network | systemd-networkd: **DHCPv4 + DHCPv6 + SLAAC** on all Ethernet ports, MAC-based DHCP client id, mDNS (`<host>.local`) |
| Serial console | kernel + GRUB + login on the port DSM used (from its kernel command line, default `ttyS0` 115200 8N1) **and** on any USB serial adapter the moment it is plugged in (udev -> `serial-getty`) |
| Local console | VGA/HDMI text console, USB keyboard/mouse (USB HID + xHCI in the initramfs, `gpm` for the mouse) |
| Management plane | `https://<ip>:8443`, see [apps/mgmt-plane/README.md](apps/mgmt-plane/README.md) |
| Updates | `unattended-upgrades` for Debian security fixes (never reboots on its own) |

### Management plane

A single-file Python 3 service (standard library only, so identical on x86 and ARM) that runs as a systemd unit:
system status (CPU/RAM/temperature/disks/SMART/RAID/LVM/mounts), **users** (create/delete/password/lock/SSH keys),
**network** (DHCP or static v4/v6, DNS; applied with an automatic rollback timer so a wrong address can't lock you
out), **volumes** (create md RAID + LVM + xfs/ext4/btrfs, extend, delete), **mounts** (xfs/ext4/btrfs/exFAT/FAT32/NTFS,
fstab entries by UUID), and **import** of volumes preserved from the old DSM (assembled and mounted read-only by
default). HTTPS with a self-signed certificate (fingerprint printed at first start and shown in the final summary),
session cookies + CSRF tokens, login throttling and an audit log. 83 unit/HTTP tests.

## Backups, data loss and "keep my volumes"

* **Wipe everything**: every disk gets a new GPT. Everything on it is gone.
* **Keep existing volumes**: only DSM's *system* (partition 1) and *swap* (partition 2) areas, which DSM mirrors across
  all disks, are reclaimed (plus any array you explicitly release). Debian goes there. Data partitions (DSM's `md2`,
  `md3`, ... usually an LVM volume group with btrfs or ext4) are never part of any command: the planner refuses to
  generate one, and after the install the script re-reads the RAID superblocks of every preserved partition and
  **halts** if one changed. Because DSM's system area is only ~10 GiB (DSM 6: ~4.4 GiB), the system LV is limited to
  what fits (minimum 8 GiB); the planner tells you when it cannot fit.
* Preserved volumes are **not** mounted automatically. DSM-specific features (shared folders, users, packages,
  Hyper Backup tasks) do not carry over; only the files do. Btrfs/ext4 volumes from DSM mount read-only by default.
* Even in "keep" mode there is no guarantee. A failing disk, a power cut during repartitioning, or a bug can destroy data.

## Discovery

Same mechanisms Synology uses, merged into one inventory (`discovery/`):

* **Synology Assistant broadcast**: UDP 9999 query with TLV replies (hostname, MAC, DSM build, model, serial, ports).
  The packet format is **not published by Synology**; it comes from third-party reimplementations and is
  *unverified* here. It is best effort.
* **Bonjour/mDNS** (`_diskstation._tcp`; and `_synofurb._tcp` for NAS units this tool installed),
* **UPnP/SSDP** (`M-SEARCH`, device description with manufacturer *Synology*),
* **DSM Web API fingerprint** (unauthenticated `SYNO.API.Info` on port 5000) plus a `/24` port sweep for networks that
  block multicast,
* **MAC OUI** (`00:11:32`, `90:09:d0`) from the ARP cache.

## Supported hardware

| Platform | Status | Notes |
|---|---|---|
| x86-64 (Apollo Lake, Gemini Lake, Denverton, Ryzen V1000/R1000, Broadwell-DE, Atom, ...) | supported | boots standard Debian; BIOS or UEFI is detected. The internal Synology flash module may take boot precedence: check the BIOS boot order |
| ARMv7 / `armhf` (Marvell Armada 370, 38x, XP) | supported through the documented U-Boot raw-sector method | needs a serial adapter. See [docs/ARM.md](docs/ARM.md) |
| ARM64 (Realtek RTD1296/1619B, Annapurna Alpine v2, Armada 37xx) | experimental | installer + system install work generically; booting from disk through U-Boot is not automated |
| PowerPC, 32-bit x86 (Evansport), ARMv5 (Kirkwood), Comcerto, Hi3535 | **refused** | Debian 13 cannot run on them |

## How the install works

```
desktop                                       NAS (DSM)                       NAS (Debian installer, in RAM)
-------                                       ---------                       ------------------------------
inspect  --- ssh (read-only script) --------> reads sysfs/mdstat/lvm
plan, you confirm
download netboot kernel+initrd, verified:
  InRelease -> SHA256SUMS -> files
initrd += preseed (+ non-free firmware)
start local server (token-protected HTTP,
  TCP log sink, UDP syslog sink)
upload static kexec + kernel + initrd -----> kexec -l ; sync ; remount ro ; kexec -e
                                                                              boots, DHCP, fetches stage2.sh
                                              <------ HTTP (token) ---------- runs it:
log stream (live, saved to file)  <----------- TCP + syslog ------------------  preflight (disks match? payload hash? keyring?)
                                                                                 ---- point of no return ----
                                                                                 mdadm stop, wipe reclaimed areas
                                                                                 parted, mdadm, LVM, mkfs
                                                                                 debootstrap trixie, chroot-setup.sh
                                                                                 verify preserved arrays, reboot
wait for the new system (mDNS / SSH / :8443), print logins
```

`stage2.sh` is **generated, straight-line shell** (every command visible, no JSON parsing in the installer); a copy is
saved next to the debug log before anything runs, and `SYNOFURB_DRY=1` turns every mutating command into a log line.
Before the point of no return it checks that each disk's serial/model/size and its complete partition table still match
what you confirmed.

### Boot methods

* **kexec over SSH** (automatic): needs a static `kexec` binary for the NAS CPU (`scripts/build-kexec.sh`, see
  [docs/kexec.md](docs/kexec.md)) and `CONFIG_KEXEC` in the DSM kernel (checked at inspection time).
* **Installer kit** (manual): writes `vmlinuz`, `initrd-synofurb.gz`, `preseed.cfg`, the exact kernel command line and a
  README for USB/GRUB, PXE or the U-Boot serial console. Everything else (stage 2, log streaming) is identical.

## Debug logging

* `~/.local/share/synofurb/logs/synofurb-<time>.log` (Linux; the platform data dir elsewhere), mode 0600, **secrets
  redacted**. Contains every SSH command and its output, every installer line, the generated `stage2.sh` and
  `plan-<id>.json` are saved beside it.
* The installer runs with `DEBCONF_DEBUG=5 BOOT_DEBUG=2` (switchable), `log_host=` syslog forwarding, and a TCP stream of
  stage 2's own log (`@@STEP n/12`, `@@FAIL`, ... markers drive the progress bar). On failure the installer **halts**
  after dumping `/proc/mdstat`, partitions, mounts, LVM state and `dmesg`; it never guesses.
* The NAS keeps `/var/log/synofurb/install.log`, `stage2.sh`, and `/var/lib/synofurb/{plan.json,dsm-inspect.txt}`.

## What is verified

Done for real in the development environment (no Synology hardware available):

* 82 Rust unit tests (planner, parsers, script rendering, payload, preseed, logging, discovery packets, kexec flow with a mock)
  and 83 Python tests for the management plane; `clippy -D warnings`, `rustfmt`, `shellcheck` clean.
* Generated partition commands executed with real `parted` on a loop device laid out like DSM: preserved partition 5
  stayed byte-identical (same start/end/type) while the reclaimed area was rebuilt; wipe mode gave the planned table.
* The SSH layer against a real `sshd` (exit codes, stderr, stdin, 40 MiB streamed upload with matching SHA-256, mode bits,
  host fingerprint, wrong password).
* The inspection script on a real Linux host and through the real parser.
* **`debootstrap` of Debian trixie and the real `chroot-setup.sh`**: all packages resolve on amd64/arm64/armhf; user,
  `sudo`, SSH config (`sshd -t`), networkd DHCP file, serial getty, USB-serial udev rule, GRUB serial config, USB HID/xHCI/RAID/
  device-mapper in the initramfs, management plane install. The new password hash was checked against glibc `crypt()`.
* Live download + hash-chain verification of the amd64 and armhf installers from the Debian mirrors; `cpio` reads the
  initrd addition.
* The GUI starts and renders (software OpenGL under Xvfb).

**Not verified** (be honest with yourself about these):

* Any run on a real Synology; `kexec` on a DSM kernel; Debian installer behaviour with the generated preseed
  (`partman/early_command` hook, availability of busybox tools such as `nc`/`sha256sum` in the installer).
* md RAID, LVM and `mkfs` steps (the sandbox kernel has no md/dm) and `grub-install` to real disks.
* Synology UDP 9999 discovery against real DSM; mDNS TXT field names.
* U-Boot boot recipes on real boards; `scripts/build-kexec.sh` (never run).
* PGP verification of the Debian release file happens only when `gpgv` and a recent `debian-archive-keyring` are installed
  on the desktop; otherwise the chain is HTTPS + SHA-256 only.

## Build

```
cargo build --release          # Rust 1.85+; Linux needs the usual X11/Wayland + OpenGL runtime libs
cargo test --workspace
python3 -m unittest discover -s apps/mgmt-plane/tests
```

Layout: `crates/synofurb-core` (engine, no GUI), `crates/synofurb` (GUI + CLI), `apps/mgmt-plane` (the NAS web UI, its own project), `payload/` (files copied to the NAS),
`scripts/`, `docs/`.

## Roadmap / known gaps

* Built-in TFTP/DHCP-proxy (PXE) so the installer kit needs no extra server.
* Pre-built static `kexec` binaries published from CI (workflow exists: `kexec-static`, run manually).
* Verify and document per-model DTBs and U-Boot environments for common ARM boards; ARM64 raw-boot.
* LUKS option; email/webhook alerts from the management plane.
* Relation to ForgeVault: the installed system is plain Debian 13, the OS ForgeVault targets, so ForgeVault can be installed on
  top of it. That integration is not part of this repository.

License: MIT.
