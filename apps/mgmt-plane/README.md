# synofurb-mgmt

Small management plane that the synofurb installer drops onto a freshly installed Debian 13 NAS.
One Python file (standard library only, pure Python, runs on x86_64/arm64/armhf), HTTPS on port **8443**,
runs as root under systemd (`synofurb-mgmt.service`).

Needs these Debian packages for the features that use them: `openssl` (first-start certificate),
`iproute2`, `util-linux`, `mdadm`, `lvm2`, `xfsprogs`/`e2fsprogs`/`btrfs-progs`, `ntfs-3g`, `exfatprogs`,
`smartmontools` (optional), `openssh-server` (optional).

Version: the `VERSION` file next to this README (currently 0.6.0) is the single source; it is packaged with the script, read at start-up,
shown in the UI (login and sidebar footer) and returned by `GET /api/me`, `/api/status`, `/api/services` and `/api/tls-fingerprint`.

## What it does

| Page | Features |
| --- | --- |
| Dashboard | tiles for health, storage, memory, load, CPU vendor (Intel/AMD), kernel (flagged when >= 7), microcode package state, network; RAID/LVM/mount tables; disks with role, SMART fetched lazily; unit model (from `/proc/sys/kernel/syno_hw_version` or `/var/lib/synofurb/dsm-inspect.txt`); serial console (`console=` on the kernel command line) |
| RAID | array inventory (level, state, members with slot/model/serial, spares, bitmap, resync/reshape/check progress and speed, mismatch count, what uses it), create (RAID 0/1/5/6/10 and **Hybrid / SHR-style 1 and 2**), add spare, fail/remove, replace (failed or healthy with `--replace --with`), grow, change level, check/repair/stop task, stop/assemble, tuning, monthly check timer, SHR add/replace disk; every change is plan, typed confirmation, commands, audit |
| Storage | free-device detection, 4-step create-volume wizard (devices, RAID layout, volume, review) with a live plan preview (exact commands, nothing executed), typed-name confirm to execute; extend LV + grow fs; delete |
| Mounts | tabs **Mounted** and **Imported**: "Scan disks" (mdadm assemble + vgchange), mount unmounted filesystems (xfs/ext4/btrfs/vfat/exfat/ntfs) **read-only by default for foreign volumes**, optional fstab persistence, unmount, fstab view, signatures that need activation or unlocking |
| Network | per-interface DHCP (v4+v6+SLAAC) or static config written to `/etc/systemd/network/10-synofurb-<if>.network`; safe apply with a 90 s automatic rollback and a countdown banner on every page; hostname |
| Users | list local accounts (uid>=1000 + root marker), create/delete, set password, lock/unlock, add SSH key |
| Boot manager | units that boot through the internal boot flash (`/usr/local/sbin/synofurb-dom`): entries (DSM, Debian, previous kernel, boot agent), current default, DSM fail-safe note, **Return to DSM**, set default, reboot into an entry, boot-flash free space and DOM backup location. Other units get a "not a boot-flash unit" notice |
| System | sshd state and host key fingerprints, serial-getty units, restart of an allow-list of services, reboot/poweroff |
| Add-ons | Docker apps as small definitions (`addons/<id>/addon.json` + compose template): catalog cards, install dialog with settings, **privileges, devices and storage shown before install**, jobs with progress, logs, update, remove (delete data needs the typed name); built in: Docker, Portainer, Frigate NVR, Plex; contract in [docs/ADDONS.md](../../docs/ADDONS.md) |
| Certificates | Let's Encrypt certificates through `lego`: DNS-01 via challenges.addr.tools (one CNAME, a secret that is hashed with SHA-224), HTTP-01, TLS-ALPN-01; several certificates and wildcards, staging switch, rate-limit messages, Check DNS and a port pre-check, jobs with progress, **automatic renewal** (systemd timer), expiring or failed certificates in `health --json`; [docs/CERTIFICATES.md](../../docs/CERTIFICATES.md) |
| Hostnames | one **HAProxy front** (default off) for the management plane and every add-on web page: hostname and certificate per target, `haproxy -c` before every reload, snapshot and a **revert timer unless Keep is pressed**, recovery with `proxy disable` from the console; [docs/CERTIFICATES.md](../../docs/CERTIFICATES.md) |
| Updates | installed and available version, channel **Stable** (newest release asset) or **Main** (`apps/mgmt-plane` of a branch or tag), Check, release notes, **Update now** (consent while unsigned, typed commit for Main), live progress that survives the restart, automatic rollback, **Roll back**, History of every update (desktop app, command line, GitHub, who, result); see "Self-update" below |
| Help | twelve-plus short topics with search (sign-in, certificate, storage, imports, boot, recovery, report, RAID levels, Hybrid RAID, replace, scrub, safe apply); every (i) button in the UI opens the matching topic |
| About | logo, version, **Copy diagnostics** (version, host, OS, kernel, service state; no secrets), links to documentation, source and issue tracker, Credits with licence texts (Debian, Lucide ISC, Inter and JetBrains Mono OFL 1.1, Montserrat, synodeb note) |

## Add-ons (Docker apps)

`synofurb_addons.py` (standard library only, loaded by `synofurb_mgmt.py`) is the plugin framework; the definitions are in `addons/<id>/` (`addon.json` manifest, `compose.yml.tmpl`, optional `*.tmpl` files) and in
`/etc/synofurb/addons.d/<id>/` for your own. Shipped: `docker` (Debian `docker.io`, `docker-cli`, `docker-compose`; idempotent; adds the administrators to the `docker` group), `portainer`, `frigate`, `plex`.
Everything goes through `docker compose -p synofurb-<id>`; installs, updates and removals are background jobs (one at a time, shared with management-plane updates) written to the audit log.
Update bundles carry the framework as `synofurb_addons.py` plus `addons.pack.json` (the definitions as one file; `tools/make-update-bundle.py` builds it).

```
synofurb_mgmt.py addons list|show ID|status ID|logs ID [--tail N]
synofurb_mgmt.py addons install ID [--set key=value ...] [--storage key=/srv/data/...] [--device id=on|off] [--settings-stdin] [--yes]
synofurb_mgmt.py addons update|start|stop|restart ID          synofurb_mgmt.py addons remove ID [--delete-data --yes]
synofurb_mgmt.py addons apply-pending                          # first boot: installs what the installer left in /etc/synofurb/addons.d/pending
```

Secrets (a Plex claim token, camera URLs with passwords) are never accepted on the command line: pass them as JSON on standard input with `--settings-stdin`. API and details: [docs/ADDONS.md](../../docs/ADDONS.md).
Nothing of it has run against a real Docker daemon yet; the test suite uses a fake `docker`.

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

## Certificates and the HAProxy front

`synofurb_edge.py` (standard library only, loaded by `synofurb_mgmt.py`; an update bundle without it simply has neither page) issues, stores and renews certificates and manages one HAProxy in front of the management plane and the add-ons.
Everything goes through `lego` (Debian `lego` 4.9.1, installed on demand with apt) and `haproxy` as argv lists; the secret of the challenge service (challenges.addr.tools) lives in one 0600 file and never in an argv, a log or an API answer.
**Nothing of it has run against a real Let's Encrypt, DNS, lego, HAProxy or systemd yet**; the tests use fake executables and loopback servers. Design, files, flags to verify on the unit, recovery path and the full list of what is unverified: [docs/CERTIFICATES.md](../../docs/CERTIFICATES.md).

```
synofurb_mgmt.py certs list|show ID|issue NAME... [--challenge dns-01|http-01|tls-alpn-01] [--environment staging|production]|change ID NAME...|renew [--id ID] [--force]|remove ID --confirm ID|check-dns NAME...
synofurb_mgmt.py certs settings [--email E] [--accept-tos] [--generate-secret|--secret-stdin] [--check-secret] [--environment E] [--key-type T]
synofurb_mgmt.py proxy status|enable|apply|disable|test|confirm|revert|assign TARGET HOSTNAME CERT|unassign TARGET      # disable is the way back from a console or SSH
```

With the front on, the web UI is at `https://<ip>/` (or its hostname), the management plane listens on `127.0.0.1:8443` only, and `/etc/synofurb/mgmt.json` has `bind` and `trust_proxy` set by the front (`trust_proxy` makes it believe `X-Forwarded-For` from a loopback peer, so login throttling and the audit log see the real client).
Update bundles carry the module as `synofurb_edge.py`.

## Command-line contract (for the desktop app and SSH)

No server is started; run as root over SSH (`health` works without root). Exit code 0 unless stated.

```sh
python3 /opt/synofurb-mgmt/synofurb_mgmt.py health --json
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --from /tmp/bundle --json      # --force allows a downgrade
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --rollback --json
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --from-github --check --json      # newest release; add --ref main|<tag> for the source folder
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --from-github --accept-unsigned [--ref R] [--force] --json
python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --status --json                   # installed version, previous version, history
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

### Self-update (web UI, `--from-github`, the app's push: one pipeline)

`update --from-github` adds *check -> download -> verify* (and the steps `lock`, `check`, `download`, `consent`) in front of the same `Updater`, so the web UI, the shell and the desktop
app behave identically (same files, same atomic swap, same 30 s health wait, same automatic rollback, same exit codes). Details, trust model and the network policy: `docs/MANAGE.md`
("Updating the management plane itself"). In short:

* Sources: **stable** = the newest release asset `synofurb-mgmt-<ver>.tar.gz` + `SHA256SUMS` (built by `scripts/package-mgmt.sh`); **main** = `--ref <branch|tag>`, the `apps/mgmt-plane` folder of the monorepo, pinned to the commit the check showed.
* https only, to GitHub's hosts (every redirect hop checked), hard size limits, extraction by allow-listed file names, no token or credential sent. Releases are unsigned today, so `--accept-unsigned` (the web UI's consent box)
  is required; a real minisign key (`update_pubkey` in `/etc/synofurb/mgmt.json`) makes signatures mandatory.
* Refused (exit 2, nothing changed) while the unit is `crit`, the service is down or another update/add-on job holds `/var/lib/synofurb-mgmt/job.lock`, unless `--force` (health only).
* The web UI starts the job in its own `systemd-run` unit and polls it, so restarting `synofurb-mgmt.service` cannot kill it. Every update is audit-logged and kept as a job record in
  `/var/lib/synofurb-mgmt/update/jobs/` (the History). Add-on jobs take the same lock (`JobLock`).

## Look and feel

The UI follows the "ForgeVault family" direction from `design/mockups/ui` (Direction 1): left navigation and a 54 px top bar on
desktop, bottom tab bar plus slide-in menu below 860 px, tables that stack into cards below 600 px, 44 px touch targets on phones.
Colours are the exact tokens from `design/mockups/ui/tokens.json` (revised after the critic review: pressed states, placeholder, `*_border`, darker destructive set; `theme_tokens()`/`accent_tokens()` in the code, a test compares them with the JSON), five accents selectable in the top bar (web plane only),
as CSS custom properties: light by default, dark via `prefers-color-scheme`, and a toggle in the top bar (stored in `localStorage`).
No `color-mix()`, gradients or inset shadows (tested). Type floor 12 px, weights 400/600/700, sentence case. Status is never colour alone: every badge and banner has an icon and a word, "off" states use a dashed border, attention rows get a 3 px edge.
**Fonts**: Inter (400/600/700) and JetBrains Mono (400), subset to Latin and embedded as woff2 `data:` URIs (about 62 KB of base64 in total, `font-src data:` in the CSP); the stacks keep the system fonts as fallback, so nothing breaks if a font fails to decode.
**Logo**: `assets/brand` mark and wordmark (light and dark variants follow the theme) in the sidebar, the login card and About; `favicon.svg` is an inline `data:` favicon.
**Icons**: Lucide only (ISC), a fixed list of 56 symbols inlined as an SVG sprite; a test checks that every icon the UI asks for exists and equals the Lucide source. No external fonts, scripts, images or CDNs. `prefers-reduced-motion` and
`forced-colors` are honoured. Destructive actions use the typed-name confirm dialog (button stays disabled until the text matches).
**60-word rule** (`design/v2/UX-SPEC.md`): every page shows at most 60 words by default, shell included (headings, labels, column headers, buttons, prose; values that come from the system are data). Explanations live in tooltips (`tip`), popovers (native `popover`, Esc closes) and the Help topics; all text is in the `COPY` module with budgets per kind. `tests/test_ui.py` checks the copy budgets, forbids hard-coded prose in the script and, when node and Playwright are installed, opens every page of the demo (both units, desktop and phone) and counts the words really rendered.
**Manifest prose is counted too.** Add-on summaries (`data-d="prose"`) are not exempt from the audit: the browser audit counts them apart from the 60 words of the page chrome and caps them on their own, at most 20 words per summary (`tests/test_addons_catalog.py`) and at most 60 words of manifest prose on one page (`MANIFEST_BUDGET` in `tests/test_ui.py`, measured with nothing installed, which is the worst case: four cards).
**A control that is off says why in text.** A disabled button swallows hover, focus and touch, so a tooltip on it is never seen. Controls that are off for a reason (Update now, Install without Docker, Install before the privileges are accepted, Remove before the name is typed, Roll back while a job runs) use `aria-disabled` plus a visible line of text the control points to with `aria-describedby` (`setOff()` and `whyLine()` in the script): the button stays focusable, clicks are ignored, keyboard and touch users read the reason. The browser tests fail on a native `disabled` control without a reason and on a reason that is hidden or outside the viewport (`tests/ui_lib.js`).
**No stray text.** The same library fails any page state that shows the words `null`, `undefined`, `NaN`, `[object ...]` or an unresolved `{placeholder}` (it caught the "null" under the Health step of the update card); `tests/test_ui.py` also proves that the check detects them.

**Screenshots of the new pages** (`design/screenshots/mgmt-demo-<state>-<light|dark>[-phone].png`, 1280 and 390 wide; the demo shows Add-ons states with `?addons=states|problems|nodocker`, Updates states with `?update=available|current|offline|rollback|busy|unhealthy`, Certificates states with `?certs=states|unset|empty` and Hostnames states with `?proxy=on|pending`):

```sh
python3 apps/mgmt-plane/tools/build-demo.py                       # rebuild apps/mgmt-plane/demo/index.html
NODE_PATH="$(npm root -g)" node apps/mgmt-plane/tools/shots.js apps/mgmt-plane/demo/index.html design/screenshots   # all states (about 4 minutes, ~100 files)
... shots.js demo/index.html OUT --only addons-nodocker,updates-current   # some states      ... shots.js demo/index.html OUT --list   # the names
```

Needs node, playwright and a chromium build. Every state is checked with `tests/ui_lib.js` while it is captured (the exit code is 1 on stray text, a reason only a mouse can see, or overflow) and each PNG must stay below 120 KB.

Regenerate the embedded fonts, logos, favicon, icons and licence texts with `python3 tools/build-assets.py` (needs `pip install fonttools brotli`, `assets/brand` and a Lucide icon directory, `LUCIDE_DIR=...`).

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
| GET | `/api/update` | installed and previous version, channel/ref, the last check, the active job, the history (last 15 jobs) |
| POST | `/api/update/check` | `{channel: stable\|main, ref?}` -> asks GitHub; returns the check (`version`, `state`, `mode`, `blocked`, `notes`, `commit`) and remembers it |
| POST | `/api/update/start` | `{channel, ref?, expect, accept_unsigned?, force?, confirm?}`: `expect` = the version (stable) or commit (main) that was shown; main needs `confirm` = its first 7 characters; returns `{job}`, 409 while another job runs |
| POST | `/api/update/rollback` | `{}` -> job that restores `/opt/synofurb-mgmt.prev` |
| POST | `/api/update/job` | `{job}` -> the job and its step events |
| POST | `/api/update/progress` | public: `{job}` -> only `state`, `step`, `kind`, `from`, `to`, `finished` (what the page can read after the restart dropped its session) |
| GET | `/api/addons` | catalog and installed add-ons with live state, `problems[]`, the running `job` |
| GET | `/api/addons/<id>` | one add-on: settings (secrets as `{set}`), storage report, devices, privileges |
| POST | `/api/addons/<id>/install` | `{settings, storage, devices, accept}` -> **202** `{job}`; privileges must be accepted |
| POST | `/api/addons/<id>/update`, `/start`, `/stop`, `/restart` | -> **202** `{job}` |
| POST | `/api/addons/<id>/remove` | `{delete_data, confirm}` (`confirm` = the id when data is deleted) -> **202** `{job}` |
| GET | `/api/addons/<id>/logs?tail=N` | container (or journal) log, secret values masked |
| GET | `/api/jobs`, `/api/jobs/<id>` | add-on jobs: state, step, progress, log; 409 on a second job
| GET | `/api/certs`, `/api/certs/<id>`, `/api/certs/job/<id>` | certificates with state, expiry, CNAME targets, hostnames using them; settings (the secret only as `{set, target}`); jobs |
| POST | `/api/certs/settings`, `/issue`, `/<id>/renew`, `/<id>/change`, `/<id>/remove` | settings (a created secret is returned once as `secret_once`), issue / renew / change -> **202** `{job}`, remove `{confirm: id}` |
| POST | `/api/certs/precheck`, `/check-dns`, `/check-secret` | what can be known without asking Let's Encrypt: names, ports, the CNAME, the service |
| GET | `/api/proxy` | the front: on/off, rows (management plane and add-ons with hostname, certificate, backend scheme), HAProxy state, the pending change, addresses, the recovery command |
| POST | `/api/proxy/hostnames`, `/test`, `/enable`, `/apply`, `/disable`, `/confirm`, `/revert` | save while off, `haproxy -c` on a candidate, jobs (**202**), Keep, go back; `/api/me` has `edge_pending` for the banner |

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

Configuration (all optional) in `/etc/synofurb/mgmt.json`: `port`, `bind`, `idle_timeout`, `abs_timeout`, `rollback_seconds`, `login_group`, `trust_proxy` (set by the HAProxy front: see above; believe `X-Forwarded-For` only from a loopback peer).
* **Certificates and the HAProxy front** (`synofurb_edge.py`): no shell, every name through a strict validator (punycode, no path or control characters) before it reaches an argv, a file name or the HAProxy configuration; the challenge secret in one 0600 file, scrubbed from job logs, never in the audit log; every HAProxy change is checked with `haproxy -c` first and has an automatic way back; the front's loopback PROXY port (127.0.0.1:8444) can be reached by local users. Details: [docs/CERTIFICATES.md](../../docs/CERTIFICATES.md).

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
