# Managing a converted unit: health and updates of the web UI

After the install the unit runs the **management plane** (`apps/mgmt-plane`, a web UI on `https://<ip>:8443`). The desktop app and
`synofurb-cli` can look after it from your computer, over SSH, without opening a browser:

* **Health**: is the unit well? Degraded arrays, failed disks, full volumes, services that are down, pending security updates.
* **Version**: which management plane version is on the unit, and is the one shipped with this program newer?
* **Update**: push the shipped management plane to the unit, restart the web UI, verify, and go back by itself if it does not come up.
* **Rollback**: return to the version the unit had before the last update.
* **Open**: the address of the web UI.

This is deliberately small. Everything else (users, shares, network, RAID) is done in the web UI itself. Nothing in this document touches
disks, partitions, arrays or the boot flash: the only files written are `/opt/synofurb-mgmt` (and its `.prev`/`.new` siblings), the
`synofurb-mgmt.service` unit file and temporary files below `/tmp` that are removed again.

Contents: [Commands](#commands) - [The contract with the unit](#the-contract-with-the-unit) - [What the verdict means](#what-the-verdict-means) -
[Update flow](#update-flow) - [Updating the management plane itself](#updating-the-management-plane-itself-web-ui-and-github) - [Offline copies](#offline-copies) - [Engine API](#engine-api-for-the-desktop-app) - [What is verified](#what-is-verified)

## Commands

All commands log in like `inspect` does (`--host`, `--user`, `--port`, `--key`; password from `$SYNOFURB_PASSWORD` or a file, never on the
command line). The unit's SSH host key is checked first: pinned on first use, a changed key is refused with exit 11 and nothing is sent (`--host-key`, `--known-hosts`,
`--accept-new-host-key`, `--insecure-host-key`, `host-key forget`: [CLI.md](CLI.md#ssh-host-keys)). If the SSH user is not root, `sudo` is used (the password goes through stdin, never argv). Full option list: [CLI.md](CLI.md#manage).

| Command | What it does | Exit |
|---|---|---|
| `manage health --host H [--json]` | health report and the short problem list | 0 ok, **8 warn, 9 crit**, 1 could not ask |
| `manage version --host H [--json]` | unit's version vs the shipped one; says whether an update is available | 0 |
| `manage update --host H [--from-bundle DIR] [--force] [--dry-run] [--yes]` | the update flow below | 0 done or already current, 1 failed, 3 declined / needs `--force` |
| `manage rollback --host H [--yes]` | `update --rollback` on the unit | 0, 1 |
| `manage open --host H` | prints `https://H:8443`; no connection | 0 |

`--log-format json` turns progress into NDJSON events (same `step`/`progress`/`log` vocabulary as `install`) and ends with a `result` event.

## The contract with the unit

The unit-side program is `/opt/synofurb-mgmt/synofurb_mgmt.py` (Python 3, standard library only). The engine uses three of its calls and
nothing else. The authoritative description is in [apps/mgmt-plane/README.md](../apps/mgmt-plane/README.md); this is what the engine relies on.

### 1. `python3 /opt/synofurb-mgmt/synofurb_mgmt.py health --json`

Prints one JSON object. Required: `schema` (1), `version` (the management plane's own version) and `overall` (`ok|warn|crit`). Everything
else is optional and parsed leniently (a field of the wrong type, a `null`, a missing section or an unknown field never fails the report; what
had to be skipped is listed in `parse_notes`):

```json
{"schema":1,"version":"0.4.0","hostname":"nas","uptime_s":273600,"load":[0.12,0.10,0.05],
 "mem":{"total":8000000000,"avail":6000000000},"os":"Debian GNU/Linux 13 (trixie)","kernel":"6.12.41+deb13-amd64",
 "root_fs":{"used_pct":23},
 "arrays":[{"name":"md2","level":"raid5","state":"clean","degraded":false,"sync_pct":null,"sync_action":null}],
 "disks":[{"dev":"/dev/sda","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":34}],
 "volumes":[{"mount":"/srv/data","used_pct":41,"fstype":"ext4"}],
 "services":[{"name":"synofurb-mgmt","active":true}],
 "boot":{"default":null,"flash_present":false},"updates":{"apt_pending":4,"security_pending":0},
 "overall":"ok","warnings":[]}
```

`health` needs no root for most of it (SMART does); a user without root and without a sudo password gets the unprivileged part. A unit whose
script is too old to know `health` answers with an argparse error (exit 2); the engine reports that as "too old to report health" and still
shows the installed version.

### 2. `... update --from DIR --json`

`DIR` holds exactly `synofurb_mgmt.py`, `VERSION`, `README.md`, optionally `synofurb-mgmt.service` and the add-on framework (`synofurb_addons.py`, `addons.pack.json`: the add-on definitions of
`addons/<id>/*` as one file, [ADDONS.md](ADDONS.md)), and `MANIFEST.sha256` (`sha256sum`
format, every file listed, no others). The unit verifies the manifest, compiles the script (syntax only), copies to `/opt/synofurb-mgmt.new`,
swaps it with `/opt/synofurb-mgmt` (the old tree is kept as `/opt/synofurb-mgmt.prev`), installs the unit file when packaged, restarts
`synofurb-mgmt.service`, waits up to 30 s for the new code to answer `health --json` with the expected version, and on any failure restores
the old tree and unit file and checks again. It prints one NDJSON line per event, **live**, and the engine shows them as they arrive:

```json
{"step":"verify","status":"ok","message":"manifest verified, version 0.4.0 (installed 0.3.0)"}
```

Steps: `verify`, `stage`, `swap`, `restart`, `health`, `rollback` (only after a failure), `result`. Statuses: `start`, `ok`, `fail`. The
engine accepts other spellings (`running`, `done`, `failed`, `warn`, `skipped`, ...) and extra keys, and shows plain text lines as they are.
Exit codes: **0** updated, **2** refused before anything changed, **3** failed and rolled back, **4** failed and the rollback failed.
`--force` allows installing an older version.

### 2b. `... addons ...` (add-ons, not used by the health/update flow)

`synofurb_mgmt.py addons list|show|status|logs|install|update|remove|start|stop|restart|apply-pending [--json]` manages the Docker add-ons (Docker, Portainer, Frigate, Plex, user add-ons). It runs as root on the unit,
takes the same job lock as `update` (so an add-on job and a management-plane update never run together: the second is refused), and `apply-pending` is what the first-boot unit calls to take over the installer's
choices. A management plane update replaces the whole `/opt/synofurb-mgmt` tree: add-on state (`/var/lib/synofurb-mgmt/addons*.json`) and data (`/srv/addons`, `/srv/data/addons`) are outside it and untouched, and the
bundle must carry `synofurb_addons.py` and `addons.pack.json` or the unit loses the add-on page until the next update that has them. Full description: [ADDONS.md](ADDONS.md).

### 3. `... update --rollback --json`

Exchanges the installed and the `.prev` tree (running it twice rolls forward). Same output and exit codes. Exit 2 with
`rollback fail: no previous version is kept` when there is nothing to go back to.

## Updating the management plane itself: web UI and GitHub

The unit can update itself, from its own web UI (page **Updates**) or from the shell (`synofurb_mgmt.py update --from-github`), in addition to
the push from the desktop app. All three end in the same `Updater` (verify, stage, swap, restart, health, automatic rollback, the previous
version kept in `/opt/synofurb-mgmt.prev`); the GitHub paths only add *check*, *download* and *verification* in front of it, and the push path
needs no Internet at all (a test makes any network use on it fail).

| Entry point | Command | Source label in the history |
|---|---|---|
| Web UI | page Updates, **Update now** / **Roll back** (runs the CLI below as a job) | GitHub release / GitHub source / Rollback |
| GitHub, shell | `update --from-github [--ref main\|<tag>] [--check] [--accept-unsigned] [--force] [--json]` | GitHub release (no `--ref`) / GitHub source (`--ref`) |
| Desktop app | `update --from DIR --json` (staged under `/tmp/.synofurb-mgmt-stage-*`) | Desktop app |
| Shell, a bundle | `update --from DIR` (any other folder) | Command line |
| Any | `update --rollback`, `update --status [--json]` | Rollback |

**Sources.** *Stable* (no `--ref`): the newest non-draft, non-pre-release GitHub release that carries `synofurb-mgmt-<ver>.tar.gz` (the highest
*management plane* version wins, not the newest upload; the file `scripts/package-mgmt.sh` builds) and its `SHA256SUMS`. *Main* (`--ref R`, a
branch or tag, default `main` in the web UI): the `apps/mgmt-plane` folder of `andreaswiren/synofurb` at `R`. The check resolves `R` to the
newest commit that touches that folder (shown in the UI, with the last commit subjects as the "release notes"), and the download is pinned to
that commit (`codeload.github.com/.../tar.gz/<sha>`), so what was shown is what is installed. Only the files the bundle contract allows are taken
(`BUNDLE_FILES`: `synofurb_mgmt.py`, `VERSION`, `README.md`, `synofurb-mgmt.service`, `synofurb_addons.py`) by exact name, never by the archive's path; the
add-on definitions `addons/<id>/<file>` are validated (names, sizes, counts) and packed into `addons.pack.json` by the installed `synofurb_addons.build_pack`, exactly as
`tools/make-update-bundle.py` does; links, duplicates and a second top directory are refused; `MANIFEST.sha256` is written by `write_manifest`, whose output a test compares
with the tool's, byte for byte. `VERSION` must equal the version that was checked and `DEFAULT_VERSION` in the script.

**Trust.** Releases carry `SHA256SUMS` but no signature today (the signing key is a placeholder, `docs/UPDATES.md`), so the desktop app's model applies:
the asset's SHA-256 is compared with the line in the `SHA256SUMS` *of the same release*, which catches a damaged download but not a tampered release,
and the person must consent: the web UI shows "Not signed" and a required "I understand and want to update" (for Main also the short commit to type);
the CLI needs `--accept-unsigned`. A source tree has no signature at all, so Main always needs that consent. Once a real minisign key is configured
(`update_pubkey` in `/etc/synofurb/mgmt.json`, the same `RW...` key `install.sh` takes), signatures are mandatory: Stable runs `minisign -Vm SHA256SUMS`
and refuses when `minisign` is missing, the signature asset is missing or the signature is wrong (never a silent fall back to SHA-256); Main is blocked
unless `update_allow_unsigned_source` is true in the same file. The mgmt plane has no minisign implementation of its own: it needs the `minisign` package.

**Network policy.** https only, to `api.github.com`, `github.com`, `codeload.github.com`, `objects.githubusercontent.com` and `release-assets.githubusercontent.com`
(the CDN a release download is redirected to); the policy is applied to the first URL and to every redirect hop, URLs with credentials and other ports are refused,
no GitHub token or any other credential is sent or stored (anonymous API: 60 requests per hour, a rate limit is reported with the minutes to wait). Limits: 2 MiB per API
answer, 256 KiB for `SHA256SUMS`, 8 MiB for a release asset and for every file, 256 MiB for the repository tarball, 2 GiB unpacked (a gzip bomb stops there).

**Safety rules.** A GitHub update is refused (exit 2, nothing changed) while the unit's health is `crit`, the web service is not active or less than 64 MiB is free, unless
`--force` (the page offers **Update anyway** after such a refusal); a version older than the installed one also needs `--force`. The push path (`--from DIR`)
keeps the contract above: the app looks at health itself first. **One job at a time**: every update and every add-on job holds `/var/lib/synofurb-mgmt/job.lock`
(`JobLock`, created with `O_EXCL`, stale when its process is gone); a second one is refused with exit 2 and the step `lock` (web: HTTP 409). Every update is written to the audit log
(`mgmt-update-start`, `mgmt-update`, with job id, source, from, to; no secrets) and to a **job record** `/var/lib/synofurb-mgmt/update/jobs/<id>.json` (the last 30 are kept): that is the
version history the page shows (when, from, to, source, who, result, and the step events). A job whose process disappeared (power loss) is closed as failed at the next look.

**Surviving the restart.** The web request only creates the job record and starts `synofurb_mgmt.py update --from-github --job-id ...` with `systemd-run --collect` (its own transient unit,
outside the service's cgroup, so `systemctl restart synofurb-mgmt.service` cannot kill it), then answers with the job id. The job waits 3 s before the restart so that answer and
the first polls are delivered. The page polls `POST /api/update/job`; when the service restarts, connections fail and then answer 401 (sessions live in memory), and the page switches to
`POST /api/update/progress`, a public route that returns only the job's state, step and version numbers for an unguessable job id, until the job ends; it then offers **Sign in again**.
If the new version fails its health check the job restores the old files and checks again (the page says "Rolled back").

NDJSON steps added by the GitHub paths (before the Updater's own): `lock`, `check`, `download`, `consent`. Their failures end the stream like `verify` does (exit 2). `update --from-github --check --json`
prints one JSON object (`channel`, `version`, `tag`/`commit`, `state` = `available|current|same|older`, `mode` = `signed|unsigned|blocked`, `blocked`, `notes`, `installed`) and changes nothing.

## What the verdict means

The unit computes `overall` itself. For small screens the engine adds a **summary** (`Health::summary()`, the `summary` object of
`manage health --json`): the verdict, a headline, at most five one-sentence bullets (worst first, each at most 96 characters; a sixth problem
becomes "And N more."), and counts: `degraded_arrays`, `rebuilding_arrays`, `failed_disks`, `services_down`, `volumes_nearly_full`,
`security_updates`, `pending_updates`.

| Finding | Level |
|---|---|
| array `degraded` (a missing or failed member) | crit |
| a disk whose SMART self-assessment failed; a disk at 70 C or more | crit |
| a file system (or the system disk) at 95 % or more | crit |
| array `inactive`; a rebuild/resync/reshape in progress on a healthy array | warn |
| a service reported `active: false`; 85 % full; a disk at 60 C or more; less than 5 % memory available | warn |
| security updates pending (ordinary pending updates are only counted) | warn |

The derived verdict is the worse of the unit's `overall` and these rules, so it can raise but never lower what the unit says. When the unit
says `warn`/`crit` for something these rules do not explain (a `check` running, a section that was unavailable), the unit's own sentences from
`warnings` are listed instead of an unexplained tag. An unrecognised `overall` word is shown as a warning, not trusted. `manage health` exits
0 for ok, 8 for warn, 9 for crit.

## Update flow

`manage update` (and `manage::push_update` in the engine):

1. **Look.** Read `VERSION` (or the script's built-in default for very old installs) and the script's SHA-256, then ask `health`. Plan:
   * same version and same script: *up to date*, nothing is done (`--force` reinstalls);
   * unit older: *update*; same version number but a different script: *update* (a development build);
   * unit newer than this program's copy: refused (exit 3) unless `--force`, which passes `--force` to the unit;
   * not installed, or version unreadable: *install* this copy.
2. **Ask.** A normal confirmation (`--yes` to skip, `--dry-run` to only print the plan). It is not data-destructive; it says the web UI
   restarts for a few seconds. Without a terminal and without `--yes` the answer is "no" (exit 3).
3. **Upload.** Build a deterministic tar.gz of the files plus `MANIFEST.sha256`, upload it to a private file in `/tmp` over the existing SSH
   layer, check its SHA-256 on the unit, unpack into a `0700` staging directory and `sha256sum -c` the manifest there.
4. **Apply.** Run the unit's own `update --from STAGE --json`. Progress lines are shown live.
5. **Verify.** Call `health` again (up to four tries, three seconds apart): the version must be the new one and the `synofurb-mgmt` service,
   if reported, must be active. If the unit said "updated" but this check fails, the engine asks the unit to `update --rollback` and reports a
   failure.
6. **Clean up.** The staging directory and the archive are removed, also on every failure path.

**After a failure the report says what state the unit is in**, decided by a fresh `health` call, not by trusting a message:

| Rollback state | Meaning |
|---|---|
| `restored` | the unit answers and runs the version it had before: the old files are back |
| `not_applicable` | the unit refused the files (exit 2) before changing anything, or the update never started |
| `incomplete` | the unit answers but runs a different version than before: do not assume anything, look at it |
| `unverified` | the health call after the failure did not complete: look at the unit (`ssh`, `journalctl -u synofurb-mgmt`) |

Units installed before this feature have no `update` (or even `health`) in their copy of the script. For those the engine uploads the same
staged files and runs a small built-in swap script that does what the unit's updater does (verify, keep the old tree as
`/opt/synofurb-mgmt.prev` with the old unit file as `unit.backup`, put the new files in place, restart, check, restore on failure, same NDJSON,
exit 3 / 4) so a later `manage rollback` works. The report names the method used (`unit` or `bootstrap`).

If the connection is lost while the unit is working, the unit's updater should carry on by itself (it does not depend on the SSH session; not tested on a real unit) and the
engine reports the state it finds when it looks again. Ctrl-C before the update is applied cancels cleanly; after that the update finishes on the unit.

## Offline copies

`--from-bundle DIR` takes the files from a folder instead of the copy built into the program. `DIR` is either the files themselves
(`synofurb_mgmt.py`, `VERSION`, `README.md`, optionally `synofurb-mgmt.service`, `synofurb_addons.py`, `addons.pack.json`; `python3 apps/mgmt-plane/tools/make-update-bundle.py OUT`
makes such a folder, including its `MANIFEST.sha256`) or an **offline bundle** made by `synofurb bundle create`, which carries the same files in `mgmt/`
([OFFLINE.md](OFFLINE.md#the-management-app-mgmt)). Give either the bundle or its `mgmt/` folder:

```
synofurb manage update --host 10.0.0.5 --from-bundle /media/usb/bundle        # same as .../bundle/mgmt
```

For a real offline bundle (a `manifest.json` next to `mgmt/`) the engine opens the bundle and checks `mgmt/MANIFEST.sha256` **and** every file against the bundle's `manifest.json` before anything is
contacted (mismatch: usage error, exit 2), so the update that reaches the unit is exactly the one that was bundled; a bundle built before `mgmt/` existed is refused with the
hint to run `bundle create` again on the same directory. For a plain folder a `MANIFEST.sha256` in it is verified (same exit); symbolic links are refused and files outside the list ignored. The
unit verifies the manifest again after the upload. Nothing here needs the Internet. Without `--from-bundle` the copy built into the program is used.

**Fresh installs write `VERSION`.** The installer puts `VERSION` next to `synofurb_mgmt.py` in `/opt/synofurb-mgmt` (from `apps/mgmt-plane/VERSION`, the one source of the version for the installer
payload, the built-in copy and the bundle's `mgmt/`). Without it the script would report its built-in default, so `health --json`, `manage health` and `manage version` show the installed version
from the first boot and `manage update` can tell an old copy from the current one. Units installed by earlier builds lack the file; they report the script's built-in default version (the same number for the same script) until a `manage update` replaces the files (`--force` reinstalls identical ones) and writes `VERSION`.

## Engine API (for the desktop app)

`synofurb_core::manage` (no UI dependency; every call takes a `Privileged` shell, so it is testable with a scripted unit):

| Item | Purpose |
|---|---|
| `Health`, `parse_health`, `fetch_health(p)` | the report; `ManageError::{NotInstalled, HealthUnsupported, BadOutput, Command, Transport}` say why there is none |
| `Health::summary() -> HealthSummary` | `overall`, `headline`, `bullets[{level,text}]`, `problems`, counts: the whole Manage screen needs this |
| `bundled_mgmt_version()`, `MgmtBundle::embedded()`, `MgmtBundle::from_dir(dir)` | the management plane shipped in this program (same files the installer's payload carries, including `VERSION`), or a folder / offline bundle (`offline::Bundle::mgmt()` does the bundle checks) |
| `compare(nas, bundled) -> VersionRelation`, `read_installed(sh)`, `status(p, &bundle) -> ManageStatus` | version, health and plan in one call, never fails because of the unit's own state |
| `plan_update(&installed, &bundle, force) -> UpdatePlan` | what an update would do (`UpdateDecision`, files with hashes, a sentence) |
| `push_update(p, &bundle, &opts, &mut emit) -> UpdateReport`, `push_rollback(...)` | the flows; `emit` gets `ManageEvent`s live |
| `ManageEvent::{Step, Progress, Remote, Log}` | `to_core()` -> `logs::Event`, `emit(&Logger)`, `to_json()` (`step`, `progress`, `log`, `nas_step`) |

`UpdateReport` has `outcome` (`updated|up_to_date|rolled_back|refused|failed`), `operation`, `method`, `from_version`, `to_version`, `plan`,
`rollback` (state above), `rollback_reported`, `remote_steps[]`, `failure`, `health_after`, `summary_after`, and `headline()` (one sentence).
A failed update is an `Ok(report)`; `Err` means the unit could not be reached or driven at all.

## What is verified

Done in the development sandbox:

* **Parsing and summary**: fixtures for a healthy unit, a degraded array, a failed SMART check, services down, missing fields, nulls, wrong
  types, future schema, noise around the JSON, and the real output of the repository's script (`health --json` run with `--root`).
* **Update flow with a scripted unit** (a fake implementing `RemoteShell`): success, up to date, downgrade refused / forced, unit rolled
  back (exit 3), rollback failed (exit 4), refusal (exit 2), "success" without the new version answering, no `health`, no `update`, upload
  failure, cancellation, rollback command, sudo (no secret in any command line or upload), and a check that no command touches `/dev`,
  `mdadm`, `mkfs`, `parted`, the boot flash, `reboot`, ...
* **Against the real script**: our staged tarball is accepted by the real `verify_bundle` of `synofurb_mgmt.py`
  (`update --from ... --dry-run --root`), its live NDJSON and its exit codes 2/3/4 are parsed and mapped to the right failure text.
* **Shell scripts run for real** with `sh`: the staging script (unpack, pinned hash, manifest) and the built-in swap script (in a sandbox root:
  replace, `.prev` copy, `unit.backup`, first install, refusal on a bad checksum).
* **Offline bundle `mgmt/`**: `bundle create` against a fake archive writes it and `bundle verify` accepts it; damage (changed file, script and `mgmt/MANIFEST.sha256` edited together, missing manifest, symbolic link) is found by `verify`, by an install check and by `manage update --from-bundle`; a bundle without `mgmt/` still verifies with a note; the folder equals the output of `make-update-bundle.py`; `tests/cli/smoke.sh` pushes `--from-bundle <bundle>` over the real `sshd` through the real updater.
* **Fresh-install `VERSION`**: the payload carries `mgmt/VERSION` identical to `apps/mgmt-plane/VERSION`, and the management-plane block of the real `payload/chroot-setup.sh` is run with `sh` in a scratch root (files installed, `VERSION` next to the script, nothing installed with the management plane off); `sh -n`, `dash -n` and `shellcheck -s dash` on that script. Not run: the installer on a real unit.
* **Over a real `sshd`** (`tests/cli/smoke.sh`): the CLI uploads and stages over SSH to a fake unit whose commands are redirected to
  the repository's script in a sandbox, `manage health` (exit 8/9, JSON), `version`, `update --dry-run`, a refused update, a real update that
  ends in the unit's failure/rollback path with NDJSON, `rollback`, and the staging directory being removed.

* **Self-update from GitHub** (`apps/mgmt-plane/tests/test_selfupdate.py`, 60+ tests): a fake release server on 127.0.0.1 with the real downloader (host policy incl. redirects, size limits, partial files removed, typed errors, no credentials),
  release selection (newest non-draft non-pre-release, foreign assets ignored), SHA-256 match and mismatch, hostile archives (traversal, links, extra members, a second top directory, duplicates, add-on entries, a size cap on unpacking), signed mode with a fake `minisign`,
  consent and "what you saw is what you get", the health gate, the job lock (busy and stale), audit lines, job records and their reaping, the full flow and the automatic rollback through the same `Updater` with a fake systemd and health,
  the CLI (`--check`, `--ref`, `--status`, offline), the web routes against the real HTTP server (`systemd-run` argv checked, 409 while busy, typed commit for Main, public progress route), the real release asset built by `scripts/package-mgmt.sh` and a tarball of the real
  `apps/mgmt-plane` tree staged byte-identical to `make-update-bundle.py`; the Updates page in a real browser against the demo (check, consent, typed commit, restart and reconnect, rollback, refusal and "Update anyway", phone width).

**Not verified:**

* The self-update on a real appliance with real systemd: `systemd-run` placing the job outside the service's cgroup (tested by argv only), a real `systemctl restart` of the service that serves the page, the page reconnecting through a real restart (simulated in the demo, with the 401 the real service gives after losing its sessions).
* Anything against the real `api.github.com`, `codeload.github.com` and the release CDN: the tests use a loopback server, never the Internet. The accepted host list (notably `release-assets.githubusercontent.com`) follows how GitHub redirects release downloads today; if that changes the update says which host it refused.
* A real minisign key and signature (a fake `minisign` binary is used; the key is a placeholder).

* Anything on a real appliance, with real systemd: the happy-path restart of `synofurb-mgmt.service`, the 30 s health wait with a live
  web server, the timing of the first health call after the restart, `systemctl enable`.
* The built-in swap script under real systemd (its `systemctl` branches are skipped in the sandbox; the file operations are tested).
* Real RAID/SMART data: the summary rules are tested on fixtures shaped like the script's output, not on a unit with a failing disk.
* Behaviour when the SSH connection drops in the middle of the unit's updater (relies on the updater not depending on the session).
* The desktop app screens (done separately).
