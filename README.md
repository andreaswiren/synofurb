# synofurb

Reinstall Synology NAS units with **plain Debian 13 "trixie"** (Linux 7.x available), on **LVM + xfs** with mdadm RAID,
SSH, a serial console and a small web management plane. Intel, **AMD**, arm64 and ARMv7 units. A native desktop app for
**Windows, macOS and Linux** (Rust, GPUI Kit) finds the unit, shows what will happen, and does it. For x86 units you
need **SSH access to the NAS and nothing else**.

> **Read this first.** Reinstalling a NAS repartitions its disks. **There is no guarantee that any data on the NAS
> survives**, including volumes the tool marks as "preserved". Make a complete, verified backup somewhere else before you
> start. The app makes you confirm this (two checkboxes and a typed phrase) before it changes anything.
>
> **Hardware status.** Nothing has been run on a real Synology yet. The compatibility list marks **every model
> "Untested"** except the DS923+, which another project (synodeb) reports working with the same boot technique. See
> [What is verified](#what-is-verified) for what *was* tested, and [docs/RECOVERY.md](docs/RECOVERY.md) for what happens when something fails.

## Install the app

```sh
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh
```
```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1 | iex
```
Both verify the SHA-256 and the minisign signature of the release. Windows also gets an MSI and an installer `.exe`, macOS a `.dmg`,
Linux `.deb`, `.rpm`, AppImage and a portable archive, all on the [Releases](https://github.com/andreaswiren/synofurb/releases) page
(no release exists until the owner completes the checklist at the end). More one-liners (headless conversion, the management plane on
an existing Debian box): [docs/INSTALL.md](docs/INSTALL.md). The app updates itself from the releases (`synofurb self-update`, or the prompt in the app).

## What it does

1. **Finds** Synology units the way Synology's tools do (Assistant broadcast, mDNS, UPnP, the DSM web API, MAC prefixes, subnet sweep).
2. **Looks the model up** in the repository's [compatibility list](compat/models.json) (272 models) and shows its status.
3. **Inspects** the unit over SSH, read-only: disks, RAID, LVM, volumes, RAM, CPU vendor, firmware, boot flash, serial console.
4. **Plans** the layout and lists exactly what will be destroyed and what is kept.
5. **Backs up** what can be backed up first: the unit's boot flash and the partition tables of every disk it will touch.
6. **Boots the installer**, installs, streams a detailed debug log, then shows the logins.

You can also stop after the boot-flash step in **boot manager mode**: an SSH-controlled boot environment on the unit lets you netboot and try
other Linux systems by hand, while DSM stays one reboot away ([docs/BOOT.md](docs/BOOT.md)).

```
synofurb                      # the desktop app (add `gui --demo` to try it with a built-in sample NAS)
synofurb-cli ...              # the same commands with NO graphical libraries: servers, SSH sessions, CI (docs/CLI.md)

synofurb-cli wizard           # the app's nine steps as plain terminal prompts (also: synofurb wizard)
synofurb scan [--json]        # discovery only (results stream in; saved to the inventory)
synofurb inventory list|add --ip IP|forget PATTERN
synofurb inspect --host IP [--json] [--save facts.json]       # read-only: disks, arrays, firmware, boot flash, serial, compatibility
synofurb compat DS923+ [--json] [--refresh]                   # look a model up in the compatibility list
synofurb plan --host IP --mode keep|wipe [--keep md2] [--system-gib N] [--copies N] [--raid auto|none|0|1|5|6|10|shr1|shr2] [--free-percent P] [--json]   # read-only
synofurb install --host IP ... --dry-run [--out DIR]          # render the stage-2 script, payload, plan and preseed; change nothing
synofurb install --host IP ... --have-backup --accept-data-loss --confirm "ERASE <MODEL>"   # DESTROYS DATA; all three are always required
synofurb boot-manager install|enter|status --host IP          # remote-controllable boot environment
synofurb recover --host IP                                    # make the unit boot DSM again
synofurb backup-dom|disk-backup --host IP                     # boot flash image / partition-table backup, nothing else
synofurb logs list|show [N]|path   synofurb config show|paths   synofurb fetch-installer ARCH
synofurb bundle create --arch amd64 --out DIR                 # offline bundle: amd64 ~440 MiB (+ ~470 MiB installer firmware shared by all architectures)
synofurb bundle verify|info DIR   synofurb install --host IP --source offline --bundle DIR ...   # no Internet needed on either side (docs/OFFLINE.md)
synofurb self-update [--check]   synofurb completions bash|zsh|fish|powershell   synofurb version [--json]
```
Machine-friendly: `--log-format json` (NDJSON events), `--json` documents, stable exit codes (0 ok, 2 usage, 3 refused by the safety gate, 4 plan errors,
5 unsupported hardware, 6/7 install failed after/before the point of no return), no colour with `NO_COLOR`. Passwords never go on the command line.

## The resulting system

| | |
|---|---|
| OS | Debian 13 trixie for `amd64`, `arm64`, `armhf`. Kernel: **Newest** (Linux 7.x from `trixie-backports`, default) or **Stable** (6.12 LTS) |
| System | LVM `vg_sys` on md RAID1 (swap + **xfs** root). Selectable size, recommended 32 GiB. **With more than two disks the system is always mirrored** (default 3 copies) |
| Boot | x86 with a boot flash: entries on the unit's internal flash ([docs/BOOT.md](docs/BOOT.md)). Other x86: GRUB (BIOS/UEFI, mirrored ESP). ARM: U-Boot raw sectors ([docs/ARM.md](docs/ARM.md)) |
| Data | md RAID (auto none/1/5/6, or choose 0/1/5/6/10) -> LVM `vg_data` -> **xfs** `/srv/data`; 20 % of the group left free by default. **Hybrid RAID (`--raid shr1` / `shr2`)**: Synology-Hybrid-RAID-like slices of mdadm arrays joined in `vg_data`, so disks of different sizes are used fully (SHR-1 survives 1 failed disk, SHR-2 two). Needs wiped disks; details in [docs/RAID.md](docs/RAID.md) |
| Filesystems | xfs, btrfs, ext4, exFAT, FAT32, NTFS (ntfs-3g), mdadm, LVM |
| CPU support | microcode for Intel **and AMD** (`intel-microcode` / `amd64-microcode`), AMD memory-mapped serial consoles |
| User | your name, generated strong password, `sudo`; root locked, SSH root login off |
| SSH | enabled; key login preferred (generated or yours), password login allowed |
| Network | systemd-networkd: DHCPv4 + DHCPv6 + **SLAAC**, mDNS `<host>.local` |
| Consoles | serial on the port DSM used (incl. AMD `uart8250,mmio32,...`) **and** on any USB serial adapter when plugged in; VGA/HDMI, USB keyboard and mouse |
| Management plane | `https://<ip>:8443`: status, users, network (safe apply with auto-rollback), volumes, mounts, imported DSM volumes, boot manager. Its own project: [apps/mgmt-plane](apps/mgmt-plane/README.md) |

## If it fails

DSM stays the default boot entry and every failure path points back to it: a reboot into DSM after 60 s if no disk was changed, and into the DSM
loader after 15 minutes if disks were. Before the first write the boot flash is imaged to your computer and the partition tables of every touched
disk are saved. What this can and cannot give back is written down in [docs/RECOVERY.md](docs/RECOVERY.md), including what is **not** recoverable.

## Supported hardware

[compat/models.json](compat/models.json) lists 272 models (Synology's CPU table plus older ones) with architecture, CPU vendor, boot type and status:
**Verified** (none yet), **Community-reported** (DS923+ only), **Untested**, **Known issues**, **Unsupported**. The app shows the status; `synofurb compat`
and a refresh from this repository keep it current ([docs/COMPAT.md](docs/COMPAT.md)). Please report your result with the issue template: that is how a model becomes Verified.

| Platform | Notes |
|---|---|
| x86-64 Intel and **AMD** (Ryzen V1000/R1000/R1600, EPYC, Atom, Celeron, Xeon-D, ...) | SSH only |
| ARMv7 / `armhf` (Armada 370/38x/XP) | Debian wiki method; needs a serial adapter ([docs/ARM.md](docs/ARM.md)) |
| arm64 (RTD1296/1619B, Armada 37xx) | experimental: installs, U-Boot boot not automated |
| PowerPC, 32-bit x86, ARMv5/armel, Comcerto, Hi3535 | refused: Debian 13 cannot run on them |

## Repository layout (monorepo)

| Path | What |
|---|---|
| `crates/synofurb-core` | engine: discovery, inspection, planner, installer script generator, boot-flash/agent orchestration, updater, compatibility lookup |
| `crates/synofurb` | the desktop app (GPUI Kit UI in `src/ui`) and the CLI |
| `apps/mgmt-plane` | the NAS web UI (Python 3, standard library only), versioned separately |
| `boot/dom` | `synofurb-dom`: safe boot-menu manager for the internal boot flash (POSIX sh, with tests) |
| `boot/agent` | the boot agent builder: kernel + tiny initramfs with SSH and kexec |
| `payload/` | files copied to the NAS during install |
| `compat/` | the model compatibility list, schema and sources |
| `design/` | mockups, critic reviews, screenshots, approvals ([docs/DESIGN-GATE.md](docs/DESIGN-GATE.md)) |
| `scripts/`, `packaging/`, `.github/` | release engineering |

## Releases need design approval

Two independent critics, a UX critic and a UI critic (`.claude/agents/`), must approve the design before any release build runs; approval is
bound to a hash of the design files ([docs/DESIGN-GATE.md](docs/DESIGN-GATE.md), [docs/RELEASING.md](docs/RELEASING.md)). `python3 scripts/release-gate.py`
reports the current state (today: not approved, on purpose).

## What is verified

Done for real in the development sandbox (no Synology available):

* Rust unit tests for the planner, parsers, script generator, payload, preseed, logging, discovery packets, the boot-flash flow with a mock shell, the
  compatibility database and the updater; Python tests for the management plane (116), the install scripts and the release gate; `clippy -D warnings`,
  `rustfmt`, `shellcheck`, `actionlint` clean.
* The generated partition commands run with real `parted` on a loop device laid out like DSM (a preserved partition stayed identical).
* The SSH layer against a real `sshd`; the inspection script on a real Linux host.
* A real `debootstrap` of trixie and the real `chroot-setup.sh` (packages on amd64/arm64/armhf, users, SSH config, network, serial, initramfs content, password hash against glibc `crypt()`).
* Live download of the Debian installer with the Release -> SHA256SUMS -> file hash chain.
* `synofurb-dom` and the boot agent against **synthetic** boot-flash images and (agent) QEMU: see [boot/dom](boot/dom/README.md) and [boot/agent](boot/agent/README.md).
* The installers against a local fake release server with real minisign signatures; PowerShell 7 on Linux.
* The GPUI Kit app builds and renders on Linux (software Vulkan); screenshots in `design/screenshots/`.

**Not verified:**

* Any run on a real Synology; `kexec` on a DSM kernel; the Debian installer's behaviour with the generated preseed on real hardware.
* md RAID, LVM and `mkfs` steps and `grub-install` to real disks (the sandbox kernel has no md/dm). This includes the SHR-like data layout: its slice algorithm, partitioning (real `parted` on loop devices) and generated commands are tested, the `mdadm`/`pvcreate`/`vgcreate` calls are not ([docs/RAID.md](docs/RAID.md#implementation-and-verification)).
* Windows and macOS builds, MSI/NSIS/DMG packaging, signing and notarisation, the release workflow on GitHub, the updater against a real release.
* Screen-reader behaviour of the native app; real users (the test plan is in `design/mockups/ux/RECOMMENDATION.md`).
* Synology's UDP 9999 discovery format (from third-party sources) and mDNS field names.
* U-Boot recipes on real boards; `scripts/build-kexec.sh` (never run; the sandbox blocks compiling downloaded sources).

## Before the first release (owner's checklist)

1. Approve the designs (or ask for changes); then run the two critics and commit their verdicts ([docs/DESIGN-GATE.md](docs/DESIGN-GATE.md)).
2. `minisign -G`, paste the public key where [docs/RELEASING.md](docs/RELEASING.md) lists it, add the GitHub secrets, run the workflow once with `dry_run`.
3. Decide the synodeb licence question ([docs/BOOT.md](docs/BOOT.md#licence-note-on-synodeb)).

License: MIT.
