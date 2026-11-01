# How units boot (and what needs only SSH)

`synofurb` needs **SSH access to the NAS and nothing else**: no opened case, no USB stick, no extra tools on the NAS.
That holds for x86 units. ARM boards are the exception (see the table).

| Unit | How Debian is started | Needs |
|---|---|---|
| **x86-64 Synology with an internal boot flash ("DOM")**: Intel *and* AMD (Ryzen V1000/R1000/R1600, EPYC, Atom, Xeon-D, Celeron, ...) | An extra entry in the boot flash's menu loads Debian's kernel + initrd; everything else lives on the disks. A tiny SSH-controlled **boot agent** (also stored on the flash) downloads and `kexec`s the full Debian installer. | **SSH only** |
| x86-64 without an editable boot flash | `kexec` from the running DSM (needs a static `kexec` binary and `CONFIG_KEXEC` in DSM's kernel), or the installer kit | SSH only if kexec works |
| ARMv7 (`armhf`) and arm64 boards with U-Boot | Kernel + initrd written to raw disk sectors, U-Boot environment changed once | a TTL **serial adapter** (see [ARM.md](ARM.md)) |

## x86: the internal boot flash

Synology x86 units always boot a small USB flash module soldered to the mainboard (128 MB). It holds Synology's GRUB-style
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
  default** except during an install that is actively progressing; every failure path sets the default back to DSM
  ([RECOVERY.md](RECOVERY.md)).
* The flash is small (the ext2 partition is at least 64 MiB and DSM's own files take most of it), so the installed system's initramfs is
  built with an explicit module list and `xz` (`MODULES=list`), and a kernel update that does not fit is refused, leaving the previous
  entry in place. Kernel updates are registered by a hook; the previous kernel is kept as `DEBIAN_OLD`.

### The boot agent and "boot manager mode"

[boot/agent](../boot/agent/README.md) builds a small Linux (Debian's own kernel + a tiny initramfs: busybox, dropbear SSH, kexec, our scripts).
With Linux 7.2 it is 21.8 MiB for amd64 (budget 24 MiB; 18.9 MiB with the 6.12 kernel). The arm64 build is 52.8 MiB because Debian ships that kernel
uncompressed, which is fine: only x86 units have a boot flash. It:

1. resets the default entry to DSM as its **first action** (so it is one-shot: any later reboot returns to DSM),
2. brings up DHCPv4 + SLAAC/DHCPv6 on every Ethernet port and starts SSH with a per-unit key that the desktop app baked in,
3. announces itself by a UDP beacon, and reboots into DSM by itself if nobody logs in within the timeout,
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

## Linux 7.x

Debian 13 "trixie" ships Linux 6.12 LTS. The kernel option **Newest** installs `linux-image-<arch>` from `trixie-backports`, where Debian
currently packages Linux 7.2 for amd64, arm64 and armhf (checked against the archive when this was written); **Stable** installs 6.12.
If the backports kernel cannot be installed, the installer falls back to Stable and logs a warning. The boot agent defaults to the same suite
(`--suite trixie` for 6.12). Bear in mind that md RAID superblocks written by a newer kernel are compatible with older ones for the on-disk
formats used here, but ZFS and some out-of-tree drivers lag behind new kernels.

## AMD units

AMD Synology models (Ryzen V1000 / R1000 / R1600, EPYC) are first-class: the inspection reads the CPU vendor and the installer adds
`amd64-microcode` (or `intel-microcode` on Intel). They have **no legacy I/O-port UART**, so the serial console is a memory-mapped UART:
DSM's kernel command line contains something like `console=uart8250,mmio32,0xfedc9000,115200n8`. `synofurb` copies that argument verbatim
into the agent's and Debian's kernel command lines (and `systemd` starts a login on that console automatically), instead of assuming `ttyS0`.

## Licence note on synodeb

synodeb is licensed **GPL-2.0-only** and `synofurb` is MIT. `synofurb` contains **none of synodeb's code**: the DOM manager was written from a
behavioural description (partition magics, menu structure, copy-edit-verify-write). If you would rather integrate synodeb itself, for example as
the installer for the DS923+ where it ships a model-specific kernel with fan/LED/bay drivers, keep it a separate download that the user runs, or
decide on GPL-2.0 for the combined work. Please decide on this before release.

## What is verified

`synofurb-dom` and the agent were exercised against **synthetic** boot-flash images and (for the agent) QEMU; see their READMEs for exact results.
None of it has run on a real Synology yet. The DS923+ is the only model with an outside report (from synodeb) that this general technique works;
every other model is **Untested** in [compat/models.json](../compat/models.json).
