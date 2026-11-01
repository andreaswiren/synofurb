# synofurb-mgmt

Small management plane that the synofurb installer drops onto a freshly installed Debian 13 NAS.
One Python file (standard library only, pure Python, runs on x86_64/arm64/armhf), HTTPS on port **8443**,
runs as root under systemd (`synofurb-mgmt.service`).

Needs these Debian packages for the features that use them: `openssl` (first-start certificate),
`iproute2`, `util-linux`, `mdadm`, `lvm2`, `xfsprogs`/`e2fsprogs`/`btrfs-progs`, `ntfs-3g`, `exfatprogs`,
`smartmontools` (optional), `openssh-server` (optional).

Version: the `VERSION` file next to this README (currently 0.2.0) is the single source; it is packaged with the script, read at start-up,
shown in the UI (login and sidebar footer) and returned by `GET /api/me`, `/api/status`, `/api/services` and `/api/tls-fingerprint`.

## What it does

| Page | Features |
| --- | --- |
| Dashboard | tiles for health, storage, memory, load, CPU vendor (Intel/AMD), kernel (flagged when >= 7), microcode package state, network; RAID/LVM/mount tables; disks with role, SMART fetched lazily; unit model (from `/proc/sys/kernel/syno_hw_version` or `/var/lib/synofurb/dsm-inspect.txt`); serial console (`console=` on the kernel command line) |
| Storage | free-device detection, 4-step create-volume wizard (devices, RAID layout, volume, review) with a live plan preview (exact commands, nothing executed), typed-name confirm to execute; extend LV + grow fs; delete |
| Imported volumes | "Scan RAID and LVM" (mdadm assemble + vgchange), mount unmounted filesystems (xfs/ext4/btrfs/vfat/exfat/ntfs) **read-only by default for foreign volumes**, optional fstab persistence, unmount, fstab view, signatures that need activation or unlocking |
| Network | per-interface DHCP (v4+v6+SLAAC) or static config written to `/etc/systemd/network/10-synofurb-<if>.network`; safe apply with a 90 s automatic rollback and a countdown banner on every page; hostname |
| Users | list local accounts (uid>=1000 + root marker), create/delete, set password, lock/unlock, add SSH key |
| Boot manager | units that boot through the internal boot flash (`/usr/local/sbin/synofurb-dom`): entries (DSM, Debian, previous kernel, boot agent), current default, DSM fail-safe note, **Return to DSM**, set default, reboot into an entry, boot-flash free space and DOM backup location. Other units get a "not a boot-flash unit" notice |
| System | sshd state and host key fingerprints, serial-getty units, restart of an allow-list of services, reboot/poweroff, version and unit info |

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
| POST | `/api/volumes/plan` | `{name,devices[],level,fs,mountpoint?,size_gib?}` -> steps with exact commands, runs nothing |
| POST | `/api/volumes/create` | same body + `confirm` = volume name; executes the re-derived plan |
| POST | `/api/volumes/extend` | `{vg,lv,add_gib?}` (empty = all free space) |
| POST | `/api/volumes/delete` | `{vg,lv,confirm=lv,remove_vg?}` |
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
