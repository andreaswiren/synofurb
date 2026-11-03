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
/var/lib/synofurb-mgmt/addons.json              state: installed add-ons, their settings (secrets stored 0600), versions, status
/srv/addons/<id>/                               the add-on's data (compose file, config, volumes); bulk data on the redundant RAID volume
```

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
  "ports": [ { "host": 8971, "container": 8971, "proto": "tcp", "label": "Web UI" } ],
  "settings": [
    { "key": "retain_days", "label": "Keep recordings (days)", "type": "int", "default": 7, "min": 1, "max": 365, "help": "Older recordings are deleted." }
  ],
  "storage": [ { "key": "media", "label": "Recordings", "path": "/srv/addons/frigate/media", "bulk": true, "min_free_gib": 50 } ],
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
  (multi-line). Every setting has `key` (`[a-z][a-z0-9_]*`), `label` (short), optional `help` (shown behind an info button, not as a paragraph), `default`, validation (`min`/`max`/`pattern`/`choices`/`required`).
  Settings become `${key}` in templates; the loader also provides `${ADDON_ID}`, `${DATA_DIR}` (`/srv/addons/<id>`), `${STORAGE_<KEY>}` (storage paths),
  `${PUID}`, `${PGID}`, `${TZ}` and `${DEVICES_BLOCK}` (a ready-made compose `devices:` list for the devices the user enabled).
* `storage[].bulk = true`: large data (recordings, media). It must live on the redundant data volume (the mdadm/LVM volume of the installed system),
  never on the system disk's single copy; the loader refuses a bulk path outside the data volume and warns when free space is below `min_free_gib`.
* `devices[]`: optional hardware passthrough. `detect` is `path:<file>` or `usb:<vid:pid>|<vid:pid>`; the UI offers only devices that are present (or lets the user
  force it with a warning). A Coral that is not plugged in must not stop the add-on from installing.
* `privileges[]`: any of `docker-socket`, `host-network`, `privileged`, `cap:<NAME>`. They are shown to the user **before** install and recorded in the audit log.
* `files[]`: extra templates rendered into `${DATA_DIR}`; `once: true` means render only when the destination does not exist (never overwrite user edits).
* `web`: the add-on's web UI behind the host (`scheme` http or https to the container). Stage 1 shows `http(s)://<host>:<port>`; Stage 2 publishes it through HAProxy
  under a hostname with its own certificate (`hostname_hint` is the suggested name).
* `healthcheck`: `http` (status < 500 counts as up), `tcp` (port opens) or `container` (compose reports running).

## Lifecycle (the management plane owns it; web UI, API and CLI share one implementation)

`install` (render, pull, up) · `start` · `stop` · `restart` · `update` (pull images, up, keep data) · `remove` (down; data kept unless the user ticks "delete data") ·
`status` · `logs`. Compose project name `synofurb-<id>`. Every mutating action asks for confirmation in the UI, is written to the audit log and runs as a background job with
visible progress. Image pulls need Internet access; offline units show a clear reason.

CLI (also used by the installer's first boot): `synofurb_mgmt.py addons list|show <id>|install <id> [--set key=value ...] [--yes]|remove <id> [--delete-data]|start|stop|restart|update|status|logs`.
API: `GET /api/addons`, `GET /api/addons/<id>`, `POST /api/addons/<id>/install|start|stop|restart|update|remove`, `GET /api/addons/<id>/logs`, job status `GET /api/jobs/<id>`.

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
(through `challenges.addr.tools`, see `docs/CERTIFICATES.md` when it exists), HTTP-01 and TLS-ALPN-01, renewed automatically.
