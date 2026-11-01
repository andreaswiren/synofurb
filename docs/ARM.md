# ARM (ARMv7 `armhf` and arm64) NAS units

Source for the ARMv7 method: the Debian wiki page
[InstallingDebianOn/Synology](https://wiki.debian.org/InstallingDebianOn/Synology) (documents the DS218j and RS816, both
Marvell Armada 385, last edited 2021). synofurb automates the parts that can be automated and prints the rest as exact
commands. **None of this has been run on a real board by the authors**; treat it as a documented procedure with tooling
around it, and keep a TTL serial adapter connected.

## Why ARM is different

Synology ARM boards run Synology's U-Boot from SPI flash. It starts DSM from flash (`bootspi`) and has no UEFI and no
disk-based boot menu, so GRUB cannot simply be installed to the disks. The Debian wiki's approach, which synofurb follows:

* the first **50 MiB of every system disk stays outside the GPT**; partitions start at sector 102400;
* the Debian kernel (a `uImage` with the board's DTB appended to `zImage`) is written **raw at 10 MiB**, the initrd
  (`uInitrd`) **raw at 20 MiB**;
* you interrupt U-Boot once on the serial console and point it at those sectors (`scsi read ...; bootm`);
* root is on LVM/xfs as everywhere else; the initramfs assembles md and LVM.

After every kernel or initramfs update, `/etc/kernel/postinst.d/zz-synofurb-uboot` rebuilds both images and rewrites them
to **every** system disk (so any mirror boots), and regenerates `/boot/uboot-env.txt` with the exact U-Boot commands,
including the correct hexadecimal block numbers and counts.

## What you have to do

1. **Serial adapter.** 3.3 V TTL (not RS-232 levels), 115200 baud 8N1. Debian wiki pin order for the board header: pin 2 GND,
   pin 4 TX, pin 6 RX (adapter RX/TX crossed). The pinout differs per model: check yours.
2. Run synofurb. Pick **Prepare an installer kit** (kexec on ARM DSM kernels is usually not compiled in; the
   app detects this and says so). It writes `vmlinuz`, `initrd-synofurb.gz`, `preseed.cfg` and a README.
3. Serve the kit by TFTP, interrupt U-Boot with Ctrl+C and run the recipe from the kit's README. The generic recipe is
   `tftpboot` of DTB, kernel, initrd, then `bootz`. Variable names and addresses differ between U-Boot builds; use
   `printenv`.
4. The installer runs exactly as on x86 (stage-2 script, live log, point of no return). **Before it reboots, it prints the exact
   U-Boot commands** (lines starting `uboot-env|` in the log; the GUI shows them in a yellow "ACTION NEEDED" box with a copy
   button; they are also saved on the NAS as `/boot/uboot-env.txt`). After the reboot U-Boot would start DSM from flash, and DSM's
   system partition is gone, so the NAS will not come up on its own: interrupt U-Boot with Ctrl+C and paste those commands.
5. Test with `run sf_boot`. Only when Debian comes up, make it permanent: write down `printenv bootcmd` first, then
   `setenv bootcmd 'run sf_boot; bootspi'; saveenv`. (`bootspi` stays as a fallback to DSM.)
6. Some boards need extra setup (the wiki lists `mw.l f1018100 00008000` for the DS218j). That is board-specific and is
   not automated.

## Device trees

Debian's kernel packages ship DTBs for **three** Synology boards only:

| Model | DTB |
|---|---|
| DS213j (Armada 370) | `armada-370-synology-ds213j.dtb` |
| DS116 (Armada 385) | `armada-385-synology-ds116.dtb` |
| DS414 (Armada XP) | `armada-xp-synology-ds414.dtb` |

For anything else (the wiki's DS218j and RS816 used a DTB from the Doozan forum) supply your own `.dtb`: put it in
`/etc/synofurb/` on the NAS (or in the kit folder) and set `DTB=` in `/etc/synofurb/uboot.conf`. The GUI/CLI
(`--dtb`) accept a file name known to Debian's installer; custom files must be placed by hand.

## Other things the wiki warns about

* **Entropy**: ARM boards may hang at `openssh-server` start with no entropy. synofurb installs `haveged` on armhf.
* **LEDs / power-off** quirks on some Debian ARM kernels (cosmetic).
* **ext3 `LABEL=rootfs`** was what the wiki tested; synofurb uses xfs-on-LVM and `root=UUID=...`, which is untested on these boards.
* **32-bit limit**: a 32-bit kernel can address at most 16 TiB per filesystem; the planner warns if the data volume is larger.
* **RAM**: 512 MiB boards (DS218j class) can run the installer but it is tight; the planner refuses below ~480 MiB.

## arm64 (Realtek RTD1296/1619B, Alpine v2, Armada 37xx)

The installer and the Debian system install work generically (`debootstrap --arch=arm64`, `linux-image-arm64`). Booting from
disk through U-Boot is **not automated** for arm64 (the wiki only covers armhf); the system is installed with kernel and
initrd in `/boot` and a warning is logged. Mainline support for these SoCs is partial; expect missing drivers.
