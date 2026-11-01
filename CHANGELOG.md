# Changelog

## 0.2.0 (unreleased)

### Added
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

### Changed
- Installer failure handling: on units with a boot flash a failed or cancelled install reboots into DSM (60 s before the point of no return, 15 min after).
- ARM boards: the installer prints the exact U-Boot commands before the reboot.

### Fixed
- Password hashes now use the canonical 16-character salt (glibc `crypt()` rejects longer ones, which would have broken the first login).

## 0.1.0
- First version: discovery, inspection, storage planner, generated installer, management plane, egui desktop prototype.
