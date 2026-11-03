# First hardware run: DS918+ ("awss01")

A step-by-step, safe procedure for the first time `synofurb` meets the owner's real DS918+. It is written so that every step either
changes nothing or can be undone, and so that the first step that can destroy data is reached only with a backup that has been
**verified**, not merely made.

> **Never run an install on this unit without a verified backup.** Nobody who wrote this plan has access to the unit; everything
> below that says "expected" is what the software is built and rehearsed to do, not something that has been seen on this hardware.
> The section [What is verified, and where](#what-is-verified-and-where) separates the two.

## The unit (facts read from its own sysfs and DSM)

| | |
|---|---|
| Model / CPU / RAM | DS918+, Intel Celeron J3455 (Apollo Lake), 4 GiB, DSM 7.2.1-69057, kernel 4.4.302+ |
| Boot | internal USB flash module ("DOM") `synoboot`, 120 MiB, reported **editable** (both loader magics match), on `usb1/1-1` behind xHCI `0000:00:15.0` |
| DSM kernel command line | `root=/dev/md0 console=ttyS2,115200n8 ...` -> **serial console is ttyS2 at 115200** (the third legacy UART, I/O port 0x3e8). The unit has **no video output** |
| Disks | 4 x ST8000NM0055 (8 TB, SATA). `sdc`, `sdd`: SoC AHCI `0000:00:12.0` (ata1, ata2). `sda`, `sdb`: an add-in AHCI controller `0000:01:00.0` behind the bridge `0000:00:13.0` (ata3, ata4); they report `removable=1`, which `synofurb` no longer lets exclude them |
| Network | 2 x Intel i211 (`igb`; needs no firmware) |
| Live data | DSM `md0`/`md1` RAID1 over all 4 disks, `md2` and `md3` RAID5 (one LVM pool `vg1`: `volume_1` ext4, `volume_2` btrfs, behind `cachedev` devices). `md2` was still **resyncing (90.8 %)** when it was captured |

The repository's fixture of this unit is `synofurb_core::inspect::fixtures::ds918_two_volumes()`; as a file:
`tests/cli/ds918-real.inspect.txt` (`synofurb-cli plan --facts tests/cli/ds918-real.inspect.txt --raid shr1 --copies 4` works on any computer, no unit needed).
Two things in the fixture are placeholders until step 2 replaces them with the unit's real numbers: the partition sizes of the boot flash (the unit reports it as 120 MiB and editable; the split between the FAT and ext2 partitions is a guess) and the mount sizes and usage.

## What you need

* A **serial adapter** (3.3 V TTL or the unit's RS-232 header, whichever the unit has; 115200 8N1) and a terminal program, connected **before** step 6.
  The unit has no video output: the serial console is the only place where the boot menu, kernel messages and the installer log are visible, and the only way
  to pick a boot entry when the network is down.
* A second computer on the same LAN that stays on for the whole run (it holds the backups, the DOM image and serves the installer).
* **Backup storage** at least as large as the used data (volume1 ~ 8.5 TiB + volume2 ~ 2.3 TiB in the fixture's sample numbers; check yours) plus the same again for a second copy of anything irreplaceable.
* **A spare disk set** for the first install: one or more disks of at least 8 GiB each (see step 7), or empty slots to run with after pulling the real disks.
* A way to look at the DHCP server's lease list (the installed system announces itself as `nas` / the host name you choose).
* A USB-to-SATA adapter or another Linux computer (only for the "boot flash from another computer" rollback).

## Step by step

Each step names what it proves. Do not skip ahead; stop at the first thing that does not match.

### 0. Rehearse on your computer (nothing touches the unit)

```sh
synofurb-cli version
synofurb-cli plan    --facts tests/cli/ds918-real.inspect.txt --raid shr1 --copies 4 --kernel stable
synofurb-cli install --facts tests/cli/ds918-real.inspect.txt --raid shr1 --copies 4 --kernel stable \
                 --hostname nas --username nasadmin --have-backup --accept-data-loss --confirm "ERASE DS918+" \
                 --dry-run --out ./review-ds918
```

Read `review-ds918/stage2.sh` once from top to bottom. The things to confirm (all of them are also checked by tests in the repository):

* `firmware=Dom`; no `bios_grub`, no `/boot`, no ESP: the disks carry no boot loader, the kernel and initramfs go to the flash.
* `resolve_disk D_sda ... ZA1EJ7BG` and the same for `sdb`, `sdc`, `sdd`: disks are found by **serial**, not by name. Debian may number the controllers differently from DSM
  (QEMU with the same topology numbers the SoC controller first: DSM's `sda` became `sdd`); the script maps by serial and prints `disk sda (...) -> /dev/sdd`.
* `dom_flash_check 32` and `release_storage` come **before** `@@POINT_OF_NO_RETURN`; `default debian --yes` is the **last** thing before the reboot, after the chroot step.
* The installer command line `review-ds918/installer-cmdline.txt` ends in `console=tty0 console=ttyS2,115200n8`; `payload/env.sh` has `SERIAL_ARG='console=ttyS2,115200n8'`, `FIRMWARE='Dom'`.

*Proves:* the tool is usable and the plan matches the facts. Not a hardware test.

### 1. Wait for the resync, then back up the data (this step decides everything)

`md2` was resyncing when the facts were captured. An install on a pool that is still resyncing starts from a pool that is not fully redundant: let it finish (DSM:
Storage Manager shows the pool as Healthy; `cat /proc/mdstat` shows no `resync`).

**What to copy** (everything that cannot be recreated):

| What | How | Verify |
|---|---|---|
| All shared folders on `/volume1` (ext4) and `/volume2` (btrfs): documents, photos, media, ... | `rsync -aHAX --numeric-ids` to the backup storage (not a Synology "backup package": plain files you can read on any Linux computer) | `rsync -aHAXc --dry-run --itemize-changes` second run prints **nothing**; file counts and `du -sb` match; open a few of each kind |
| Docker data under `/volume2/@docker` and compose / container settings | copy the volume directories **and** write down image names/tags and environment (`docker inspect`, `docker compose config`) | restore one container on another machine |
| Synology Active Backup for Business data (`/volume2/ActiveBackupforBusiness`) | it is deduplicated and only readable by that package: copy only if you can restore it elsewhere; otherwise treat it as lost | -- |
| DSM configuration | Control Panel > Update & Restore > Configuration Backup (`.dss`); a screenshot list of installed packages and users | the file exists and is not 0 bytes |
| btrfs snapshots | **are not backups** (same pool); copy what matters out of them | -- |

Then **prove restorability**: pick 5 important files on the backup, open them; compare a SHA-256 manifest (`find ... -type f -print0 | xargs -0 sha256sum`) of source and backup.
Keep one copy of irreplaceable data **off this network** if it exists nowhere else. Only then write the sentence "I have a verified backup" in your notes; the installer's own
confirmation (`--have-backup`) is a switch, not a check.

*Proves:* a failure in any later step costs time, not data.

### 2. Read the unit (read-only)

```sh
synofurb-cli scan
synofurb-cli compat DS918+
synofurb-cli inspect --host <ip> --user <admin> --save ds918-live.json     # writes nothing on the unit
synofurb-cli plan    --host <ip> --user <admin> --raid shr1 --copies 4 --kernel stable
```

Compare with the fixture: model `DS918+`, **firmware: internal boot flash (DOM)**, serial console `console=ttyS2,115200n8`, 4 candidate disks `sda`..`sdd` (a "not used" line for `sda`/`sdb`
means the removable fix is missing: stop), the arrays and volumes, the md2 warning gone if the resync finished. **Write down the DOM numbers** the inspection prints (flash size, partition 1 and 2 sizes): replace the
placeholders in `inspect.rs`'s fixture with them (the boot flash partition sizes decide how much room Debian has).

If the plan says Bios instead of DOM, stop: the inspection did not recognise the flash as editable, and a BIOS-style install would not boot on this unit.

*Proves:* the real facts match the assumptions of every later step.

### 3. Back up the boot flash and the partition tables (before the first write)

```sh
synofurb-cli backup-dom  --host <ip> --user <admin> --backup-dir ~/ds918-backups
synofurb-cli disk-backup --host <ip> --user <admin> --out-dir ~/ds918-backups
```

`backup-dom` streams `/dev/synoboot` to your computer and compares its SHA-256 with one computed on the unit; it writes `synofurb-dom-backup-<serial>-<time>.img`, `.img.sha256` and `.img.json`.
Then, yourself: `sha256sum -c *.img.sha256` (must say OK), copy the three files to a **second** place, and check that the image is 120 MiB (`ls -l`). This image is what brings
the unit back if the flash is ever damaged ([RECOVERY.md](RECOVERY.md) "Manual recovery").

*Proves:* the boot flash can be read completely and restored. Nothing on the unit changed.

### 4. Boot manager: prove the whole boot chain without touching a disk

This is the most valuable step. It writes to the flash (reversibly), boots Debian's own kernel on this hardware, and leaves DSM as the default.

```sh
synofurb-cli boot-manager status  --host <ip> --user <admin>              # what the flash tool reports; read-only
synofurb-cli boot-manager install --host <ip> --user <admin>              # backup (again), then the agent; DSM stays the default
synofurb-cli boot-manager enter   --host <ip> --user <admin> --yes        # arm for ONE boot, reboot DSM, wait for the agent
```

Have the serial console open. Expected on the console, in this order (times are guesses; the unit takes a minute or two to reboot):

1. Synology's boot loader menu (`SYNOLOGY_1`, `SYNOLOGY_2`, `SYNOFURB_BOOT`) and the agent entry being chosen once (it is armed for one boot).
2. Kernel messages on **ttyS2**: `ahci 0000:00:12.0: AHCI ...` and `ahci 0000:01:00.0: AHCI ...` (both controllers), `ata1`..`ata4: SATA link up`, `usb-storage ... USB Mass Storage device detected`
   (the DOM), `igb 0000:0x:00.0 ...` for both ports. **If nothing at all appears, the serial port number is wrong:** see "If something does not match" below.
3. `boot agent starting: ...`, then `boot default set to DSM (synofurb-dom default dsm, attempt 1)` (the fail-safe fires as the first thing; `attempt` > 1 only means the USB flash showed up late),
   `SSH: root@<ip> port 22, 1 authorized key(s) ...`, and a `READY` banner. The app prints `the boot agent answered` and how to log in.

Then, on the agent (`ssh root@<ip>`), look at the hardware and write the answers down:

```sh
sf-boot status                    # dom_default=ok, agent_dom_stub=0
sf-boot net                       # both igb ports, their addresses
ls -l /sys/block/ | cat           # sda..sdd + the flash; note which controller each is on
for d in /sys/block/sd*; do echo "$d $(cat $d/device/model) $(cat $d/device/vpd_pg80 2>/dev/null | tail -c +5)"; done
dmesg | grep -Ei 'ahci|ata[0-9]|usb-storage|igb|firmware|error|fail|ttyS|serial' | head -60
```

Then prove the **watchdog** once: `sf-boot arm 3`, log out, do not touch it. After about 3 minutes the unit must reboot into DSM by itself (`synofurb recover wait --host <ip> --timeout 15`
says DSM is back). Then `synofurb recover status --host <ip>` must show the default entry as DSM.

*Proves:* the flash write path on this flash; that Synology's loader boots a Debian kernel entry; Debian's kernel on Apollo Lake; **both AHCI controllers** and all four disks visible; the USB flash
visible to Linux (which `synofurb-dom` needs); `igb`; the ttyS2 console; DHCP; the fail-safe default; the watchdog. No disk was written.

### 5. Choose the install target

Two ways. Take the first for the very first run; the second only when step 4 and the dress rehearsal on spare disks worked.

**A. Spare disk set (recommended).** Power the unit off, label and pull the four data disks (note the slot of each), insert 1 to 4 spare disks, power on, install DSM 7.2.1 on them
through Synology's own installer (`find.synology.com`; the DOM still holds the loader), set up SSH. Repeat steps 2-4 on this unit state, then step 6 below. The spare layout
is only a DSM system + swap, so the plan will be smaller than the production plan; that is fine: it exercises everything except the data disks' size. Keep the real disks
in a drawer; re-inserting them in their slots later brings DSM back with all data (DSM's loader on the flash is restored by step 9 if needed).

**B. The real disks.** Only with the backup of step 1 verified, the resync finished, and a go from you. Everything on the four disks is destroyed.

### 6. Install

Use the serial console and the app's log side by side. The command (the confirmation phrase is the one the plan printed):

```sh
synofurb-cli install --host <ip> --user <admin> --raid shr1 --copies 4 --kernel stable \
                 --hostname nas --username nasadmin --authorize-key ~/.ssh/id_ed25519.pub --password-file ~/nas-pw.txt \
                 --have-backup --accept-data-loss --confirm "ERASE DS918+"
```

(`--copies 4` mirrors the system over all four disks so the data arrays are even; `--kernel stable` is Linux 6.12, 3.5 MiB smaller on the flash than 7.2 and the less surprising first
choice. Without `--copies` the plan warns about 32 GiB left unused on one disk. `--mode keep` cannot be combined with `--raid shr1`: this plan wipes all four disks.)

What you should see, in order (the lines are the installer's own markers; they appear on the console and in the app's log):

```
@@BEGIN run=... arch=amd64 firmware=Dom mode=WipeAll dry=0
INFO  boot flash manager: /usr/local/sbin/synofurb-dom
@@STEP 1/12 Loading installer components ...
@@STEP 2/12 Preflight: locating disks and verifying they match the inspected NAS
INFO  disk sda (ST8000NM0055-1RM112) -> /dev/sdX          (one line per disk; the X can differ from DSM's name)
INFO  boot flash: NN MiB free for Debian's kernel + initramfs on the DSM partition, about 32 MiB needed
        (if NN < 32: WARN the boot flash is too full; removing everything synofurb added ... then a second "boot flash: NN MiB free after cleaning")
@@STEP 3/12 POINT OF NO RETURN: releasing existing RAID/LVM and wiping reclaimed areas
INFO  releasing: md arrays [md3 md2 md1 md0 ] device-mapper [dm-0 ] volume groups [vg1 ]
INFO  released after 1 round(s): nothing holds the disks any more
@@POINT_OF_NO_RETURN
@@STEP 4/12 Partitioning ... @@STEP 5/12 Creating RAID arrays (2 array(s)) ... @@STEP 6/12 ... @@STEP 7/12 ... @@STEP 8/12 ...
@@STEP 9/12 Installing the Debian trixie base system (debootstrap, amd64)
@@STEP 10/12 Writing fstab/mdadm.conf and unpacking the configuration payload
@@STEP 11/12 Configuring the system (...)
[chroot] mdadm.conf lists 2 array(s)
[chroot] initramfs of 6.12.xx+deb13-amd64 carries the storage drivers of the system disks: sd_mod ahci
 - Debian kernel /synofurb/vmlinuz (11.6 MiB), initrd /synofurb/initrd.img (13 MiB)
 - menu entry DEBIAN
 P2 written and verified (sha256 ...)  /  P1 written and verified (sha256 ...)
@@STEP 12/12 Finishing: saving logs, unmounting, rebooting
INFO  making Debian the default entry of the boot flash (DSM stays selectable)
 - boot menu default -> DEBIAN (index 2)
@@DONE install completed
```

Long steps: step 5 starts the RAID5 initial resync (hours for 4 x 8 TB; the install does not wait for it and the system is usable meanwhile);
step 9-11 download ~500 MB from the mirror (use the offline bundle on a LAN without Internet: [OFFLINE.md](OFFLINE.md)).
`@@FAIL` before `@@POINT_OF_NO_RETURN`: nothing was written; the unit reboots to DSM in 60 s. After it: the unit reboots into the DSM **loader** in 15 minutes;
DSM's system partition is gone, so see [RECOVERY.md](RECOVERY.md) "Reinstalling DSM" and use the backup.

### 7. First boot of Debian: what to check

On the serial console: Synology's menu, then the `DEBIAN` entry (it is the default now), kernel messages on ttyS2, the initramfs assembling `md` and LVM (a `rootdelay=5` pause is normal),
`Welcome to Debian GNU/Linux 13 (trixie)!`, then `nas login:` on ttyS2. The DHCP lease list shows `nas`. Then:

```sh
ssh nasadmin@nas.local           # or the lease address; key login first, password as set
cat /proc/mdstat                 # system (RAID1 x4) and data1 (RAID5 x4, resync running), no md2/md3 of DSM
sudo pvs; sudo vgs; lsblk        # only vg_sys and vg_data; NO vg1
sudo mdadm --examine --scan      # only the new arrays (names system, data1); the old DSM UUIDs are gone
sudo synofurb-dom status         # default = DEBIAN, DEBIAN_OLD absent on the first install, SYNOLOGY_* untouched
cat /etc/synofurb/dom.conf       # ROOT_UUID, SERIAL_ARG=console=ttyS2,115200n8
cat /proc/cmdline                # root=UUID=... ro rootdelay=5 panic=60 console=tty0 console=ttyS2,115200n8
systemctl --failed               # nothing (autofs4 is not needed)
ip -br a; ethtool -i <interface> | head -3    # driver: igb, for both ports
sudo journalctl -b -p warning | head -40
```

Then the robustness checks, one at a time, each followed by a reboot that must come back by itself:

1. **Cold power cycle** (pull the plug 10 s): comes back as Debian without help.
2. **Degraded boot**: power off, pull one disk, power on: the system RAID1 starts degraded and boots (allow ~1 minute for `mdadm` to give up waiting); put the disk back, `sudo mdadm --add` it.
3. **Kernel update path**: `sudo synofurb-dom-update --hook` says "Nothing to change"; after a real kernel update `synofurb-dom status` shows the new files, the old ones as `DEBIAN_OLD`.
   Pick `DEBIAN_OLD` in the serial boot menu once to see that it boots.
4. **Wrong serial port test** (only if step 4 showed nothing on ttyS2): see below.

*Proves:* the init system, the initramfs with `MODULES=list` on **both** AHCI controllers, md + LVM + XFS root, `igb`, the getty on ttyS2, SSH, the flash entry and default.

### 8. Make DSM-less life easier (optional, later)

`synofurb manage health --host nas`. Not part of this plan.

### 9. Rollback

| Where you are | Do this |
|---|---|
| Before step 6, any failure | nothing changed except flash files: `synofurb recover remove --host <ip> --yes` (takes our entries out, default DSM, read back) |
| Agent never answered (CLI exit 10) | see [RECOVERY.md](RECOVERY.md) "When the boot agent does not answer": wait for the watchdog (30/60 min after the agent *started*); if the agent never ran, pick `SYNOLOGY_2` in the loader menu **on the serial console**, then `synofurb-dom default dsm --yes`; last resort: the flash image in another computer (`dd`, with `sha256sum -c` first) |
| Install failed before `@@POINT_OF_NO_RETURN` | the unit reboots to DSM by itself after 60 s; check with `synofurb recover status` |
| Install failed after it | disks are changed; the unit boots the DSM loader after 15 minutes. Reinstall DSM from `find.synology.com`, restore the backup (spare-disk runs: re-insert the real disks instead) |
| Debian installed but misbehaving | serial console: choose `DEBIAN_OLD` or `SYNOLOGY_2` (the latter only shows a reinstall page if the disks were the real ones); `synofurb recover boot-dsm --host nas --reboot --yes` once it is up |
| Everything lost on the flash | `synofurb recover restore --host <ip> --backup FILE.img --yes`, or from another computer: `sha256sum -c`, then `dd if=FILE.img of=/dev/sdX bs=1M conv=fsync` on the **flash**, not on a disk |
| Spare-disk run is over | power off, pull the spare disks, re-insert the real disks in their slots, power on: DSM boots as before (restore the flash image from step 3 if the loader menu still shows Debian entries: `synofurb-dom remove` from any Linux, or the image) |

## If something does not match

* **Nothing on the serial console in step 4** (no kernel messages after the loader menu, but the unit comes up on the LAN): the kernel is sending its console to a port you are not listening on.
  Try ttyS0/ttyS1 on the adapter; the unit's physical header may be wired to another UART than DSM's `console=ttyS2` suggests. Report which port shows output; `install --serial ttyS0` sets the
  installer's and the installed system's console (the boot-manager commands have no such option yet: they copy DSM's). Do not install Debian before you can read its console.
* **One or both `ahci` controllers missing in `dmesg`** (step 4): note the PCI ids from `lspci -nn` on DSM (`/sys/bus/pci/devices/*/vendor` and `device`); the Marvell 88SE92xx family is driven by
  `ahci` (`1b4b:9215`, `9230`, `9235` are in its table); `sata_mv` is for the older 88SX/88SE61xx parts. If the id is neither, add its module to the list in `payload/chroot-setup.sh`.
* **`dom_default=failed`** in `sf-boot status`: the flash was not visible in time or `synofurb-dom` refused; `sf-boot log | grep -i dom`.
* **Plan says "boot flash too small"** or the install log says the flash is too full after cleaning: the DSM partition of the flash cannot take Debian's ~28 MiB kernel + initramfs. Stop; nothing was written.
  The budget (measured, `MODULES=list`, xz, 2026-11-03): Linux 6.12.107 = 12.1 MB kernel + 13.6 MB initramfs = **24.6 MiB**; Linux 7.2.11 = 14.3 + 15.1 MB = **28.0 MiB**; the agent is 18.9 / 21.8 MiB;
  `DEBIAN_OLD` doubles Debian's share (it is dropped automatically when there is no room).
* **The installed system does not come up and the console shows the initramfs prompt**: `cat /proc/mdstat`, `ls /dev/disk/by-uuid`, `dmesg | grep -i 'ahci\|ata'`; compare with step 4.

## What each step proves

| Step | Changes anything? | Proves |
|---|---|---|
| 0 Rehearse | no | tool, plan, stage-2 script for these facts |
| 1 Backup | no | data survives any later step |
| 2 Inspect | no | real facts = fixture (DOM, ttyS2, 4 disks, volumes) |
| 3 Flash + table backups | no | the flash can be restored |
| 4 Boot manager | flash files + one-shot entry | flash write, loader -> Debian kernel, both AHCI controllers, USB flash, igb, ttyS2, DHCP, fail-safe, watchdog |
| 5-6 Install | disks (target) | wipe of old md/LVM, partitioning by serial, md/LVM/XFS, flash space check, initramfs storage check, DEBIAN entry last |
| 7 First boot | no | init, initramfs on both controllers, root on md+LVM, igb, getty on ttyS2, SSH, no old arrays |
| 7.1-7.3 Robustness | no | power cycle, degraded boot, kernel update path |

## What is verified, and where

Verified on the build computer (no unit): the stage-2 script for these exact facts (renders, valid `sh`, order of steps, one array per rule), the flash check and the array release
against stand-in tools, the initramfs storage check against a sysfs shaped like this unit, the kernel hook (including a refused kernel), `synofurb-dom` against synthetic flashes (312 of 314 checks;
the 2 failures are loop-device scanning in the sandbox), and a **QEMU rehearsal** with this topology (SoC AHCI at 00:12.0 and an add-in AHCI behind the 00:13.0 root port, USB flash on
xHCI 00:15.0, two `igb` ports, the third serial port as ttyS2): DSM-like old arrays and an LVM volume group left assembled, then the generated stage-2 script run live, then the
resulting flash entry booted. In that run the old arrays and `vg1` were stopped and gone, the new system booted to a login on ttyS2, answered SSH over `igb`, and `systemctl is-system-running` said `running`.
QEMU differs from the unit in what matters here: it is not an Intel Apollo Lake SoC, the AHCI controllers are Intel ICH9 look-alikes (not the Marvell 88SE92xx the add-in card is, whose `ahci` support is
confirmed only from the module's alias table), the flash is not Synology's, and Synology's own loader (which reads the menu on the flash) was not part of it.

Verified **only on the unit** (this plan is how): the Synology loader booting the added entries; the ttyS2 mapping of the unit's physical header; the Marvell add-in controller with Debian's `ahci`; the flash's
real partition sizes and free space; the time and noise of a real 8 TB RAID5 build; the DSM kernel's USB flash being visible to Debian in the same way; everything about the real disks.
