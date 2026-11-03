# Add-on catalog

Each folder is one add-on: a manifest plus templates. The contract (fields, lifecycle, CLI/API) is `docs/ADDONS.md`; this page is the short
how-to and the list of images the shipped definitions pin. The management plane is Python standard library only, so manifests are JSON and
templates are `string.Template` text (`${name}`; write a literal dollar as `$$`). Nothing is ever parsed as YAML at run time.

```
addons/<id>/addon.json          manifest (the folder name must equal "id")
addons/<id>/compose.yml.tmpl    docker compose file, rendered into /srv/addons/<id>/
addons/<id>/<file>.tmpl         more files, listed under "files" (dest relative to /srv/addons/<id>/, "once": true = never overwrite)
```

## Writing an add-on

1. Copy `plex/` (simple) or `frigate/` (devices, per-choice blocks, a config file) and change `id`, `name`, `version`, `summary`, `homepage`.
2. List every published port under `ports` (the web UI port must be among them) and the UI under `web`.
3. Put user-facing options under `settings`. Keys are `[a-z][a-z0-9_]*` and become `${key}` in the templates. Use `secret` for anything
   that must not be shown again (tokens, camera URLs with passwords). Give every setting a default that renders a working configuration,
   because templates have no conditionals: an optional thing is a setting whose default is harmless (a disabled placeholder, an empty
   string), not a block that is left out. Put free-form rules in a `pattern`: whatever a setting can contain ends up in a config file
   (for example Frigate's camera URL pattern keeps go2rtc `exec:` sources out).
4. Big or growing data (recordings, media, transcoder files) goes under `storage` with `"bulk": true` and a path below `/srv/data`, the
   mount point of the redundant RAID/LVM data volume. Reference it as `${STORAGE_<KEY>}`. Small config goes to `${DATA_DIR}` (`/srv/addons/<id>`).
5. Hardware is `devices[]`, always `"optional": true`: the UI offers it only when `detect` finds it, and a missing device never stops the
   install. Put `${DEVICES_BLOCK}` alone on a line at the end of the service (see below).
6. Anything that widens what the container can do is a `privileges` entry (`host-network`, `privileged`, `docker-socket`, `cap:<NAME>`),
   shown before install. Prefer none.
7. Add a `healthcheck` (`http` to a 127.0.0.1 port, `tcp` or `container`) and a short `post_install` text for what the user does next.
8. Run `python3 -m unittest tests.test_addons_catalog` from `apps/mgmt-plane`: it checks the manifest against the field rules, renders every
   template strictly (unresolved `${name}` fails) for every combination of choice settings, and checks the required lines.

### Conventions the shipped definitions rely on

* `${DEVICES_BLOCK}`: the loader emits either nothing or a complete service-level block already indented by four spaces
  (`    devices:\n      - /dev/dri:/dev/dri\n`). The template puts the placeholder at column 0 on its own line.
* Booleans render as `true` / `false`.
* `choice` settings may be plain strings or objects `{ "value", "label", "vars": { "NAME": "text" } }`. The `vars` of the selected option
  become extra template variables (this is how Frigate picks its detector block and Plex its network block). Every option of one choice must
  define the same variable names. Multi-line values carry their own indentation and are placed at column 0.
* A `pattern` is checked for non-empty values; an empty optional value is allowed.
* Config files rendered with `"once": true` are written the first time only. They belong to the user afterwards, so settings that only
  feed such a file take effect on the first install only.
* Everything here is a definition; nothing is run against Docker by the tests.

## Pinned images (checked 2026-11-03)

| add-on | image | tag | what the tag was on 2026-11-03 | source of the check |
|---|---|---|---|---|
| frigate | `ghcr.io/blakeblackshear/frigate` | `stable` | image built 2026-10-15, Frigate 0.18.2 (amd64, arm64) | ghcr.io registry API, GitHub releases page, docs.frigate.video (0.18 docs) |
| plex | `plexinc/pms-docker` | `latest` | built 2026-10-26, Plex Media Server 1.43.5.11079 (amd64, arm64, arm); `1.43.6.11085` already exists as a version tag | Docker Hub tags API, github.com/plexinc/pms-docker README |

`stable` and `latest` move; `update` pulls the new image. To pin a release put the version tag in the template and bump the definition `version`.
Frigate's configuration schema changes between releases (0.17 split `record.retain` into `continuous` / `motion`); the template targets 0.18.
