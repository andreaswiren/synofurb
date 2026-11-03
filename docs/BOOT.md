# How units boot (and what needs only SSH)

`synofurb` needs **SSH access to the NAS and nothing else**: no opened case, no USB stick, no extra tools on the NAS.
That holds for x86 units. ARM boards are the exception (see the table).

| Unit | How Debian is started | Needs |
|---|---|---|
| **x86-64 Synology with an internal boot flash ("DOM")**: Intel *and* AMD (Ryzen V1000/R1000/R1600, EPYC, Atom, Xeon-D, Celeron, ...) | An extra entry in the boot flash's menu loads Debian's kernel + initrd; everything else lives on the disks. A tiny SSH-controlled **boot agent** (also stored on the flash) downloads and `kexec`s the full Debian installer. | **SSH only** |
| x86-64 without an editable boot flash | `kexec` from the running DSM (needs a static `kexec` binary and `CONFIG_KEXEC` in DSM's kernel), or the installer kit | SSH only if kexec works |
| ARMv7 (`armhf`) and arm64 boards with U-Boot | Kernel + initrd written to raw disk sectors, U-Boot environment changed once | a TTL **serial adapter** (see [ARM.md](ARM.md)) |

## x86: the internal boot flash

Synology x86 units always boot a small USB flash module on the mainboard (128 MB; see [boot/dom](../boot/dom/README.md) for how it is attached). It holds Synology's GRUB-style
menu (`EFI/boot/SynoBootLoader.conf` on a FAT partition) and the DSM kernel and initrd (`zImage`, `rd.gz` on an ext2 partition).
Because the firmware never looks at the disks first, Debian on the disks alone would not boot; Debian has to be reachable
from this menu. This is the technique the [synodeb](https://github.com/rylos/synodeb) project documents for the DS923+
(AMD Ryzen R1600), and it is why `synofurb` adds menu entries instead of installing GRUB to the disks on these units.

### Where does the operating system live? (DSM and Debian are built the same way)

The boot flash holds **only the boot loader, the kernel and the initial ramdisk**. It is not where the OS lives.

| | DSM (Synology) | Debian as installed by synofurb |
|---|---|---|
| Boot flash (about 128 MB) | loader menu, `zImage` (kernel), `rd.gz` (initial ramdisk) | the same menu plus our entries; Debian's kernel and a small initramfs |
| System (root filesystem, packages, settings) | on the **disks**: a RAID1 mirrored across all drives (`md0`, partition 1 of each disk) | on the **disks**: an LVM volume group `vg_sys` on a RAID1 across 2 to 3 disks (xfs root + swap) |
| Your data | data volumes on `md2` and up (SHR or RAID, LVM, btrfs or ext4) | `vg_data` on a RAID of your choice, xfs |

So in both cases the flash module only gets the system started: the initial ramdisk assembles the RAID and LVM and hands over to the root
filesystem on the disks. That is also why the flash is small and rarely written, and why the installed system's initramfs is kept small
(explicit module list, `xz`). Updating Debian's kernel writes about 25 to 30 MB to the flash, not the whole OS.

What `synofurb` writes there, and how safely ([boot/dom](../boot/dom/README.md)):

* **Before the first write** the whole flash is streamed to your computer and hashed, and the hash is compared with one computed on
  the NAS. The backup file name is shown; keep it off the NAS.
* DSM does not let the live partitions be mounted, so the tool reads each partition twice, edits a **copy**, checks it with `fsck`,
  re-reads the device to confirm nothing changed underneath, writes it back with `conv=fsync` and compares a read-back checksum. Any
  failure after the first byte restores the original automatically.
* Only units whose flash matches Synology's loader (FAT volume id `10EE-589C`, ext2 UUID `45e5b07d-...`, sizes at least 16/64 MiB) are
  edited. Anything else is refused.
* Entries added: `SYNOFURB_BOOT` (the agent), `DEBIAN`, `DEBIAN_OLD` (previous kernel). The DSM entry is never altered, and **is the
  default** except while the agent is armed (one boot) or after an install has switched to Debian; every failure path sets the default back to DSM, provided the agent or
  the app got to run ([RECOVERY.md](RECOVERY.md) describes the exception and the manual recovery, including the plain `sha256sum -c` + `dd` route that needs no synofurb tool on the other computer).
* The flash is small (the ext2 partition is at least 64 MiB and DSM's own files take most of it), so the installed system's initramfs is
  built with an explicit module list and `xz` (`MODULES=list`), and a kernel update that does not fit is refused, leaving the previous
  entry in place. Kernel updates are registered by a hook; the previous kernel is kept as `DEBIAN_OLD`.

### What Debian costs on the flash (measured 2026-11-03)

Debian trixie `minbase` plus the package list of `payload/chroot-setup.sh`, `MODULES=list`, `COMPRESS=xz`, `intel-microcode` for the host CPU only:

| Kernel | `vmlinuz` | `initrd.img` | Total |
|---|---:|---:|---:|
| Linux 6.12.107 (Stable) | 12,142,528 B | 13,603,940 B | **24.6 MiB** |
| Linux 7.2.11 (Newest, trixie-backports) | 14,329,792 B | 15,055,484 B | **28.0 MiB** |
| boot agent (see [boot/agent](../boot/agent/README.md)) | | | 18.9 MiB (6.12) / 21.8 MiB (7.2) |

`DEBIAN_OLD` doubles Debian's share. The biggest single items inside the initramfs are udev's `hwdb.bin` (13.5 MB raw, 1.7 MB compressed), `libcrypto`, `lvm` and `xfs.ko`;
dropping `hwdb.bin` through an initramfs hook would save about 1.7 MB but is not done (not tried on hardware). Without `xz-utils` in the target `initramfs-tools` silently falls back to
`gzip` (+2.8 MB); the installer installs it before the first kernel.

Rules the tools follow, so that a flash that is too small is found **before** anything is destroyed and never leaves the unit without a boot entry:

1. **Before the point of no return** the stage-2 script asks `synofurb-dom status --json` for the free bytes of the DSM partition (counting the `.old` fallback files as free) and
   needs 32 MiB for Debian's kernel + initramfs (`DOM_DEBIAN_NEED_MIB`). If it is short it removes everything synofurb added (the boot agent is not needed once the installer runs),
   re-checks, and stops the install with `@@FAIL preflight: the boot flash has only N MiB free ...` if it is still short. Nothing was changed; DSM stays the default.
2. **`install-debian`** (called by the kernel hook) works on copies and writes only when everything fits: the current kernel is first rotated to `vmlinuz.old`/`initrd.img.old`,
   the old fallback is deleted if there is no room, and if the new kernel still does not fit nothing is written. DSM's files are never touched.
3. **The hook** (`synofurb-dom-update --hook`) turns a refusal into an error message on the console, in syslog and in `/var/lib/synofurb/dom-update.failed`, and exits 0: a failing
   kernel hook would leave the `linux-image` package half-configured and block every later `apt` run. The unit keeps booting the kernel it booted before.
4. The **default** boot entry is changed to Debian only by the last step of the install (`synofurb-dom default debian`); the hook never changes it.

### The time limits, in one place

They are constants in `synofurb_core::timing` (the GUI and the CLI texts read them there; a test keeps the agent's built-in 20 in step with `boot/agent`):

| What | Value | Counted from |
|---|---|---|
| Agent watchdog, install (`INSTALL_WATCHDOG_MIN`) | 30 min | the agent starting |
| Agent watchdog, boot manager mode (`BOOT_MANAGER_WATCHDOG_MIN`) | 60 min | the agent starting |
| Agent watchdog, built-in default (`AGENT_DEFAULT_WATCHDOG_MIN`) | 20 min | the agent starting |
| The app's wait for the agent (`AGENT_WAIT_MIN`) | 10 min | the reboot request |
| Installer failure reboot, before the point of no return | 60 s | the failure (boot-flash units only) |
| Installer failure reboot, after the point of no return | 15 min | the failure (boot-flash units only) |
| The app's wait for the installed system (`NEW_SYSTEM_WAIT_MIN`) / for DSM after a recovery (`DSM_WAIT_MIN`) | 20 min / 15 min | the end of the install / the probe starting |
| SSH connect: TCP + handshake (`CONNECT_TIMEOUT_SECS`) | 20 s | the connection starting; the time a person spends on the host-key question is **not** counted |
| A person's answer to "trust this host key?" (`HOST_KEY_ANSWER_TIMEOUT_SECS`) | 180 s | the question being asked; no answer = refused, a late answer pins nothing |

Because the watchdog and the app's wait start at different moments, an app timeout does not mean the watchdog has fired or will not: at least `watchdog - wait` minutes (20 in an install) can still be left.

### The boot agent and "boot manager mode"

[boot/agent](../boot/agent/README.md) builds a small Linux (Debian's own kernel + a tiny initramfs: busybox, dropbear SSH, kexec, our scripts).
With Linux 7.2 it is 21.8 MiB for amd64 (budget 24 MiB; 18.9 MiB with the 6.12 kernel). The arm64 build is 52.8 MiB because Debian ships that kernel
uncompressed, which is fine: only x86 units have a boot flash. It:

1. resets the default entry to DSM as its **first action** (so it is one-shot: any later reboot returns to DSM),
2. brings up DHCPv4 + SLAAC/DHCPv6 on every Ethernet port and starts SSH with a per-unit key that the desktop app baked in,
3. announces itself by a UDP beacon, and reboots into DSM by itself if nobody logs in within the timeout (the **watchdog**: 60 minutes in boot manager mode, 30 during an install, the agent's own default is 20, **counted from the moment the agent starts**; this only works once the agent is running: if it never boots, see [RECOVERY.md](RECOVERY.md) and "Manual recovery" in [boot/dom](../boot/dom/README.md)). The app itself waits only **10 minutes** for the agent to answer; when that wait ends the app resets nothing and says so (CLI exit 10),
4. offers `sf-boot` to download any kernel + initrd and `kexec` into it. Downloads over HTTP(S) are **refused without SHA-256 hashes** (its `wget` does not
   verify TLS); the app always passes them.

Security notes: anyone with a **serial or VGA console** on the unit gets a root shell from the agent without logging in (switch off with `AGENT_CONSOLE_SHELL=0`
in the overlay), and it uses per-unit SSH keys; password login exists only as a one-time password that is revoked after its first use.

That is both the transport for installing Debian on a unit with a boot flash (the app uploads the agent, reboots, logs into it
and has it `kexec` the full Debian installer that it serves from your computer) **and** the "more standard netboot mode" you can use
on its own: `synofurb boot-manager install` then `synofurb boot-manager enter`, then `sf-boot kexec <kernel-url> <initrd-url>` to try any
Linux installer or live system by hand, with DSM or Debian one `sf-boot reboot ...` away.

Limits you should know: `kexec` starts **Linux kernels**. It cannot boot Windows or an ISO that needs a UEFI firmware. `synofurb-dom install-efi`
can add an EFI application (for example iPXE or a UEFI shell) to the flash's FAT partition and a `chainloader` menu entry, which may work
on units that boot in UEFI mode **but that has not been tested** and depends on the loader's GRUB build.

## The DS918+ (Apollo Lake) as a test case

The owner's DS918+ is the reference unit for this chain: booting through the internal flash (120 MiB), serial console `ttyS2` at 115200 (DSM's own `console=`, copied to the agent, the installer
and the installed system; **the unit has no video output**), two SATA controllers (`sdc`/`sdd` on the SoC AHCI `0000:00:12.0`, `sda`/`sdb` on an add-in AHCI behind the bridge `0000:00:13.0`, which report
`removable=1`), two Intel i211 ports. Which Debian kernel modules drive that hardware:

| Hardware | Module | In the installed system's initramfs | In the installer / agent |
|---|---|---|---|
| SoC AHCI, add-in AHCI (Marvell 88SE9215/9230/9235 are in `ahci`'s PCI table; `sata_mv` is for older 88SX/88SE61xx parts) | `ahci` (+ `libahci`, `libata`) | yes (`ahci`, `sata_mv` and `sata_sil24` are listed) | `sata-modules` udeb / agent `STORAGE_MODULES` |
| disks | `sd_mod` | yes | yes |
| the boot flash (USB) | `xhci_pci`, `usb_storage`, `uas` | yes (the flash is not needed to boot, but a USB keyboard on a recovery console is) | `usb-modules`, `usb-storage-modules` / agent |
| md / LVM / XFS | `raid1`, `raid456`, `dm_mod`, `xfs`, ... | yes | `md-modules`, `xfs-modules` / not needed |
| NICs | `igb` (no firmware: the i211 keeps its NVM on the card) | no (loaded from the root file system by udev) | `nic-modules` udeb / agent `NIC_MODULES` |

Whatever the list says, `chroot-setup.sh` walks the sysfs path of every system disk in the running installer (`disk_driver_modules`) and fails the install if the finished initramfs lacks a loadable driver on that path
(`ensure_initrd_storage`), after adding it and rebuilding once. Hardware-only checks are in [HARDWARE-TEST-PLAN.md](HARDWARE-TEST-PLAN.md).

## Linux 7.x

Debian 13 "trixie" ships Linux 6.12 LTS. The kernel option **Newest** installs `linux-image-<arch>` from `trixie-backports`, where Debian
currently packages Linux 7.2 for amd64, arm64 and armhf (checked against the archive when this was written); **Stable** installs 6.12.
If the backports kernel cannot be installed, the installer falls back to Stable and logs a warning (online only: an offline bundle holds one kernel, Newest by default, and has no fallback; see [OFFLINE.md](OFFLINE.md)). The boot agent defaults to the same suite
(`--suite trixie` for 6.12). Bear in mind that md RAID superblocks written by a newer kernel are compatible with older ones for the on-disk
formats used here, but ZFS and some out-of-tree drivers lag behind new kernels.

## AMD units

AMD Synology models (Ryzen V1000 / R1000 / R1600, EPYC) are first-class: the inspection reads the CPU vendor and the installer adds
`amd64-microcode` (or `intel-microcode` on Intel). They have **no legacy I/O-port UART**, so the serial console is a memory-mapped UART:
DSM's kernel command line contains something like `console=uart8250,mmio32,0xfedc9000,115200n8`. `synofurb` copies that argument verbatim
into the agent's and Debian's kernel command lines (and `systemd` starts a login on that console automatically), instead of assuming `ttyS0`.

## Credit to synodeb

[synodeb](https://github.com/rylos/synodeb) (GPL-2.0-only) is the inspiration for booting Debian from the DOM through boot entries, and
its DS923+ report is cited in the compatibility data as a third-party report. `synofurb` uses **none of synodeb's code**: the DOM manager was
written from a behavioural description (partition magics, menu structure, copy-edit-verify-write), so synodeb's licence does not apply to
`synofurb` (MIT). The owner confirmed on 2026-11-02 that synodeb is used only as inspiration and a how-to. If its code is ever copied or
bundled, this note and the licence question must be revisited first.

## What is verified

`synofurb-dom` and the agent were exercised against **synthetic** boot-flash images and (for the agent) QEMU; see their READMEs for exact results.
The generated stage-2 script was also run live in QEMU on a DS918+-shaped machine (two AHCI controllers, a USB flash, old DSM arrays left assembled) and the resulting system booted
to a login on the third serial port and answered SSH ([HARDWARE-TEST-PLAN.md](HARDWARE-TEST-PLAN.md)). None of it has run on a real Synology yet. The DS923+ is the only model with an outside report (from synodeb) that this general technique works;
every other model is **Untested** in [compat/models.json](../compat/models.json).
