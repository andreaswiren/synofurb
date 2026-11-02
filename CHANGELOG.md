# Changelog

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
