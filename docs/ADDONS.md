# Add-ons (plugin system for Docker apps)

Status: **contract** shared by the management plane (`apps/mgmt-plane`), the desktop installer (`crates/synofurb`, `synofurb-core`)
and the add-on catalog. Implementation notes are added at the bottom by whoever builds each part.

An add-on is a small directory in the management plane's `addons/` folder (shipped with the management plane and updated with it,
plus an optional user folder `/etc/synofurb/addons.d/<id>/`). The management plane renders its compose template, runs
`docker compose` for it, shows it in the web UI under **Add-ons**, and (Stage 2) publishes its web UI through the HAProxy front.
The management plane is Python standard library only: manifests are **JSON**, compose files are **text templates**
(`string.Template`, `${key}` placeholders), never parsed as YAML.

## Layout

```
apps/mgmt-plane/addons/<id>/addon.json          the manifest (below)
apps/mgmt-plane/addons/<id>/compose.yml.tmpl    docker compose template
apps/mgmt-plane/addons/<id>/<file>.tmpl         other files to render (e.g. config.yml.tmpl)
/etc/synofurb/addons.d/<id>/                    optional user add-ons, same layout (an id the built-in catalog already has is refused)
/var/lib/synofurb-mgmt/addons.json              state: installed add-ons, their non-secret settings, versions, status (0600)
/var/lib/synofurb-mgmt/addons-secrets.json      secret setting values (0600, never returned by the API, never logged)
/srv/addons/<id>/                               the add-on's config: compose.yml and rendered files (`${DATA_DIR}`, on the system volume)
/srv/data/addons/<id>/...                       bulk data (recordings, media, transcoder files): the redundant data volume the installer mounts at /srv/data
```

An add-on folder is flat (no sub-folders, no links, file names `[A-Za-z0-9._-]`, at most 256 KiB each). The update bundle is a flat file list too, so the shipped
form of `addons/` is one file, `addons.pack.json` (`synofurb_addons.build_pack()`); the loader reads the folder when it exists (a source checkout) and the pack otherwise.

## Manifest (`addon.json`, schema 1)

```json
{
  "schema": 1,
  "id": "frigate",
  "name": "Frigate NVR",
  "version": "1",
  "summary": "Local AI camera recorder.",
  "category": "video",
  "icon": "camera",
  "homepage": "https://github.com/blakeblackshear/frigate",
  "kind": "compose",
  "requires": ["docker"],
  "web": { "port": 8971, "scheme": "https", "path": "/", "hostname_hint": "frigate" },
  "ports": [ { "host": 8971, "container": 8971, "proto": "tcp", "label": "Web UI" }, { "host": 5000, "container": 5000, "proto": "tcp", "label": "Internal API", "bind": "127.0.0.1" } ],
  "settings": [
    { "key": "retain_days", "label": "Keep recordings (days)", "type": "int", "default": 7, "min": 1, "max": 365, "help": "Older recordings are deleted." },
    { "key": "detector", "label": "Object detector", "type": "choice", "default": "cpu", "choices": [
      { "value": "cpu", "label": "CPU", "vars": { "DETECTOR_BLOCK": "detectors:\n  cpu1:\n    type: cpu\n" } },
      { "value": "coral", "label": "Coral USB", "vars": { "DETECTOR_BLOCK": "detectors:\n  coral:\n    type: edgetpu\n    device: usb\n" } } ] }
  ],
  "storage": [ { "key": "media", "label": "Recordings", "path": "/srv/data/addons/frigate/media", "bulk": true, "min_free_gib": 50 } ],
  "devices": [
    { "id": "dri", "label": "Intel graphics (hardware decoding)", "host": "/dev/dri", "optional": true, "detect": "path:/dev/dri/renderD128" },
    { "id": "coral_usb", "label": "Coral USB accelerator", "host": "/dev/bus/usb", "optional": true, "detect": "usb:1a6e:089a|18d1:9302" },
    { "id": "coral_pcie", "label": "Coral PCIe/M.2 accelerator", "host": "/dev/apex_0", "optional": true, "detect": "path:/dev/apex_0" }
  ],
  "privileges": [],
  "compose": "compose.yml.tmpl",
  "files": [ { "template": "config.yml.tmpl", "dest": "config/config.yml", "once": true } ],
  "healthcheck": { "type": "http", "url": "http://127.0.0.1:5000/api/version" },
  "post_install": "Open the web UI, then add your cameras in Settings."
}
```

Field rules (the loader validates and rejects the whole add-on with a clear message on a violation):

* `id`: `[a-z0-9][a-z0-9-]{0,31}`; the folder name must equal it. `version`: the *definition* version (a string that changes when the manifest or templates change).
* `kind`: `compose` (a Docker app) or `system` (a host package/service such as Docker itself, installed by a named handler; no compose file).
* `requires`: other add-on ids that must be installed and running first (Docker first). Cycles are rejected.
* `settings[].type`: `string`, `int`, `bool`, `secret` (stored 0600, never shown after saving, never logged), `choice` (with `choices`), `path`, `text`
  (multi-line). Every setting has `key` (`[a-z][a-z0-9_]*`), `label` (short), optional `help` (shown behind an info button, not as a paragraph), `default`, validation
  (`min`/`max`/`pattern`/`choices`/`required`). **Enforced rules**: `int` is range-checked by `min`/`max`; for `string`, `secret`, `path` and `text` a non-empty value must
  match `pattern` (full match) and have a length between `min` and `max` (an empty optional value is allowed; `required: true` rejects it); `bool` renders as `true`/`false`;
  `path` values must be normalized absolute paths below `/srv` or `/mnt` (letters, digits, `. _ @ + - /`) and the folder must exist when the add-on is installed.
  A shipped `secret` may have a harmless placeholder `default` (a loopback address, empty).
* `choices[]` are strings or objects `{ "value", "label", "vars": { "NAME": "text" }, "privileges": [...] }`. The `vars` of the **selected** option become extra template variables
  (this is how conditional content is done: `string.Template` has no conditionals); every option of one choice defines the same variable names; multi-line text carries its own
  indentation and is placed at column 0. `privileges` of an option are added to the add-on's privileges only while that option is selected (Plex: `host-network` only with host networking).
* Template variables: settings become `${key}`; the loader also provides `${ADDON_ID}`, `${DATA_DIR}` (`/srv/addons/<id>`), `${STORAGE_<KEY>}` (storage paths),
  `${PUID}`, `${PGID}`, `${TZ}` and `${DEVICES_BLOCK}`: either an empty string or a finished service-level block indented four spaces
  (`"    devices:\n      - /dev/dri:/dev/dri\n"`) for the devices the user enabled; the template puts the placeholder alone on a line at column 0. `${key_json}` is the value as a JSON
  string (quoted, safe anywhere). **Where a value may go** is decided by where its placeholder sits: inside "double quotes" of the compose file it may not contain `"`, `\` or a dollar sign;
  unquoted in the compose file it must be a plain token (letters, digits, `. _ / @ + = , : ~ % ? & -`); inside quotes of a rendered file it may not contain `"` or `\`; no value contains a control
  character (`text` may contain newlines but cannot be used in the compose file). A literal dollar sign in a template is written twice (`string.Template` rule).
* `PUID`/`PGID`: the first administrator (member of `sudo`) with a regular uid, and its primary gid; never 0: when there is none, 1000/1000. No extra user is created.
* `ports[]` may add `"bind": "127.0.0.1"` (default `0.0.0.0`); a web port bound to loopback gets no "Open" link in the UI.
* `storage[].bulk = true`: large data (recordings, media). It must live on the redundant data volume. The installer mounts the data LV at `/srv/data`, so the path must be below
  `/srv/data` (the loader refuses anything else, also when the user edits the path) and `/srv/data` must be a mount of its own (not the system disk) when the add-on is installed;
  the loader creates the folders (owned by PUID/PGID) and warns when free space is below `min_free_gib`. `${DATA_DIR}` stays `/srv/addons/<id>` on the system volume for configuration.
* `devices[]`: optional hardware passthrough. `detect` is `path:<file>` or `usb:<vid:pid>|<vid:pid>`; the UI offers only devices that are present (or lets the user
  force it with a warning). A Coral that is not plugged in must not stop the add-on from installing.
* `privileges[]`: any of `docker-socket`, `host-network`, `privileged`, `cap:<NAME>` (plus those of the selected choices). They are shown to the user **before** install, must be
  confirmed (`accept`), and are recorded in the audit log. The loader checks the compose template against them: a mounted Docker socket, `privileged: true`, `network_mode: host` or a
  `cap_add` that is not declared is refused; host paths such as `/etc`, `/` or `/var/lib/docker`, shared pid/ipc namespaces, `env_file`, `build`, `extends`, `include`, a hand-written `devices:` and
  `unconfined` are never allowed.
* `files[]`: extra templates rendered into `${DATA_DIR}`; `once: true` means render only when the destination does not exist (never overwrite user edits). The UI says
  "used on first install only" for settings that feed only such files (a later "re-render" action is out of scope). Files that contain a secret are written 0600, root.
* `changes[]` (optional, mostly system add-ons): short sentences shown before install about what changes on the unit. `handler` (system add-ons): the name of built-in code (`docker`).
* `web`: the add-on's web UI behind the host (`scheme` http or https to the container). Without the front the UI shows `http(s)://<host>:<port>`; with the HAProxy front (Stage 2, [CERTIFICATES.md](CERTIFICATES.md)) the person assigns a hostname and a certificate to each installed add-on that has a `web` section
  (`hostname_hint` is the suggested name, shown as the placeholder). HAProxy terminates TLS and talks to `127.0.0.1:<web.port>`: **plain HTTP when `web.scheme` is `http`, re-encrypted without verification (loopback, self-signed) when it is `https`** (Frigate 8971, Portainer 9443); the Hostnames page can override the scheme per add-on.
  The add-on keeps its published port: HAProxy does not change containers, so the direct `http(s)://<host>:<port>` link keeps working on the LAN unless the manifest binds that port to `127.0.0.1`. Hostnames, certificates and the override live in `/etc/synofurb/edge/proxy.json`, not in the add-on's state.
* `healthcheck`: `http` (`http(s)://127.0.0.1:<port>/...` only; status < 500 counts as up; certificates are not verified on loopback), `tcp` (port opens) or `container` (compose reports running);
  `timeout` (seconds, default 120) is how long install waits and how long "Starting" is shown before "Not answering".
* Unknown manifest fields are rejected (typos); fields starting with `x-` or `_` are ignored.

## Lifecycle (the management plane owns it; web UI, API and CLI share one implementation)

`install` (render, pull, up) · `start` · `stop` · `restart` · `update` (pull images, up, keep data) · `remove` (down; data kept unless the user ticks "delete data") ·
`status` · `logs`. Compose project name `synofurb-<id>`. Every mutating action asks for confirmation in the UI, is written to the audit log and runs as a background job with
visible progress. Image pulls need Internet access; offline units show a clear reason.

CLI (also used by the installer's first boot): `synofurb_mgmt.py addons list|show <id>|install <id> [--set key=value ...] [--yes]|remove <id> [--delete-data]|start|stop|restart|update|status|logs|apply-pending`.
API: `GET /api/addons`, `GET /api/addons/<id>`, `POST /api/addons/<id>/install|start|stop|restart|update|remove`, `GET /api/addons/<id>/logs`, job status `GET /api/jobs/<id>` (details in the implementation notes below).

## Built-in add-ons

| id | kind | what |
|---|---|---|
| `docker` | system | Docker Engine and the compose plugin from Debian (or Docker's apt repository), plus `docker` group wiring; no web UI |
| `portainer` | compose | Portainer CE as the base container manager (`portainer/portainer-ce:lts`, port 9443, mounts the Docker socket: privilege `docker-socket`) |
| `frigate` | compose | Frigate NVR (`ghcr.io/blakeblackshear/frigate:stable`); recordings on the redundant data volume; optional Intel VAAPI and Coral |
| `plex` | compose | Plex Media Server (`plexinc/pms-docker`); needs a claim token (setting, secret) at first start; media folders on the data volume |

## Installer integration (desktop app and CLI)

The Setup step has an **Add-ons** tab: a list of the catalog entries (id, name, summary, what it needs, privileges), tick to install with the system. Quick setup offers presets
(for example "Docker + Portainer", "Frigate NVR"). The choice travels as `InstallConfig.addons: Vec<AddonChoice { id, settings }>` and as repeatable
`--addon <id>[:key=value,...]` on the CLI (and in "Copy command"). The stage-2 script installs the Docker packages (needs the mirror) and writes
`/etc/synofurb/addons.d/pending/<id>.json` (the choices and non-secret settings; secrets go through the same stdin path as the other secrets, never argv);
a first-boot systemd unit runs `synofurb_mgmt.py addons apply-pending`, which installs them in order, retries on failure and reports the result in the web UI.
Images are pulled at first boot (Internet needed); the installation does not fail when a container cannot be pulled, the add-on shows "needs attention".

## Stage 2 (not part of the first delivery): certificates and the HAProxy front

One HAProxy fronts the management plane and every add-on web UI; certificates are managed in one place (several certificates, per hostname), issued by Let's Encrypt with DNS-01
(through `challenges.addr.tools`), HTTP-01 and TLS-ALPN-01, renewed automatically. **Implemented** (never run against a real Let's Encrypt or HAProxy): `docs/CERTIFICATES.md`, pages **Certificates** and **Hostnames**, module `apps/mgmt-plane/synofurb_edge.py`.

## Implementation notes: installer side (`synofurb-core::addons`, `payload/`)

**Catalog.** The installer carries a small built-in list (`crates/synofurb-core/src/addons.rs`: id, name, summary, kind, `requires`, `privileges`,
needs-Internet, rough size, and the few settings worth asking for while installing). The test `catalog_matches_the_management_plane_manifests` compares id, name,
summary, kind, requires, privileges and the type of every install-time setting with `apps/mgmt-plane/addons/<id>/addon.json`. **Change a manifest's name, summary,
requires or privileges and the installer's list in the same commit**, or that test fails. Install-time settings are deliberately few: Frigate `retain_days`,
Plex `claim_token` (secret, optional). Cameras, detector and everything else are set in the web UI afterwards.

**Docker packages (Debian 13 trixie).** `docker.io` (Engine 26.1), `docker-cli` (a "Recommends" of `docker.io`, so it must be named: the installer installs without
recommends) and `docker-compose` (Compose v2, Go; it ships `/usr/libexec/docker/cli-plugins/docker-compose`, so `docker compose` works; `docker-compose-v2` does not
exist in trixie). Stage 2 installs them from the configured mirror. An **offline bundle has no Docker packages**: stage 2 then skips them and the first-boot script installs them from the
official Debian mirrors (Internet needed). A later extension can add the Docker packages to the bundle.

**Files stage 2 writes** (only when add-ons were chosen; the directories are 0700, the files 0600, root):

```
/etc/synofurb/addons.d/pending/<id>.json          one per add-on, Docker first; only these (no secrets)
/etc/synofurb/addons.d/pending-secrets/<id>.json  secret settings as {"key": "value"}; only for add-ons that have some
/etc/synofurb/addons.d/docker.packages            the package list used for Docker (one line)
```

```json
{ "schema": 1, "id": "frigate", "order": 3, "source": "installer", "auto": false, "required_by": [], "requires": ["docker"],
  "settings": { "retain_days": "14" }, "secrets": [] }
```

`order` is the install order (dependencies first). `auto: true`: added only because something else requires it (`required_by` says what). `settings` values are **strings as
typed**; coerce by the manifest's setting type and validate. `secrets` lists the secret setting names that are in `pending-secrets/<id>.json`. Secret values are never
in `pending/`, in `env.sh`, on a command line or in a log: they travel in the install payload (the same token-protected LAN transfer as the password hash), and
the CLI takes them from an environment variable or a file.

**`synofurb_mgmt.py addons apply-pending`** (implemented, see the management plane notes below): for each `pending/<id>.json` in `order`: install the add-on with those settings (the `docker` entry is a no-op when
Docker is already installed, else its handler installs it), move the secrets into the add-on's state (0600), and when the add-on is recorded (installed, or "needs attention" with the reason) delete its
`pending/<id>.json` and `pending-secrets/<id>.json`. Exit code 0 when every file was taken over, **including add-ons that failed and now show "needs attention"** (the user retries in the web UI);
non-zero only when it could not work through the files at all (Docker missing, state not writable, network down before anything was tried): the files then stay and the
first-boot script retries. Files that do not match `<id>.json` with `id` in `[a-z0-9][a-z0-9-]*` are ignored.

**First boot.** `synofurb-addons-firstboot.service` (`After=network-online.target docker.service`, `ConditionPathExists` and `ConditionDirectoryNotEmpty` on the pending folder, `Type=oneshot`)
runs `/usr/local/sbin/synofurb-addons-firstboot`, which makes sure Docker is installed, runs `python3 /opt/synofurb-mgmt/synofurb_mgmt.py addons apply-pending` up to four times
(two minutes apart) until the pending folder is empty, writes `/var/lib/synofurb/addons-firstboot.json` (`docker`: present|installed|failed, `apply_pending_exit`, `attempts`,
`remaining`: ids still pending) and always exits 0. The pending folder survives reboots, so the unit tries again at the next boot until the management plane has taken everything over.

**Desktop app.** Setup has an **Add-ons** tab (after Storage and System): a checkbox per catalog entry with its summary, at most one risk Tag (the privilege) and one info button (needs, size, minutes, data-volume need), the
settings above under a ticked row, Docker shown on and locked with "Required by ..." while something ticked needs it, and an **Untested** Tag (these add-ons have only run against simulated Docker). An add-on that keeps data below `/srv/data`
(`needs_data_volume`: Frigate and Plex, as the manifests' bulk storage says; a core test compares) cannot be ticked while the plan creates no data volume (Keep mode, `Plan::has_data_volume`): its row says why and Next is disabled with the same reason. Quick setup has presets
(None, Docker + Portainer, Frigate NVR, Plex, Frigate + Plex; each with Docker and Portainer as the base) and every preset names everything it installs. **Confirm** shows, whichever way the add-ons were ticked, the names, the Untested Tag, the lines of
`addons::notes` that must not be missed (`NoteKind::is_loud`: privileges, tight system volume, no data volume, no Docker in the offline bundle) and the rest in Details; a fourth confirmation **Accept privileges** (`addons::consent_items`, tied to
`consent_signature`) is part of the erase gate while a privileged add-on is chosen. **Done** has a card per add-on (`addons::after_install`: address from the manifest's `web`, minutes at 150 MiB a minute, size, `first_use`; nothing is read back from the
unit). "Copy command" writes one `--addon` per ticked add-on (a secret as `key=@env:KEY`). The CLI is `--addon ID[:KEY=VALUE,...]` on `plan` and `install` and `addons list [--json]` (docs/CLI.md, secrets as `KEY=@env:VAR` or `KEY=@file:PATH`).

## Implementation notes: management plane (`apps/mgmt-plane/synofurb_addons.py`)

Nothing in this part has run against a real Docker daemon. The tests use `FakeDocker` (a test double for the `Docker` class) or the real `Docker` class on a scripted runner (to check
the exact command lines), scripted `apt-get`/`systemctl`, a fake clock and a temp root. **First run on a unit** (Docker install from the Debian mirror, a pull, a Portainer start) is the open check.

**Code layout.** One new module, standard library only, with no import from `synofurb_mgmt` (the web module imports it, so it also works when `synofurb_mgmt.py` runs as `__main__`). The
glue in `synofurb_mgmt.py` is one delimited block (section 17b: environment, route registration, CLI hook), a `find_route()` for paths with an id in them, a `query` field on `Request`, and the Add-ons page.
A bundle without `synofurb_addons.py` simply has no add-ons. `AddonEnv` is the seam: paths (`--root` aware), the command runner, `mounts`, `audit`, the job lock, probes, clock.

**Loader.** `parse_manifest()` validates one add-on (precise messages such as `settings[2] (port): 'min' is larger than 'max'`); `load_catalog()` loads the built-in folder or pack and
`/etc/synofurb/addons.d/<id>/`, never lets one bad add-on hide the others (`problems` in `GET /api/addons` and `addons list`), rejects unknown or rejected requirements and every member
of a dependency cycle (`dependency cycle: a -> b -> a`), and refuses a user add-on whose id the built-in catalog has. Templates are checked at load time: unknown `${name}`, a stray `$`,
`${DEVICES_BLOCK}` anywhere but alone at column 0, a `text` setting in the compose file, and the privilege consistency check described above.

**Rendering.** `string.Template`, values validated per placeholder position (see "Where a value may go"). The compose file is written to `compose.next.yml`, checked with
`docker compose config -q`, and only then moved over `compose.yml`, so a bad update never destroys the working file; `once` files are not touched when they exist; writes go through a
temp file and `os.replace`, and a folder that is a link out of the add-on's directory is refused. The rendered compose text is audited again against the effective privileges
(manifest plus selected choices) before it is written. Files using a secret are 0600.

**Lifecycle and jobs.** `install` (check, render, pull, start, health), `update` (the same with the stored settings; the definition version is recorded), `start` (`compose up -d`), `stop`, `restart`,
`remove` (`compose down --remove-orphans`; **delete data** also removes the Docker volumes and the add-on's own folders `/srv/addons/<id>` and `/srv/data/addons/<id>`, never a mount point,
never a path outside those folders, a link is unlinked and not followed; the API wants `confirm: "<id>"`), `status` (containers from `compose ps`, then the health check: `running`, `stopped`,
`starting`, `unhealthy`, `error`, `needs-attention`, `installing`, `updating`, `removing`, `not-installed`), `logs`. Docker cannot be stopped or removed while an installed add-on requires it. Every
mutating call is a **job**: `{id, kind, addon, state running|done|failed, step, steps, progress, message, ok, error, warnings, log[]}`, kept in memory and, once finished, in
`/var/lib/synofurb-mgmt/addons-jobs.json` (a job that was running at a crash comes back as failed, and its add-on as "needs attention"). Install keeps the add-on in `needs-attention` with the
reason when a step fails after the files were written (a pull without Internet, a bad compose file); running `install` again is the retry and reuses the stored settings.
**One job at a time**: the add-on jobs take the management plane's `job.lock` (the same lock `update` uses), so a second request, or an update of the management plane, is refused with HTTP 409 while one runs
(`Another job is running (install portainer)`); in a plain environment it is a `flock` on `addons.lock`. The lock is released before the job shows as finished. Audit log: the request (secret
values replaced by `***`), `addons.job.start` (add-on, kind, privileges, devices, storage, `accepted: true`, user, address) and `addons.job.<kind>` with the result.

**Health.** `http` probes `127.0.0.1` only (no redirects followed, certificate not verified), `tcp` connects, `container` needs every container running and not unhealthy. After `up`, install waits up to
`healthcheck.timeout`; a late answer is a warning, not a failure.

**Secrets.** Stored only in `addons-secrets.json` (0600, atomic writes); the state file lists their names. The API returns `{"set": true}`; the stored values are removed from job logs, `logs` output and
error messages; an empty secret field keeps the stored value; `secret` values are refused on the CLI command line (`--set`), they come as JSON on standard input (`--settings-stdin`).

**Docker add-on (`kind: system`, `handler: docker`).** Debian 13 (trixie), checked against the archive on 2026-11-03: `docker.io` 26.1.5, `docker-cli` 26.1.5 (a "Recommends" of `docker.io`, so it is named),
`docker-compose` 2.26.1 (Go; installs `/usr/libexec/docker/cli-plugins/docker-compose`; there is no `docker-compose-v2` in trixie), `docker-buildx` 0.13.1 (not needed). Steps (all idempotent): when all three packages are
installed nothing is downloaded; otherwise `apt-get update`, a `Candidate:` check with `apt-cache policy`, then `env DEBIAN_FRONTEND=noninteractive apt-get -y install --no-install-recommends ca-certificates docker.io docker-cli docker-compose`;
`/etc/docker/daemon.json` with `json-file` rotation (10 MB x 3) only when no such file exists; `systemctl enable --now docker.service`; every administrator (member of `sudo`, uid >= 1000) joins the
`docker` group when not already in it (the group is root-equivalent: the add-on's `changes` say so); a final `docker version` / `docker compose version` check. An offline unit gets "needs the Debian mirror and
Internet access". `update` runs `apt-get install --only-upgrade`; `remove` is refused (the packages stay; the message names the `apt-get remove` command).

**Portainer.** `portainer/portainer-ce:lts`, `9443` (https, self-signed), `${DATA_DIR}/data`, mounts `/var/run/docker.sock` (privilege `docker-socket`, shown and confirmed before install). The first-run note (create the
administrator within 5 minutes, otherwise restart the add-on) is `post_install`, shown in the job log and in Details.

**Web UI.** Add-ons page: installed list (name, state Tag, open / stop-start / restart / update / logs / details / remove; a problem's reason, such as "No space left on device", is a second line under the name; an add-on that needs
attention is listed only here, with **Retry**), catalog cards for what is not installed (icon, name, summary, state Tag, Install; without Docker the button is `aria-disabled` and the card says "Install Docker first." in text),
install dialog (settings form from the manifest, `help` behind an info button whose accessible name carries the setting ("Help: Media access"), devices with Found / Not found, storage with free space and the data-volume error,
**privileges listed before install with a required confirmation**, changes; the pinned footer says "Accept the privileges first." beside the disabled Install), job dialog (progress bar, step, log; its button reads **Hide** while the job
runs, because closing the dialog does not stop it, and **Close** afterwards), logs viewer, update and remove confirmations (**delete data** needs the typed id).
The open link is `http(s)://<host>:<port>` (Stage 2 replaces it). Add-on names and labels come from the manifests and count as data in the word-budget audit (`data-d`); summaries and notes are manifest prose (`data-d="prose"`), which the audit
counts separately and caps on its own: **a summary has at most 20 words** (tested) and one page at most 60 words of manifest prose. Give every `choice` option a `label` (`{ "value": "ro", "label": "Read only (safe)" }`): a bare string is shown as the raw value.
All chrome text is in `COPY`. Screenshots of every state: `design/screenshots/mgmt-demo-addons-*` (made with `apps/mgmt-plane/tools/shots.js`, see the README of the management plane).

**API.** `GET /api/addons` `{addons[], problems[], job}`; `GET /api/addons/<id>` (settings, values with secrets as `{set}`, `storage_report`, `device_states`, `privileges`, `changes`, `warnings`);
`POST /api/addons/<id>/install` body `{settings:{}, storage:{key:path}, devices:{id:true|false}, accept:true|[privileges]}`; `.../update|start|stop|restart` (empty body); `.../remove` body `{delete_data, confirm}`;
all answer **202** `{ok, job}`; `GET /api/addons/<id>/logs?tail=N` (10..2000); `GET /api/jobs/<id>`, `GET /api/jobs`. Errors: 400 (rejected input, message safe to show), 404, 409 (busy). Session cookie, CSRF and the same-origin check apply as everywhere.

**CLI.** `addons list|show|status|logs|install|update|remove|start|stop|restart|apply-pending [--json]`; `install ID [--set k=v] [--storage k=path] [--device id=on|off] [--settings-stdin] [--yes]` (`--yes` accepts the
privileges shown); `remove ID [--delete-data --yes]`. Progress is printed live; exit 0 ok, 1 the operation failed or another job is running, 2 refused or bad input.

**`apply-pending`** reads the installer's files exactly as described in the installer notes (strings as typed, secrets from `pending-secrets/<id>.json`, refused when that file is readable by group or others), installs
in dependency order (the catalog's `requires`, then `order`), retries a failed install 3 times with 10 s, 30 s, 90 s backoff, records an add-on that still fails (and every add-on that waits for it) as "needs attention" with the reason
and the settings, and removes the pending files of everything it recorded. Unknown ids and unreadable files are moved to `pending/rejected/`. It writes `/var/lib/synofurb-mgmt/addons-apply.json`
(`{ok, stuck, items:[{id, status installed|failed|skipped|rejected|waiting, message, attempts}]}`, no secrets). Exit 0 when every file was taken over; **1 only when files had to stay** (another job held the lock, the state could not be
written), so the first-boot script retries. `docker` already installed (for example by stage 2) is a no-op that is recorded.

**Packaging.** `addons.pack.json` and `synofurb_addons.py` are optional members of the update bundle, of `tools/make-update-bundle.py`, `scripts/package-mgmt.sh` and `verify_bundle()`. The desktop app's embedded copy
of the management plane (`crates/synofurb-core/src/manage.rs`, `MGMT_FILES` and the staging script) must list the same two files, or an app push removes the add-ons from the unit (the unit keeps working, the page says so).

**Security review of this part.** No shell anywhere (argv lists, `shell=False`; a test greps the module); every id, project name, compose path, setting value, storage path and device path is validated before it reaches an argv, a
template or a file name; compose command lines are built in one place (`Docker.argv`). Template injection: values are checked per placeholder position, `$` is escaped for compose, unknown placeholders are refused, `text` never reaches the compose
file, and the rendered compose is audited again. Path traversal: add-on folders are flat, file names and `dest` paths are validated, `realpath` of the parent must stay inside the add-on folder, links are refused or never followed, delete-data only removes
the add-on's own folders. Docker socket: privilege `docker-socket` is shown before install, confirmed, audited, and a template that mounts the socket without declaring it is refused. A user add-on is trusted code (root-owned `/etc`): the template
check protects against mistakes and mismatched privilege lists, it is not a sandbox. Not done: image signature verification (images are pulled by tag), pinning by digest, a network egress policy for containers.

