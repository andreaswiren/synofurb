# User information and instructions (Annex II draft)

Status: **DRAFT; several fields are owner placeholders** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER: name>`
Engineering self-assessment, not legal advice. The list of required items is from the authors' memory of Annex II (**verify**). When finished, this content belongs on the project
website/README and inside the product (About page, `--help`), with the same text in the technical file. Nothing here is a conformity claim.

## 1. Manufacturer and contact (Annex II items 1 and 2)

| Item | Value |
|---|---|
| Manufacturer name / trade name | `<OWNER: legal name>` (if the project is not a manufacturer under the CRA, say who the maintainer is: `<OWNER>`) |
| Postal address | `<OWNER: postal address>` |
| E-mail / web | `<OWNER: contact email>`, <https://github.com/andreaswiren/synofurb> |
| **Single point of contact for vulnerability reports** | `<OWNER: security contact email>` and GitHub private vulnerability reporting (<https://github.com/andreaswiren/synofurb/security/advisories/new>) |
| Coordinated vulnerability disclosure policy | [SECURITY.md](../../SECURITY.md), [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md) |

## 2. Unique identification (item 3)

Name: **synofurb**. Type: software (desktop app, command line, boot agent, boot-flash tool, NAS management plane). Version: `synofurb --version` or `synofurb-cli version`
(workspace version in `Cargo.toml`, currently 0.2.0; the management plane has its own version in `apps/mgmt-plane/VERSION`, shown in its UI and in `GET /api/me`; the boot-flash tool prints its `VERSION`).
Identifying artefacts: the file names `synofurb-<version>-<os>-<arch>.<ext>` and their SHA-256 in the signed `SHA256SUMS` of the GitHub release. Source: tag `v<version>`.

## 3. Intended purpose and security environment (item 4)

**Intended purpose:** to let the owner of a Synology NAS replace DSM with plain Debian 13 (LVM + xfs + mdadm RAID), to manage the NAS boot entries, and afterwards to administer the unit through a web page. The desktop app and CLI run on the
owner's own Windows, macOS or Linux computer; the boot agent and the management plane run on the NAS.

**Security environment the manufacturer assumes:** a home or small-office network the owner controls (no untrusted hosts on the same LAN during an install); physical access to the NAS limited to people the owner trusts;
an administrator who has a **complete, verified backup of all data on the NAS elsewhere**; SSH enabled on the NAS with an administrator account; the NAS management page not reachable from the internet.

**Essential functions:** discover and inspect a NAS (read-only), plan an installation, install Debian (destructive), install the boot agent/boot manager, recover to DSM, update the management plane, self-update the app.
**Security properties** are listed honestly in [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md); in short: SSH host keys are checked before credentials are sent, updates and downloads are hash- and signature-verified,
destructive steps need typed confirmation.

## 4. Known or foreseeable circumstances that may lead to significant cybersecurity risks (item 5)

| Risk | What to do |
|---|---|
| **Data loss.** Reinstalling repartitions the disks. There is **no guarantee that any data survives**, even volumes marked "preserved". Nothing has been tested on real Synology hardware yet; every model except the DS923+ is "Untested". | Make a full, verified backup elsewhere first. Read [docs/RECOVERY.md](../RECOVERY.md). Do not use on a unit holding the only copy of anything. |
| **Unbootable unit.** A failure while the boot flash is edited or while the agent starts can leave the NAS unable to boot DSM. | Keep the boot-flash backup image the app saves on your computer (off the NAS). Know the manual recovery routes in docs/RECOVERY.md before starting. |
| **Root access.** The management plane runs as root and the boot agent gives root on its physical console. Anyone who can log in to the management plane (a member of group `sudo`) administers the whole unit. | Use strong unique passwords; restrict the management port with a firewall or by setting `bind` in `/etc/synofurb/mgmt.json`; do not expose port 8443 or SSH to the internet; set `AGENT_CONSOLE_SHELL=0` in the agent overlay if the console is not physically secure. |
| **Network exposure.** SSH password login is enabled on installed units; no firewall is configured; the unit announces itself with mDNS; the installer's temporary HTTP server runs on your LAN during an install. | Use SSH keys and disable password login (section 7); install a firewall; install only on a network you trust. |
| **Unsigned first releases.** Until the project has published a signing key and signed a release, the updater refuses to install and the installers warn; Windows and macOS binaries may be unsigned and trigger OS warnings. | Compare checksums with the release page; do not run binaries from untrusted copies. |
| **Self-signed certificate** on the management plane. | Compare the fingerprint shown on the NAS login banner (`/etc/motd`) with the browser's before accepting it. |
| **Misuse of flags.** `--insecure-host-key`, `--insecure` (agent downloads without hashes) disable protections. | Do not use them outside a lab. |

## 5. EU declaration of conformity (item 6)

None exists. `<OWNER: internet address, once a declaration has been issued; see EU-DECLARATION-OF-CONFORMITY.template.md>`

## 6. Support and updates (items 7 and 8c)

| Item | Value |
|---|---|
| Type of support | community support through GitHub issues; security fixes as described in SECURITY.md; no paid support |
| **End of the support period** | `<OWNER DECISION D-03: date. The CRA expects at least 5 years unless the product is expected to be used for less.>` |
| Installing security updates | App: `synofurb self-update` (or the update prompt in the GUI) or re-run the one-line installer; see [docs/UPDATES.md](../UPDATES.md). Management plane: `synofurb manage update` from the desktop app or `python3 /opt/synofurb-mgmt/synofurb_mgmt.py update --from DIR` on the unit ([docs/MANAGE.md](../MANAGE.md)). Debian packages on the unit: `unattended-upgrades` runs automatically; `sudo apt update && sudo apt full-upgrade` for manual updates. Kernel updates from backports also rewrite the boot flash; a reboot is needed (the unit does not reboot by itself). |
| Automatic updates | The app does **not** update itself in the background; you start it. The installed Debian applies security updates automatically (default on). To turn this off: create `/etc/apt/apt.conf.d/99-local` containing `APT::Periodic::Unattended-Upgrade "0";` (the later file wins). |
| Current state | **No update can be installed by the app today** (the signing key is a placeholder), see SECURITY.md. |

## 7. Secure configuration (item 8a, 8b)

These are the manufacturer's recommendations; the measures are standard Debian/OpenSSH practice and were not all tested on real hardware.

1. **After installing:** log in over SSH with the account you created, add your public key (`ssh-copy-id`), confirm key login works, then disable password login: create `/etc/ssh/sshd_config.d/05-local.conf` with `PasswordAuthentication no`
   (sshd takes the first value it reads, files are read in alphabetical order, so `05-` beats the shipped `10-synofurb.conf`), then `sudo systemctl reload ssh`.
2. **Management plane:** open `https://<unit>:8443`, compare the certificate fingerprint with the one shown in `/etc/motd` / `/var/lib/synofurb/login-summary.txt`. Restrict who can reach port 8443 with a firewall or bind it to one address in `/etc/synofurb/mgmt.json` (`{"bind": "192.168.1.20"}`), then `sudo systemctl restart synofurb-mgmt`. Only members of group `sudo` can log in; root web login is refused.
3. **Boot agent:** the SSH key in the overlay is per unit and secret; keep the boot-flash backup and the app data directory (`agent_*_ed25519`) private. Set `AGENT_CONSOLE_SHELL=0` ([boot/agent/docs/overlay-contract.md](../../boot/agent/docs/overlay-contract.md)) to remove the console shell.
4. **Do not change** the boot flash by hand while the unit is in use; use `synofurb-dom` / `synofurb-cli recover`.
5. **How changes can affect the security of data:** changing RAID or LVM layout, mounts or the network configuration (the management plane applies network changes with a 90 s automatic rollback), or re-running the installer can destroy data or lock you out.
6. **Initial commissioning:** read [docs/INSTALL.md](../INSTALL.md), [docs/BOOT.md](../BOOT.md), [docs/RECOVERY.md](../RECOVERY.md) first.

## 8. Decommissioning, factory reset and secure data removal (item 8d, Annex I Part I 2(m))

* **Return a NAS to DSM:** `synofurb-cli recover boot-dsm`, `recover remove`, or restore the saved boot-flash image; if DSM's system partition was reclaimed, reinstall DSM with Synology's own tool ([docs/RECOVERY.md](../RECOVERY.md)).
* **Remove synofurb from your computer:** `install.sh --uninstall` / `install.ps1 -Uninstall` ([docs/INSTALL.md](../INSTALL.md)). Delete the app data directory yourself to remove the known-hosts file, logs, the per-unit agent keys and the boot-flash backups.
* **Remove data from the NAS disks:** **the product has no secure-erase function** and does not encrypt volumes. Before selling or discarding a unit, wipe the disks with standard tools (for example `blkdiscard`/`nvme format`, `hdparm --security-erase`, or overwrite with `shred`), from the installed Debian or a live system. These commands were not tested by the project. The boot flash module can be restored to Synology's content from the saved image or wiped separately.
* **Revoke access:** remove the unit's entries from `known_hosts.json` (`synofurb-cli host-key forget --host <ip>`), delete the agent keys, remove users from the management plane.

## 9. Software bill of materials (item 9)

Each release is intended to include `synofurb-<version>-sbom.cdx.json` (CycloneDX 1.5) and `synofurb-<version>-sbom.spdx.json` (SPDX 2.3) and `THIRD-PARTY-LICENSES.txt`, listed in the release's signed `SHA256SUMS`
([SBOM.md](SBOM.md); not yet attached to any release). Licences of bundled components: Inter and JetBrains Mono (SIL OFL 1.1), Lucide (ISC/MIT), Rust crates (mostly MIT/Apache-2.0; see the licence file).

## 10. Integrators

synofurb is not designed to be integrated into other products with digital elements. If you redistribute it, you take on your own obligations; contact the maintainer.
