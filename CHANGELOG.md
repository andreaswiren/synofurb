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

### Changed
- Installer failure handling: on units with a boot flash a failed or cancelled install reboots into DSM (60 s before the point of no return, 15 min after).
- ARM boards: the installer prints the exact U-Boot commands before the reboot.

### Fixed
- Password hashes now use the canonical 16-character salt (glibc `crypt()` rejects longer ones, which would have broken the first login).

## 0.1.0
- First version: discovery, inspection, storage planner, generated installer, management plane, egui desktop prototype.
