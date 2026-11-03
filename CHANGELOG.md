# Changelog

## 0.2.3 (unreleased)

### Added (discovery, CLI)
- **Choose what Find NAS scans.** `discovery::networks()` lists this computer's IPv4 networks (id `<interface>|<ip>/<prefix>`, CIDR, broadcast, `kind` = `lan` / `vpn` / `virtual` / `link_local` / `other`, `recommended`). Only real LAN networks are recommended and scanned by default: Tailscale (100.64.0.0/10), WSL, Hyper-V, Docker, VMware, VirtualBox and similar adapters are listed but not scanned unless selected. `ScanOptions { networks, methods, timeout }` with `scan_with(&options, &events, &cancel)` runs the methods on the selected networks only (broadcast to their broadcast addresses, mDNS/SSDP on their adapters, port sweep inside them, never our own address) and reports `ScanEvent`s (`Started`, `MethodStarted`, `MethodProgress`, `MethodFinished` with the error, `Skipped`, `Found`, `Finished`); `CancelToken` stops the sweep in about a second. Custom CIDR networks larger than /20 are refused with a clear message. The old `scan()` and the per-method `scan` functions now use the recommended networks.
- CLI: `scan --list-networks [--json]`, `scan --network <CIDR|interface|id>` (repeatable), `scan --all-networks`; progress per method and network on stderr, and in `--ndjson` (every line has an `event` tag; device lines keep their fields); `--json` adds `networks` and `errors`.

### Added (installer: add-ons)
- **Add-ons in the install.** `synofurb_core::addons`: a built-in catalog (Docker, Portainer, Frigate NVR, Plex Media Server; id, name, summary, requires, privileges, needs-Internet, rough size, install-time settings) with a test that compares it with the management plane's manifests, `AddonChoice { id, settings }` (secret settings are held apart, never serialized, redacted in `Debug`), dependency resolution (Docker is added when anything needs it), plan lines and warnings (Internet at first boot, privileges such as `docker-socket` and `host-network`, disk use, offline limits) and validation. `InstallConfig.addons` is optional (`serde(default)`), so older saved configurations load; add-ons need the web management plane.
- **Stage 2 and first boot.** With add-ons chosen, `chroot-setup.sh` installs the Docker packages from the mirror (Debian 13: `docker.io docker-cli docker-compose`; `docker-compose-v2` does not exist in trixie), writes `/etc/synofurb/addons.d/pending/<id>.json` (choices, no secrets) and `pending-secrets/<id>.json` (0600), and enables `synofurb-addons-firstboot.service`, which runs `synofurb_mgmt.py addons apply-pending` after the network is up (retried, never fails the boot) and writes `/var/lib/synofurb/addons-firstboot.json`. An offline bundle has no Docker packages: they are fetched on the first boot (Internet needed). The unit, the script and the contract are in `docs/ADDONS.md`. Tested with fake `apt-get`/`systemctl`/management script under `sh`, `shellcheck`, `systemd-analyze verify`; not run on a real unit or against Docker.

### Added (CLI: add-ons)
- `--addon ID[:KEY=VALUE,...]` (repeatable) on `plan` and `install`, and `addons list [--json]`. Docker is added when anything needs it; the plan (text and `--json` `addons` key) and the dry run list what is installed, privileges, disk use and the Internet need. A secret setting is only accepted as `KEY=@env:VAR` or `KEY=@file:PATH`; a literal value is refused and never echoed, and dry runs write a placeholder. See `docs/CLI.md`.

### Added (add-on catalog)
- Definitions for the **Frigate NVR** and **Plex Media Server** add-ons (`apps/mgmt-plane/addons/frigate`, `addons/plex`, contract in `docs/ADDONS.md`) and `addons/README.md` (how to write an add-on, pinned images). Frigate: recordings and clips on the data volume (`/srv/data/addons/frigate/media`), detector choice (CPU, OpenVINO on Intel graphics or CPU, Coral USB or PCIe), optional VAAPI decoding, four camera slots, no privileges, optional Intel graphics and Coral passthrough that never blocks the install. Plex: host or bridge network, optional claim token, read-only media folder, transcoder files on the data volume, optional Quick Sync. Catalog tests in `tests/test_addons_catalog.py` (own validator, strict template rendering). Definitions only; not run against Docker, cameras or a Plex account.

## 0.2.2

### Fixed (desktop app, updates)
- **Find NAS: saved bogus entries are removed.** Versions before 0.2.1 saved this computer's own network adapters as "Synology" units in `inventory.json`; they came back at every start even after the scan fix. On load, entries seen only by the Synology UDP scan that name nothing (no model, host name, serial or MAC) are dropped and the file is rewritten.
- **A failed update check was almost invisible** (the owner saw the button "blink once"). Settings, Updates and the About page now show one line for every state: checking (spinner), "0.2.2 is current", "0.2.2 available" (Details) or "Couldn't check: <reason>" with the full reason behind an (i) and a "Check again" button; the result stays until the next check. The About page has a "Check for updates" button and status line.
- **"Offline" was shown for any failed check.** The failure is now typed: only a transport failure (GitHub's API host cannot be reached) sets the Offline chip. A 404 (a private repository) says "Releases not found. If the repository is private, the update check cannot see them."; a rate limit shows the engine's hint with the time; no release says "No release found for this system."
- **The red dot on the Settings button was always drawn** (the badge ignored its count). It now appears only while an update waits, and the button's name says so.
- **The About page said "dev" on release builds.** It read an environment variable the build never set; it now shows `release` for a build made from a tag matching the version, `dev <commit>` otherwise and `local build` without git information.
- A manual "Check for updates" shows an update even when that version was skipped or snoozed.

### Added (desktop app, updates)
- The update dialog for a release that is not signed (placeholder key): plain-text release notes in a scroll area with a scrollbar, ONE warning line, a required "I understand and want to install it" checkbox with a plain-words risk note, "Download and install" (disabled with a visible reason until ticked), then "Unsigned, checksum verified" with the SHA-256. One warning only (the duplicate "signing key is a placeholder" and "no SHA256SUMS.minisig" alerts are gone).
- Demo screens `updates-checking`, `updates-current`, `updates-available`, `updates-failed-notfound`, `updates-failed-offline`, `updates-unsigned`, `settings-updates`, `update-unsigned`; screenshots `design/screenshots/v2-app-update*-*.png`.

### Added
- **Opt-in unsigned update path** while the update signing key is still the placeholder (`updater::unsigned_allowed()`): `download_unchecked_signature(info, dest, UnsignedConsent::acknowledged(), progress)` downloads with the same host policy and size limits and checks the SHA-256 against the `SHA256SUMS` of the same release (the `.minisig` is not needed). It is refused (`UpdateError::UnsignedNotAllowed`) as soon as a real key is embedded, so signatures become mandatory automatically. `VerifiedDownload` now says `signed` / `integrity: Signed | ChecksumOnly`; `UpdateInfo::install_mode()` reports `Signed | UnsignedWithConsent | Blocked(reason)`; `updater::UNSIGNED_WARNING` is the shared warning text. CLI: `self-update --allow-unsigned` (needs `--yes` without a terminal; `--yes` alone never enables it); `self-update --check --json` adds `install_mode`, `install_blocked_reason`, `signature_asset`, `unsigned_allowed`, `notes_plain`. The refusal text now says how to opt in.
- `updater::plain_release_notes(md)` and `UpdateInfo::notes_plain()`: release notes as readable plain text instead of raw Markdown.

## 0.2.1

### Added
- **Welcome guide** on the first launch of the desktop app: five short steps (what synofurb does, the six steps, before you start, if something goes wrong, ready), each with one icon, a headline and at most two lines; details sit behind info buttons. It replaces the old "Before you start" dialog and keeps its statements (nothing changes until you confirm, back up and test, screen readers are not supported). Reopen it from Help (first row) or the title-bar menu. Demo screens `welcome`, `welcome-2` to `welcome-5`; design notes in `design/v2/UX-SPEC.md` (D1b) and DEV-UI-23.
- Review fixes (UX and UI critics): step 4 is conditional and consistent with `docs/RECOVERY.md` (before the disks change a failure returns the NAS to DSM; after that DSM must be reinstalled; the caveats are behind the info button); Skip, Esc, the close button and a click outside close the guide for this launch only, and only the last step's "Don't show again" box stores "seen" (a replay shows the stored flag and unticking clears it); the screen-reader statement is visible on step 1; step 2 says where changes begin and captions each step; stock carousel pagination for the position; one dialog height on every step and text size; the headline is the dialog title; focus starts on the primary button and Enter works; no Back on step 1.

### Changed
- The "seen" flag of the first-launch dialog is now `welcome_done` in `ui-settings.json`; the old `first_launch_done` flag is ignored on purpose, so everyone sees the new guide once.

### Fixed
- **Windows: opening the app no longer opens an empty console window.** `synofurb.exe` is now a GUI-subsystem program in release builds; when it is started
  from a console for a command-line job (`synofurb.exe scan`, `--version`, ...) it attaches to that console so the output still appears. `synofurb-cli.exe`
  stays a console program (use it in scripts). The release workflow checks both subsystems. Reported on the first Windows release (0.2.0).
- **Discovery listed this computer's own network adapters as "Synology" units** (Find NAS showed the PC's LAN, Tailscale and virtual-adapter addresses).
  The Synology UDP scan received its own broadcast query and accepted it as a reply. Our own packets and packets from our own addresses are now ignored,
  and a reply must name a unit (model, host name, serial or MAC) to count.

## 0.2.0 (unreleased)

### Added
- **SBOM and EU Cyber Resilience Act self-assessment** (engineering documents, not legal advice, no conformity claim): `scripts/gen-sbom.py` (CycloneDX 1.5 + SPDX 2.3 + licence report from `cargo metadata --locked`, with the management plane, fonts, icons, installers, payload and boot-agent build inputs; reproducible; `--check` validator, `--cargo-lock` cross-check), `scripts/gen-licenses.py` (`THIRD-PARTY-LICENSES.txt` from the cargo registry sources), 30 tests in `tests/sbom`, `docs/compliance/` (scope and classification, Annex I mapping, risk assessment, technical-file index, vulnerability handling, user information, unsigned declaration template, gap list), a draft `SECURITY.md`, `deny.toml` and `.github/dependabot.yml` (neither wired into CI), and optional SBOM/licence names in `scripts/package-collect.sh`. Generated SBOMs are release artefacts and are not committed.
- **Boot through the internal boot flash (DOM)** for x86 Synology units, Intel and AMD, over SSH only: `boot/dom/synofurb-dom` (copy-edit-verify-write with
  automatic rollback and a full backup streamed to the user's computer first), the SSH-controlled **boot agent** (`boot/agent`) that `kexec`s the Debian
  installer, and `BootMethod::Dom` in the engine. DSM stays the default boot entry; every failure path resets it (docs/RECOVERY.md).
- **Boot manager mode** (`synofurb boot-manager install|enter`) and **recovery** (`synofurb recover`): try other Linux systems by hand, always one reboot from DSM.
- **Disk metadata backup** (first and last MiB of every touched disk) before any install.
- **Linux 7.x**: kernel channel *Newest* (trixie-backports, default) or *Stable* (6.12 LTS).
- **AMD**: CPU vendor detection, `amd64-microcode`/`intel-microcode`, memory-mapped serial consoles (`console=uart8250,mmio32,...`).
- **Compatibility list** for 272 Synology models (`compat/`), lookup and refresh in the app and CLI (`synofurb compat`).
- **Auto-update** from GitHub releases with SHA-256 and minisign verification (`synofurb self-update`).
- **GPUI Kit desktop UI** (Windows, macOS, Linux) replacing the egui prototype; mockups, critic reviews and screenshots under `design/`.
- **Management plane** moved to `apps/mgmt-plane` (own version, 0.2.0): ForgeVault-family design, Boot manager page, kernel/microcode status, read-only imports.
- **Release engineering**: tag-triggered workflow (MSI, NSIS, DMG, deb, rpm, AppImage, portable), one-line installers (`install.sh`, `install.ps1`),
  design-approval gate with a UX critic and a UI critic.

- **Manage a converted unit from the app and the CLI** (`synofurb_core::manage`, `synofurb-cli manage health|version|update|rollback|open`, docs/MANAGE.md): basic health and updates of the web management UI on the appliance, over SSH.
  Health: the unit's `health --json` parsed leniently (only `schema`, `version`, `overall` required), plus a derived summary for small screens (verdict, at most five one-sentence problems, counts of degraded arrays, failed disks, services down). Update: plan (up to date / update / same version but changed files / downgrade needs `--force`), a deterministic tar.gz of the shipped files with `MANIFEST.sha256`, upload with a pinned SHA-256, the unit's own `update --from` with its progress streamed live, a second health call, and a report that says whether the unit is back on its old version after a failure (`restored|incomplete|unverified`). A built-in swap script updates units whose management plane predates `update`. `--from-bundle DIR` takes an offline copy. Never touches disks or the boot flash. New exit codes **8** (`manage health`: warn) and **9** (crit). The management plane version shipped in the app is `apps/mgmt-plane/VERSION` (embedded at build time, with a test that it matches the script's built-in default).
- **Hybrid RAID (SHR-like) data layout** in the installer: `--raid shr1|shr2` (and the wizard/plan lists). The data space of every disk is cut into slices at the disk sizes, each slice becomes an mdadm array (`data1`, `data2`, ...; RAID 1/5 for SHR-1, RAID 6 or mirrors for SHR-2), all joined as PVs of `vg_data` with one linear xfs volume, so mixed disk sizes are used fully. Pure slice algorithm `plan::shr_slices` (docs/RAID.md), warnings for unusable single-disk slices and SHR-2 mirror fallbacks, clear plan errors for too few disks and for combination with preserved arrays. `VgPlan` now holds a list of physical volumes (`pvs`; old plans with a single `pv` still load).

- **Offline bundles carry the management app** (`mgmt/` in the bundle: `synofurb_mgmt.py`, `VERSION`, `README.md`, `synofurb-mgmt.service`, `MANIFEST.sha256`, the same files as
  `apps/mgmt-plane/tools/make-update-bundle.py`). `bundle create` writes it (also when resuming), `bundle verify` checks it against both its own `MANIFEST.sha256` and the bundle's `manifest.json`
  (new `MGMT` lines, new `note:` lines, exit 1 on damage), `bundle info` shows its version (`mgmt_version` in JSON), an install refuses a bundle whose `mgmt/` is damaged, and
  `manage update --from-bundle <bundle>` (or `<bundle>/mgmt`) takes the files from there, offline, after the same checks. Bundles from earlier builds without `mgmt/` still verify and install (a note says so;
  `manage update --from-bundle` on one explains how to add it). No change to the bundle format number.

- **Typed boot-flash failures** (`synofurb_core::session::SessionError`): `AgentTimeout { waited_min, watchdog_min }` when the boot agent does not answer, with a message that states both outcomes (watchdog resets
  to DSM, counted from the agent's start; or the agent never ran and the unit may boot it again after every power cycle) and the manual recovery, and no longer says DSM is the default or boots normally; `FlashBackupFailed`
  (the boot flash was not written to), distinct from later failures. CLI exit codes **10** (agent timeout) and **11** (SSH host key refused).
- **Unit boot state read from the unit** (`recovery::read_unit_boot_state`, `UnitBootState`, `recover status [--json]`): default entry, synofurb's entries, agent files, flash backup, agent watchdog. "Could not read" is its own state
  (`ReadOutcome`, `DefaultEntry::Unknown`) and is never shown as "DSM is the default".
- **Recovery actions as engine functions** with typed `RecoveryError`s and an explicit `confirmed` flag: `enter_agent`, `restore_flash` (image checked, uploaded, hashed on the unit, read back), `remove_our_entries`,
  `boot_dsm` (verifies the default by reading it back, reboots only when asked, reports "reboot requested" only). CLI: `recover boot-dsm|remove|restore`.
- **DSM probe** (`recovery::probe_unit`, `wait_for_dsm`, `recover wait`): DsmBack / LoaderOnly / AgentStillUp / SshOnly / Unreachable from the SSH banner and DSM's web ports.
- **SSH host-key pinning** (`hostkey::HostKeyPolicy`: `Pinned`, `Tofu` with a confirm callback, `KeyAuthOnly` for the agent, `Insecure`; `SshSession::connect_with`): the key is checked before any password is sent, a changed key is a hard
  error naming both fingerprints, pins live in `known_hosts.json`. CLI: `--host-key`, `--known-hosts`, `--accept-new-host-key`, `--insecure-host-key`, `host-key list|forget`, and a first-contact question in the wizard.
- **All time limits in one module** (`synofurb_core::timing`): install watchdog 30 min, boot manager 60, agent default 20, app wait 10, installer failure reboot 60 s / 15 min.

### Fixed (engine, round 2)
- **Host-key question vs connect timeout**: the 20 s connect timeout no longer runs while a person answers "trust this key?" (answer time 180 s, `timing::HOST_KEY_ANSWER_TIMEOUT_SECS`); no answer is refused (`HostKeyError::AnswerTimeout`), a late "Trust" cannot pin, and a first-time key is pinned only after the handshake completed on a live connection.
- **Installer failure reboot** finds `synofurb-dom` by absolute path (`/usr/local/sbin`, ...) instead of relying on the installer's PATH, a boot-flash install refuses to start (before any change) when the tool is missing from the installer image, and `script::failure_handling` reports `reboots_by_itself = false` without the tool. Tested with the generated script in a stand-in environment.
- **Flash backups belong to one unit**: `domboot::backup_dom_for` writes `<image>.json` (serial, model, address); `recovery::backups_for_unit(dir, &UnitIdentity)` lists only the backups of that unit (two units of one model never see each other's); `restore_flash` / `restore_flash_for_unit_on` refuse another unit's image (`RecoveryError::OtherUnit`) unless `allow_other_unit`.
- RECOVERY.md and BOOT.md carry the plain `sha256sum -c` + `dd` route for restoring the flash from another computer.

### Fixed (engine, round 3)
- `readback_verified` of the recovery outcomes is no longer a constant: `RestoreOutcome.readback_verified` is true only if the flash was read back from the unit and hashes to the image (`readback: Verified | Mismatch(hash) | NotPossible(why)`; a mismatch is `RecoveryError::Unverified`), `RemoveOutcome` and `BootDsmOutcome` carry the same field from their own read-back. `UnitBootState.device` records the flash device the tool reported.

### Changed
- `bundle create` now builds for the **Newest** kernel by default (`--kernel newest|stable`; `--kernel-newest` still accepted), the same default as `install`, so a plain bundle fits a plain install. A kernel mismatch between bundle and install names both ways out (`--kernel <bundle's>` or `bundle create --kernel <install's>`). A bundle holds one kernel and has no offline fallback to the other.
- Installer failure handling: on units with a boot flash a failed or cancelled install reboots into DSM (60 s before the point of no return, 15 min after).
- ARM boards: the installer prints the exact U-Boot commands before the reboot.

### Fixed
- The installer's failure messages and the CLI texts no longer promise a rescue shell (there is none), no longer say that reinstalling DSM keeps data volumes, and the 60 s / 15 min reboot is only described for boot-flash units;
  other units halt and do not reboot by themselves.
- **A freshly installed unit now has `/opt/synofurb-mgmt/VERSION`.** The installer payload shipped `synofurb_mgmt.py` without it, so `synofurb_mgmt.py health --json` and `synofurb-cli manage health|version`
  reported the script's built-in default version instead of the installed one. `apps/mgmt-plane/VERSION` is the single source for the payload, the built-in `manage update` copy and the bundle's `mgmt/`.
- Password hashes now use the canonical 16-character salt (glibc `crypt()` rejects longer ones, which would have broken the first login).

## 0.1.0
- First version: discovery, inspection, storage planner, generated installer, management plane, egui desktop prototype.
