# synofurb-mgmt

Small management plane that the synofurb installer drops onto a freshly installed Debian 13 NAS.
One Python file (standard library only, pure Python, runs on x86_64/arm64/armhf), HTTPS on port **8443**,
runs as root under systemd (`synofurb-mgmt.service`).

Needs these Debian packages for the features that use them: `openssl` (first-start certificate),
`iproute2`, `util-linux`, `mdadm`, `lvm2`, `xfsprogs`/`e2fsprogs`/`btrfs-progs`, `ntfs-3g`, `exfatprogs`,
`smartmontools` (optional), `openssh-server` (optional).

Version: the `VERSION` file next to this README (currently 0.4.0) is the single source; it is packaged with the script, read at start-up,
shown in the UI (login and sidebar footer) and returned by `GET /api/me`, `/api/status`, `/api/services` and `/api/tls-fingerprint`.

## What it does

| Page | Features |
| --- | --- |
| Dashboard | tiles for health, storage, memory, load, CPU vendor (Intel/AMD), kernel (flagged when >= 7), microcode package state, network; RAID/LVM/mount tables; disks with role, SMART fetched lazily; unit model (from `/proc/sys/kernel/syno_hw_version` or `/var/lib/synofurb/dsm-inspect.txt`); serial console (`console=` on the kernel command line) |
| RAID | array inventory (level, state, members with slot/model/serial, spares, bitmap, resync/reshape/check progress and speed, mismatch count, what uses it), create (RAID 0/1/5/6/10 and **Hybrid / SHR-style 1 and 2**), add spare, fail/remove, replace (failed or healthy with `--replace --with`), grow, change level, check/repair/stop task, stop/assemble, tuning, monthly check timer, SHR add/replace disk; every change is plan, typed confirmation, commands, audit |
| Storage | free-device detection, 4-step create-volume wizard (devices, RAID layout, volume, review) with a live plan preview (exact commands, nothing executed), typed-name confirm to execute; extend LV + grow fs; delete |
| Imported volumes | "Scan RAID and LVM" (mdadm assemble + vgchange), mount unmounted filesystems (xfs/ext4/btrfs/vfat/exfat/ntfs) **read-only by default for foreign volumes**, optional fstab persistence, unmount, fstab view, signatures that need activation or unlocking |
| Network | per-interface DHCP (v4+v6+SLAAC) or static config written to `/etc/systemd/network/10-synofurb-<if>.network`; safe apply with a 90 s automatic rollback and a countdown banner on every page; hostname |
| Users | list local accounts (uid>=1000 + root marker), create/delete, set password, lock/unlock, add SSH key |
| Boot manager | units that boot through the internal boot flash (`/usr/local/sbin/synofurb-dom`): entries (DSM, Debian, previous kernel, boot agent), current default, DSM fail-safe note, **Return to DSM**, set default, reboot into an entry, boot-flash free space and DOM backup location. Other units get a "not a boot-flash unit" notice |
| System | sshd state and host key fingerprints, serial-getty units, restart of an allow-list of services, reboot/poweroff, version and unit info |

## RAID and Hybrid RAID (SHR)

The RAID page implements `docs/RAID.md`. Hybrid RAID is mdadm plus LVM: the data space of the disks is cut into slices at the disk sizes
(`shr_slices()`, vectors in `tests/shr_vectors.json`, the browser and the Rust planner use the same algorithm), every slice with two or more
members becomes an md array (`<volume>_<n>`), all arrays are physical volumes of `vg_<volume>` and the volume is one linear logical volume.
Volumes built here are SHR-style and Debian-managed; DSM does not adopt them. Imported DSM arrays are shown as *Imported*: assemble and check
are free, anything else needs the extra "I understand this is not mine" acknowledgement and the typed array name.

Safety rules (enforced by the server, not only the UI): devices must be free (not mounted, not a member, not holding `/`, `/boot` or swap);
no change while an array is resyncing, reshaping or checking (stopping the task and tuning are allowed); a disk cannot be failed if that would
lose data; an array in use by LVM or a mount cannot be stopped; only conversions mdadm supports (RAID1 to 5, 5 to 6, 6 to 5, 0 to 5);
the plan is re-derived at execution time; the typed phrase is the array (or volume) name. After a grow the follow-up steps (`pvresize`,
`lvextend`, filesystem grow) are offered and run through `POST /api/raid/after-grow` once the array is idle.
The monthly check is a pair of units the service writes on request (`synofurb-raid-scrub.timer`, runs `synofurb_mgmt.py --raid-scrub`).

## Command-line contract (for the desktop app and SSH)

No server is started; run as root over SSH (`health` works without root). Exit code 0 unless stated.

```sh
python3 /opt/synofurb-mgmt/synofurb_mgmt.py health --json
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --from /tmp/bundle --json      # --force allows a downgrade
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --rollback --json
```

`health --json` prints one JSON object (`schema` 1): `version`, `hostname`, `uptime_s`, `load[3]`, `mem{total,avail}`, `os`, `kernel`,
`root_fs{used_pct}`, `arrays[{name,level,state,degraded,sync_pct,sync_action}]`, `disks[{dev,model,size,smart: passed|failed|unknown,temp_c}]`,
`volumes[{mount,used_pct,fstype}]`, `services[{name,active}]`, `boot{default,flash_present}`, `updates{apt_pending,security_pending}`,
`overall` (`ok|warn|crit`) and short `warnings`. A part that cannot be read is `null` plus a warning, commands have timeouts and the whole
collection has a time budget, so it never hangs and never fails. Degraded arrays, failed SMART, 95 % full volumes and 70 C disks are `crit`;
syncing arrays, stopped services, 85 % full volumes and pending security updates are `warn`. (`--quick` skips SMART and apt.)

`update --from DIR`: DIR holds `synofurb_mgmt.py`, `VERSION`, `README.md`, optionally `synofurb-mgmt.service`, and `MANIFEST.sha256`
(`sha256sum` format, every file listed, nothing else in the directory). Steps (one NDJSON line each with `--json`: `{"step","status","message"}`,
status `start|ok|fail`, final line `step: "result"`): `verify` (manifest, files are regular, VERSION is not older unless `--force`, the script compiles; it is never executed
before this passes), `stage` (copy to `/opt/synofurb-mgmt.new`, re-hash), `swap` (atomic exchange, previous install kept as `/opt/synofurb-mgmt.prev`,
unit file replaced if packaged), `restart`, `health` (up to 30 s: service active, port listening, the new code answers `health --json` with its version),
and on any failure `rollback` (previous install and unit file back, restart, same health check). No network access. Exit codes: 0 updated, 2 refused
(nothing changed), 3 failed and rolled back, 4 failed and the rollback failed. `update --rollback` exchanges the installed and the `.prev` version (running it twice rolls forward).
Build a bundle with `python3 tools/make-update-bundle.py OUT_DIR`.

## Look and feel

The UI follows the "ForgeVault family" direction from `design/mockups/ui` (Direction 1): left navigation and a 54 px top bar on
desktop, bottom tab bar plus slide-in menu below 860 px, tables that stack into cards below 600 px, 44 px touch targets on phones.
Colours are the exact tokens from `design/mockups/ui/tokens.json` (revised after the critic review: pressed states, placeholder, `*_border`, darker destructive set; `theme_tokens()`/`accent_tokens()` in the code, a test compares them with the JSON), five accents selectable in the top bar (web plane only),
as CSS custom properties: light by default, dark via `prefers-color-scheme`, and a toggle in the top bar (stored in `localStorage`).
No `color-mix()`, gradients or inset shadows (tested). Type floor 12 px, weights 400/600/700, sentence case. Status is never colour alone: every badge and banner has an icon and a word, "off" states use a dashed border, attention rows get a 3 px edge.
Icons are the inline lucide-style set of the mockups; no external fonts, scripts, images or CDNs. `prefers-reduced-motion` and
`forced-colors` are honoured. Destructive actions use the typed-name confirm dialog (button stays disabled until the text matches).

## API

All `/api/*` responses are JSON `{"ok": true|false, ...}` (`error` on failure). Everything except the three
public routes needs the session cookie; every POST also needs `X-CSRF-Token` and `Content-Type: application/json`.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/` | the single-page UI (public, no data in it) |
| GET | `/api/tls-fingerprint` | SHA-256 of the server certificate, hostname, version (public) |
| POST | `/api/login` | `{username,password}` -> sets cookie, returns `csrf` (public) |
| POST | `/api/logout` | end session |
| GET | `/api/me` | user, csrf token, hostname, **version**, pending network change |
| GET | `/api/status` | system overview: host, OS, kernel (`kernel_info.ge7`), CPU vendor/model, microcode, console, unit model, memory, disks, md, LVM, mounts, interfaces |
| GET | `/api/smart` | SMART health per disk (only if `smartctl` exists) |
| GET | `/api/users` | list users and known groups |
| POST | `/api/users/create` | `{username,password,groups?,sudo?,ssh_key?}` |
| POST | `/api/users/delete` | `{username,confirm=username,remove_home?}` |
| POST | `/api/users/password` | `{username,password}` |
| POST | `/api/users/lock`, `/api/users/unlock` | `{username}` |
| POST | `/api/users/ssh-key` | `{username,ssh_key}` |
| GET | `/api/network` | interfaces, their managed config, pending change |
| POST | `/api/network/apply` | `{ifname,mode:dhcp\|static,ipv4?,gateway4?,ipv6?,gateway6?,dns?,search?}` |
| POST | `/api/network/confirm` | stop the rollback timer (keep the new config) |
| POST | `/api/network/rollback` | roll back right now |
| POST | `/api/hostname` | `{hostname}` |
| GET | `/api/volumes` | VGs, LVs, PVs, free candidate devices (+ why others are excluded), md arrays |
| POST | `/api/volumes/plan` | `{name,devices[],level,fs,mountpoint?,size_gib?}` -> steps with exact commands, runs nothing; `level` may be `shr1`/`shr2` (whole disks) |
| POST | `/api/volumes/create` | same body + `confirm` = volume name; executes the re-derived plan |
| POST | `/api/volumes/extend` | `{vg,lv,add_gib?}` (empty = all free space) |
| POST | `/api/volumes/delete` | `{vg,lv,confirm=lv,remove_vg?}` |
| GET | `/api/raid` | arrays, free disks, SHR volumes, speed limits, check timer |
| POST | `/api/raid/plan` | `{op, ...}` for any operation below: returns title, warnings, exact commands, follow-ups, `confirm` phrase; runs nothing |
| POST | `/api/raid/<op>` | `create`, `add-spare`, `fail`, `remove`, `replace`, `grow`, `change-level`, `scrub`, `stop`, `assemble`, `tuning`, `shr-create`, `shr-add-disk`, `shr-replace-disk`: same body as the plan plus `confirm` (and `foreign_ack` for imported arrays) |
| POST | `/api/raid/after-grow` | `{array, confirm}`: pvresize + lvextend + filesystem grow when the array is idle |
| GET | `/api/mounts` | mounts, fstab entries, mountable unmounted filesystems |
| POST | `/api/mounts/mount` | `{device,mountpoint,ro?,persist?,owner?}`; `ro` defaults to true for imported (non-`vg_*`) volumes |
| POST | `/api/mounts/unmount` | `{mountpoint,confirm=mountpoint,remove_fstab?}` |
| POST | `/api/mounts/activate` | mdadm assemble + vgscan + vgchange -ay, then list devices |
| GET | `/api/services` | sshd state, host key fingerprints, serial-getty units |
| POST | `/api/services/restart` | `{unit}` from an allow-list (ssh, systemd-resolved, systemd-timesyncd, cron) |
| POST | `/api/system/power` | `{action:reboot\|poweroff,confirm=<hostname>}` |
| GET | `/api/boot` | boot-flash status via `synofurb-dom status --json`, or `{available:false}` on units without the tool |
| POST | `/api/boot/default` | `{entry:dsm\|debian\|debian-old\|agent,confirm=<entry>}` runs `synofurb-dom default <entry> --yes`; `confirm` is not needed for `debian` |
| POST | `/api/boot/reboot-to` | same body; sets the default, then `systemd-run --on-active=5 systemctl reboot` |

## Security model

* **Login**: local users only, checked against `/etc/shadow` with libcrypt `crypt()` via ctypes (yescrypt
  `$y$`, `$6$`, ...; calls serialised by a lock), constant-time compare. Only members of group `sudo` with a usable,
  unlocked password may log in; uid 0 may not. `!`/`*`/empty hashes never verify. Unknown users cost the same time.
* **Sessions**: random 256-bit id, server-side only (hash stored), idle timeout 30 min, absolute 12 h, max 64.
  Cookie `__Host-synofurb; Path=/; Secure; HttpOnly; SameSite=Strict`. Sessions live in memory (restart = log in again).
* **CSRF**: `X-CSRF-Token` required on every authenticated non-GET, `Origin` must equal `Host` when present,
  JSON content type required.
* **Brute force**: per-IP and per-user lockout after 5 failures for 5 min, doubling per repeat up to 1 h; failures are audited.
  (A user-name lockout can be triggered by an attacker; accepted trade-off for a LAN appliance.)
* **Headers**: per-response CSP nonce for the single inline script/style, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`,
  `Cache-Control: no-store`, `nosniff`, HSTS. No external resources.
* **TLS**: self-signed EC P-256 cert generated on first start in `/var/lib/synofurb-mgmt/tls/` (key 0600); TLS >= 1.2.
  The SHA-256 fingerprint is logged at start, saved to `tls/fingerprint.txt` and served at `/api/tls-fingerprint`.
  The cert is not regenerated when the IP changes: delete the `tls` directory and restart.
* **Commands**: argv lists only, no shell, fixed `PATH=/usr/sbin:/usr/bin:/sbin:/bin`, timeout on each command,
  every interpolated value passes a strict validator, block devices are additionally matched against live `lsblk`.
  Passwords go to `chpasswd` on stdin. Request bodies are capped at 64 KiB.
* **Destructive actions** need `confirm` equal to the target name. Create-volume refuses devices that are mounted, RAID/LVM members,
  formatted, hold `/`, `/boot*`, swap, `/usr`, `/var`, or live on a system disk. Mountpoints are confined to `/srv/*` and `/mnt/*`
  (no `..`, no symlink components).
* **Audit**: `/var/log/synofurb-mgmt/audit.log`, JSON lines (time, user, ip, action, args with secrets masked, result), mode 0600.
* **Network safe apply**: previous file backed up, rollback timer (`systemd-run --on-active=90 --unit=synofurb-net-rollback`) armed *before*
  the link is reconfigured, cancelled only by `POST /api/network/confirm`. An unconfirmed change surviving a reboot is rolled back at startup.
  Older-sorting `.network` files that would shadow ours for that interface are renamed to `*.disabled-by-synofurb` (restored on rollback).

Configuration (all optional) in `/etc/synofurb/mgmt.json`: `port`, `bind`, `idle_timeout`, `abs_timeout`, `rollback_seconds`, `login_group`.

## Development without root (dry run)

```sh
cd apps/mgmt-plane
python3 synofurb_mgmt.py --dry-run --no-tls --port 8080          # sandbox root in a temp dir; login admin / admin
python3 synofurb_mgmt.py --dry-run --root /tmp/fv --port 8443    # with TLS, files under /tmp/fv
# open http://127.0.0.1:8080/
```

`--dry-run` records every mutating command (printed as `[dry-run] ...`) and executes none; read-only inspection
(`lsblk`, `ip`, `findmnt`, `vgs`, ...) really runs so the UI shows real data. File writes (fstab, networkd, mdadm.conf, authorized_keys,
state, audit log) land under `--root` (a temp dir if omitted). `--no-tls` is refused without `--dry-run`.

Tests (no root, no mdadm/lvm/networkd needed):

```sh
python3 -m unittest discover -s tests -v
```

## Install

```sh
install -d -m 0755 /opt/synofurb-mgmt && install -m 0644 synofurb_mgmt.py /opt/synofurb-mgmt/
install -m 0644 synofurb-mgmt.service /etc/systemd/system/ && systemctl daemon-reload && systemctl enable --now synofurb-mgmt
journalctl -u synofurb-mgmt | grep fingerprint
```

## Known limitations

* A failed multi-step volume create is reported step by step but not rolled back automatically.
* Partitions that sit on the system disk are never offered as new volume members.
* Deleting a volume leaves its md array (and `mdadm.conf` line) in place; stop/remove arrays manually.
* IPv4 static gateways must be inside the interface subnet; no multi-address or VLAN/bridge editing.
