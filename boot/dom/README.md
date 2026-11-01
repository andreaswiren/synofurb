# synofurb-dom

A safe boot-menu manager for the internal boot flash module (the "DOM") of
Synology x86 NAS units, written as one POSIX shell script.

`synofurb-dom` lets you add extra boot entries (a small boot agent, a Debian
kernel with a fallback kernel, optionally an EFI application) to the menu that
Synology's boot loader shows, **without ever changing the DSM entry or the DSM
kernel/initrd files**, and with a defence-in-depth write procedure so that a
failure cannot leave the unit unbootable.

It is part of the synofurb monorepo (MIT licensed). It is an independent
implementation written from a behavioural description; it contains no code from
any other boot-menu tool.

> **Status: tested only against synthetic images.** Everything in this
> directory was developed and tested on a root VM against *synthetic* DOM images
> that mimic the documented layout (see [What is NOT verified](#what-is-not-verified-on-real-hardware)).
> It has not been run on a real Synology unit. Take a full backup first (the
> tool insists on one) and read the limits below.

## Contents

- `synofurb-dom` - the script (`/bin/sh`; dash and busybox ash; `shellcheck -s dash` clean)
- `tests/run-tests.sh` - automated test harness (synthetic images, loop devices)
- `README.md` - this file

## Where it runs

As root, in three places, with the same script:

| Where | Notes |
| --- | --- |
| DSM | DOM is `/dev/synoboot`, partitions `/dev/synoboot1` (FAT) and `/dev/synoboot2` (ext2) |
| the installed Debian | the DOM looks like a normal USB disk, `/dev/sdX`; found by scanning `/sys/block` |
| a small initramfs | busybox + e2fsprogs + dosfstools is enough |

Tools used: `dd od sha256sum awk sed cut tr df mount umount cmp mktemp readlink
losetup`-capable `mount -o loop`, `e2fsck` (required for writes), `fsck.vfat`
(or `fsck.fat`/`dosfsck`; optional, a warning is printed if missing), `blockdev`
(optional). If the kernel cannot mount `vfat` but `mtools` is installed, the FAT
partition is edited with `mcopy`/`mdir` on the image copy instead (this is how
the tests run in a VM kernel without vfat).

## How the DOM is recognised

The tool only ever writes to a device when **both** partitions match:

| | partition 1 (FAT) | partition 2 (ext2) |
| --- | --- | --- |
| identity | volume ID bytes `9c 58 ee 10` (serial `10EE-589C`) at offset 39 (FAT12/16) or 67 (FAT32) | ext magic `53 ef` at offset 1080 and filesystem UUID `45e5b07d-4783-4867-a369-f99c0cd1e610` at offset 1128 |
| minimum size | 16 MiB | 64 MiB |
| holds | `EFI/boot/SynoBootLoader.conf` (GRUB-syntax menu) | `zImage`, `rd.gz` (DSM kernel and initrd) |

Partitions are located with the kernel partition nodes (`/dev/synoboot1`,
`/dev/sdX1`) when they exist, otherwise by reading the MBR or GPT with `dd` and
`od` (no loop partition nodes are needed). Partitions larger than 1 GiB are
rejected as "not a DOM".

Auto-detection order: `SYNOFURB_DOM1`+`SYNOFURB_DOM2`, then `SYNOFURB_DOM`, then
`/dev/synoboot`, then a scan of `/sys/block/*` (loop/ram/dm devices skipped
unless `SYNOFURB_SCAN_ALL=1`). Zero or more than one match is an error.

## On-DOM layout after installation

```
partition 1 (FAT)
  EFI/boot/SynoBootLoader.conf     menu; our entries sit in one marked block
  EFI/synofurb/NAME.efi            optional EFI applications (install-efi)

partition 2 (ext2)
  zImage, rd.gz                    DSM - never touched
  synofurb/agent.vmlinuz           boot agent kernel   (install-agent)
  synofurb/agent.initrd            boot agent initrd
  synofurb/vmlinuz, initrd.img     current Debian      (install-debian)
  synofurb/vmlinuz.old, initrd.img.old    previous Debian (fallback)
  synofurb/MANIFEST                name, sha256, size, install date of the above
```

Our menu entries live between two marker lines and nowhere else:

```
### synofurb begin (managed by synofurb-dom)
menuentry "SYNOFURB_BOOT" {
	search --fs-uuid --no-floppy --set=root 45e5b07d-4783-4867-a369-f99c0cd1e610
	linux /synofurb/agent.vmlinuz <cmdline>
	initrd /synofurb/agent.initrd
}
menuentry "DEBIAN" { ... linux /synofurb/vmlinuz <cmdline> ... initrd /synofurb/initrd.img }
menuentry "DEBIAN_OLD" { ... vmlinuz.old / initrd.img.old, cmdline of the previous install }
menuentry "EFI_<name>" {
	search --fs-uuid --no-floppy --set=root 10EE-589C
	chainloader /EFI/synofurb/<name>.efi
}
### synofurb end
```

Editing rules for `SynoBootLoader.conf`: everything outside the block is kept
byte for byte (including CRLF line endings and a missing final newline); the
block uses the file's own line ending; `set default='N'` keeps its quote style;
entries are counted from 0 by `menuentry` lines; re-running is idempotent; the
default is tracked **by entry name**, so adding or removing our entries never
silently moves the default to a different entry. If one of our entry names
already exists outside the block the tool refuses instead of duplicating it.

## Fail-safe design

The DSM entry `SYNOLOGY_2` stays selectable and is the default whenever you are
not deliberately booting something else.

"One-shot" boots work like this: `install-agent --arm` points the default at
`SYNOFURB_BOOT` for the next boot only. The booted agent must reset the default
to DSM as its very first action by running `synofurb-dom default dsm --yes`
(this script is meant to be bundled into the agent initramfs). If the agent
never runs, the unit would keep booting the agent: keep the manual recovery
below at hand.

## The safety algorithm (every write)

1. Require root. Take a lock (`SYNOFURB_LOCK`, default `/tmp/synofurb-dom.lock`).
2. Locate partitions 1 and 2, verify magics and minimum sizes. Anything else: refuse.
3. Refuse if any DOM partition, the DOM itself, or a loop device attached to a
   DOM image file is mounted (`/proc/mounts`).
4. Require temp space under `SYNOFURB_TMP` (default `/tmp`): `2 x (P1 + P2) + 16 MiB`.
5. Read each partition **twice** and compare sha256: the device must not change
   underneath us.
6. `e2fsck -fn` and `fsck.vfat -n` on the copies **before** editing (a failing
   fsck refuses, `--ignore-fsck` overrides; missing `e2fsck` refuses, missing
   `fsck.vfat` warns).
7. Loop-mount the *copies*, check the DSM entry and the DSM files are present
   and remember sha256 of `zImage` and `rd.gz`, then edit.
8. Unmount, fsck **again** (any problem is fatal), re-mount read-only and
   verify: DSM entry present, DSM files byte-identical, default index points
   to an existing entry, every file referenced by our entries exists, no
   duplicate names, MANIFEST matches, and no line outside our block differs.
   The result is also compared against problems that already existed so a
   pre-existing issue never blocks a repair such as `remove`.
9. Show the planned changes and ask for confirmation (`--yes` skips; without a
   terminal `--yes` is required; `--dry-run` stops here).
10. One-time automatic backup of the whole DOM (see below), verified by re-reading.
11. Re-read the device partitions and confirm they still hash to what was read in
    step 5.
12. Ignore `HUP INT TERM PIPE` and write only the changed partitions with
    `dd conv=fsync` (partition 2 first when adding files, partition 1 first when
    removing them, so the menu never references files that are not there).
13. Flush the device's buffer cache (`blockdev --flushbufs`), read each written
    partition back and compare sha256 with the edited copy.
14. On **any** failure after the first byte was written (including a signal, `dd`
    error, read-back mismatch) the original partition images are written back
    automatically and verified; the tool says so (`ROLLBACK OK`). If even that
    fails the temp directory with the original images is kept and the message
    says where.

The one-time automatic backup is written before the very first write ever: the
whole DOM as `synofurb-dom-first-backup.img` (+ `.sha256`, mode 600) in
`$SYNOFURB_STATE` (default `/var/lib/synofurb`; on DSM point it at a volume, the
system partition is small), falling back to `/tmp` with a warning, otherwise the
write is refused (`--skip-auto-backup` overrides). When only
`SYNOFURB_DOM1/DOM2` are set the two partition images are saved instead.

## Commands

All commands need root. Global options: `-y/--yes`, `--dry-run`, `--ignore-fsck`,
`--skip-auto-backup`, `--json` (status), `-h`, `-V`.

### `status [--json]`
Device, partition sizes and free space, every menu entry with index (ours are
marked), the current default (name and index), our files with size/sha256/date,
EFI applications, and whether a backup exists. Read-only.

```
# synofurb-dom status
Device:      /dev/synoboot  (128.0 MiB)
Partition 1: /dev/synoboot1  FAT 10EE-589C  20.0 MiB, free 18.5 MiB
Partition 2: /dev/synoboot2  ext2 45e5b07d-...  104.0 MiB, free 49.7 MiB
Boot menu entries (EFI/boot/SynoBootLoader.conf):
  0  SYNOLOGY_1 [DSM]
  1  SYNOLOGY_2 [DSM]   <- default
  2  SYNOFURB_BOOT [synofurb]
Default:     SYNOLOGY_2 (index 1)
```

### `backup [DIR]`
`dd` the entire DOM to `DIR/synofurb-dom-backup-YYYYMMDD-HHMMSS.img` (mode 600),
re-read the device and compare sha256, write `.sha256`, print the path. `DIR`
defaults to the current directory. Needs the whole device (not just partition
overrides).

```
synofurb-dom backup /volume1/backups
```

### `restore FILE`
Validate (size equals the DOM size, partition table present, FAT id and ext UUID
magics, `FILE.sha256` if present), require `--yes`, snapshot the current DOM for
rollback, write, read back and compare. If the current device no longer looks
like a DOM it is accepted only when you named it explicitly with `SYNOFURB_DOM`.

```
synofurb-dom restore --yes /volume1/backups/synofurb-dom-backup-20260101-120000.img
```

### `install-agent KERNEL INITRD [--cmdline STR] [--arm]`
Copy to `/synofurb/agent.vmlinuz` / `agent.initrd`, add or replace `SYNOFURB_BOOT`.
The default is **not** changed unless `--arm` (next boot only; the agent resets
it). Without `--cmdline` an existing entry keeps its cmdline (new entries get an
empty one). Needs size of changed files + 1 MiB free on partition 2.

```
synofurb-dom install-agent agent.vmlinuz agent.initrd --cmdline "console=ttyS0,115200n8" --arm
```

### `install-debian KERNEL INITRD --cmdline STR`
Rotate the current `vmlinuz`/`initrd.img` to `.old`, install the new ones,
(re)write `DEBIAN` and `DEBIAN_OLD` (the fallback keeps the *previous* command
line). Installing byte-identical files again does not rotate. Free space needed:
size of the new files + 1 MiB; if short, the `.old` files are deleted first, then
the command fails clearly if it still does not fit. `zImage` and `rd.gz` are
never deleted. Failure happens on the temporary copy, so the DOM is untouched.

```
synofurb-dom install-debian /boot/vmlinuz-6.12 /boot/initrd.img-6.12 --cmdline "root=UUID=... ro quiet"
```

### `install-efi NAME FILE.efi`
Copy to partition 1 `EFI/synofurb/NAME.efi` and add `EFI_NAME` (`chainloader`).
`NAME` is 1-32 characters of `A-Z a-z 0-9 _ -`; the file must start with `MZ`.
**Unverified**: a chainloader entry only makes sense if the unit boots this menu
in UEFI mode; on legacy BIOS boot it will not work.

### `default dsm|agent|debian|debian-old|efi:NAME`
Set `set default='N'` to the index of that entry. Refuses if the entry does not
exist or one of its files is missing.

```
synofurb-dom default dsm --yes
```

### `remove`
Remove our block, our files and EFI applications, set the default to the DSM
entry, and check that `zImage`/`rd.gz` are still there. Works even if our files
are already damaged.

### `verify`
Read-only: fsck both copies, DSM entry present, default index valid, every file
referenced by our entries exists, no duplicate names, MANIFEST matches. Exit code
non-zero if anything fails.

## Environment

| Variable | Meaning |
| --- | --- |
| `SYNOFURB_DOM` | whole device (block device or image file) |
| `SYNOFURB_DOM1`, `SYNOFURB_DOM2` | partition 1 / 2 as separate devices or files (must be set together) |
| `SYNOFURB_TMP` | work directory base, default `/tmp` |
| `SYNOFURB_STATE` | where the first backup goes, default `/var/lib/synofurb` |
| `SYNOFURB_LOCK` | lock directory, default `/tmp/synofurb-dom.lock` |
| `SYNOFURB_FAT_BACKEND` | `mount` or `mtools` (default: mount, fall back to mtools) |
| `SYNOFURB_SCAN_ALL=1` | also scan loop/ram/dm devices when auto-detecting |
| `SYNOFURB_TEST_FAIL_AFTER_WRITE=N` | **test hook**: abort after N partition writes to exercise rollback |
| `SYNOFURB_TEST_TAMPER=1` | **test hook**: modify the device just before the final re-read check |

Command lines for entries must be printable ASCII without `; { } \`` or newlines.

## Manual recovery

Everything below can be done from any Linux box with the DOM attached (a DOM is
a normal 128 MB USB flash on the mainboard header; a USB adapter works).

1. **Boot menu picks the wrong entry / agent never reset the default.** Use the
   boot loader menu to choose `SYNOLOGY_2` for this boot, then run
   `synofurb-dom default dsm --yes` or `synofurb-dom remove --yes` on DSM.
2. **Restore a backup.**
   `sha256sum -c synofurb-dom-backup-*.img.sha256` then
   `dd if=synofurb-dom-backup-....img of=/dev/sdX bs=1M conv=fsync` (check twice
   that `/dev/sdX` is the DOM, 128 MB), or `synofurb-dom restore --yes FILE`
   with `SYNOFURB_DOM=/dev/sdX`. The first-write backup is in
   `$SYNOFURB_STATE/synofurb-dom-first-backup.img`.
3. **Failed rollback.** The message names a temp directory containing
   `p1.orig` / `p2.orig`; write them back to the partitions with
   `dd if=p1.orig of=/dev/sdX1 bs=1M conv=fsync` (and `p2.orig` to `/dev/sdX2`).
4. **Edit the menu by hand.** `mount /dev/sdX1 /mnt`, edit
   `/mnt/EFI/boot/SynoBootLoader.conf`: set `set default='N'` to the index
   (counted from 0) of the `SYNOLOGY_2` entry, or delete everything from
   `### synofurb begin` to `### synofurb end`. Unmount.
5. **Reflash from scratch** only with the vendor's DOM image if all else fails.

## Tests

```
tests/run-tests.sh                              # dash
TEST_SH="busybox ash" TEST_BB=1 tests/run-tests.sh   # busybox ash (its built-in applets: awk, mount, losetup, ...)
KEEP=1 ONLY="t_debian t_space" tests/run-tests.sh    # keep temp dir / run some groups
```

The harness builds synthetic 128 MiB images (MBR, GPT, FAT16, FAT32, ext2 with
the Synology UUID, `zImage`/`rd.gz` of random data, several `.conf` variants) and
checks, among others: refusal of non-matching images; free-space failure with
rotation; idempotency (device bit-identical on re-run); DSM files untouched
(sha256 before/after); conf outside the block byte-identical; CRLF and
missing-final-newline confs; backup + restore round trip bit-identical; injected
failure during the write (rollback restores the device bit-identical); a
concurrent modification between read and write; mounted-partition refusal;
default-index math with entries in different orders; fsck-clean results.

It needs root, loop devices, `e2fsprogs`, `dosfstools`, `mtools`, `util-linux`,
`fdisk`, `xxd`, `python3` (JSON check) and optionally `shellcheck`/`busybox`.
Disk use: roughly 1 GiB of images in `$TMPDIR`.

## What is NOT verified on real hardware

Be sceptical of everything here until it has been run on a real unit:

- **Only synthetic images were used.** No real Synology DOM, DSM, or Synology
  boot loader was available. All facts about the layout (volume ID, ext UUID,
  file names, `set default` being an index, entry name `SYNOLOGY_2`) come from
  public documentation, not from observation.
- That Synology's boot loader accepts our GRUB-syntax entries (`search`,
  `linux`, `initrd` on a path under `/synofurb`) and boots them is **assumed**.
- That the loader tolerates the block markers/comments, a `\t` indented body,
  and a quote style different from its own is assumed.
- `install-efi` / `chainloader` entries are **entirely unverified** and only
  meaningful in UEFI mode.
- On the tests' VM kernel there is **no `vfat` mount support**, so every FAT
  *edit* in the tests went through the `mtools` fallback. The kernel
  `mount -t vfat` code path (the normal one on DSM/Debian) was exercised only
  **read-only** (`status`, `verify`) through a FUSE stand-in (`fusefat`); its
  *write* path (`cat new.conf > .../SynoBootLoader.conf` on the mounted copy,
  `mkdir`, `rm`) has never run here. The post-edit fsck and verification would
  catch a bad result and refuse to write, but you cannot rely on that path
  working until it has been run on a machine with a real vfat driver.
  (Via `mount` the conf is overwritten in place; via `mtools` the directory entry
  is recreated.)
- The cache flush before the read-back uses `blockdev --flushbufs`; whether the
  USB flash returns the data from the medium rather than a controller cache is
  outside the tool's control.
- Loop-mounting the DOM copies on DSM requires the DSM kernel's loop driver and
  `ext2`/`vfat` support; if a module is missing the tool stops (nothing written).
- DSM's own busybox/`e2fsck`/`dd` behaviour (flag support, `conv=fsync`) was
  not checked; busybox 1.36 was.
- Writing through `/dev/synoboot1` vs. the whole device with offsets: both paths
  are implemented; only the offset path (loop device without partition nodes)
  and file-backed images were exercised.
- DSM updates may rewrite the DOM (boot loader, `zImage`, `rd.gz`, possibly
  `SynoBootLoader.conf`) and so remove or invalidate our entries; re-run
  `status`/`verify` after every DSM update. Whether DSM validates DOM contents
  at boot or during updates is unknown.

## Limits

- Only units whose DOM matches the magics above (and whose partitions are
  1 and 2 of an MBR or GPT disk with 512-byte sectors, each below 1 GiB). Other
  layouts are refused, never guessed.
- The entry `SYNOLOGY_2` must exist in the menu; `default` indexes are counted
  from 0 over all `menuentry` lines (no `submenu` handling).
- Entry bodies of the DSM entries are treated as opaque and never rewritten.
- Needs roughly 2x the DOM size in temporary space (about 270 MiB).
- Text of `--cmdline` is restricted (see Environment); there is no way to embed
  newlines or `;`.
- The tool is not a security boundary: it trusts root and does not authenticate
  the kernel/initrd files you give it.

## License

MIT, as the rest of synofurb.
