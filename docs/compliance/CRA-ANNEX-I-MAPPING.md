# Annex I mapping: essential cybersecurity requirements

Status: **DRAFT engineering self-assessment, not a conformity claim, not legal advice** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER: name>`
Assessed: synofurb 0.2.0 (unreleased). Annex I wording is paraphrased from the Regulation text as the authors remember it (**verify** against the Official Journal).
Evidence paths are relative to the repository root. "Met" never means audited or tested on real hardware (nothing has run on a real Synology; README, `docs/RECOVERY.md`).

Component codes: **A** desktop app and CLI (`crates/`), **U** in-app updater and installers, **B** boot agent and boot-flash tool (`boot/`), **M** NAS management plane (`apps/mgmt-plane`),
**P** installed system and payload (`payload/`, Debian), **R** release pipeline (`.github/workflows`, `scripts/`).

## Part I: security properties of the product

### Part I (1): appropriate level of cybersecurity based on the risks

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | Risk assessment written for this purpose: [RISK-ASSESSMENT.md](RISK-ASSESSMENT.md). Design rules in code comments and docs: `apps/mgmt-plane/synofurb_mgmt.py` header (no `shell=True`, validators, stdin secrets), `boot/agent/docs/safety-model.md`, `docs/RECOVERY.md`. | The assessment is new (2026-11-02), self-made, not reviewed by anyone else, and was not used to drive the design that already exists. No real-hardware validation of the safety claims. |

### Part I (2)(a): no known exploitable vulnerabilities when placed on the market

| Status | Evidence | Gap |
|---|---|---|
| **Gap** | Tooling prepared but not run in CI: `deny.toml` (advisories, licences, sources), `.github/dependabot.yml`, SBOM with 953 Cargo packages and hashes ([SBOM.md](SBOM.md)). `cargo clippy -D warnings` and `cargo test` run in `.github/workflows/ci.yml`. | **No vulnerability scan has ever been run on the dependency set** (neither `cargo audit` nor `cargo deny` was available when this was written, so the result is unknown, not "clean"). The GUI depends on pre-release forks (`gpui-pre-*` 0.3.10, `zed-font-kit` 0.14.1-zed) with an uncertain upstream security process. The boot agent takes whatever Debian ships at build time with no vulnerability check. Debian packages installed on the NAS are Debian's responsibility but synofurb chooses the release (trixie, optionally trixie-backports kernel). Gate needed: a release must fail if `cargo deny check advisories` fails. |

### Part I (2)(b): secure by default configuration; ability to reset to original state

| Component | Status | Evidence | Gap |
|---|---|---|---|
| A | **Met** | The app has no network listener of its own except the temporary install server on the LAN (`crates/synofurb-core/src/server.rs`), random 128-bit token path, started only during an install; destructive actions need typed confirmation (`session.rs` `confirm_phrase`, "ERASE <model>"), two checkboxes and a backup acknowledgement; host-key checking on by default (`hostkey.rs`). | The temporary server speaks plain HTTP (see (e)). |
| B | **Partly met** | Fail-closed credentials (no key and no password hash = nobody can log in), one-time password revoked after first use, downloads need SHA-256, watchdog reboots to DSM (`boot/agent/docs/safety-model.md`). | Default `AGENT_CONSOLE_SHELL=1`: a root shell without login on the serial/VGA console (`boot/agent/rootfs/usr/lib/synofurb/lib.sh:48`, documented as "physical access = root"). The default is insecure by the CRA's reading; make it opt-in or require the overlay to say so. |
| M | **Partly met** | Root web login disabled (`ShadowAuth.authenticate`), only members of group `sudo`, throttling, TLS 1.2+ only, HttpOnly/SameSite=Strict/Secure cookie, CSRF token, Origin check, CSP, `X-Frame-Options: DENY`, HSTS, `Cache-Control: no-store`; systemd hardening that does not break disk management (`synofurb-mgmt.service`). | Listens on `::` (all interfaces) by default (`Config.bind`), port 8443, with no firewall configured by the payload; password-only authentication; self-signed certificate with a 10-year validity (`ensure_tls`); runs as root without `ProtectSystem` etc. (a documented trade-off in the unit file). An optional HAProxy front on 443 with Let's Encrypt certificates (`docs/CERTIFICATES.md`, default off, written but not yet run against the real services) moves the management plane to loopback while it is on; a front change that is not confirmed within two minutes reverts itself; the challenge secret is a root-only file. |
| P | **Partly met** | `PermitRootLogin no`, `PubkeyAuthentication yes`, `MaxAuthTries 4`, `X11Forwarding no`, root account locked (`payload/files/etc/ssh/sshd_config.d/10-synofurb.conf`, `payload/chroot-setup.sh`). | **`PasswordAuthentication yes` is the shipped default** (a documented usability choice: "so you can always get in"). No host firewall installed or configured. `avahi-daemon` advertises the unit on the LAN (`payload/files/etc/avahi/services/synofurb.service`). |
| All | **Partly met** | Reset to original state = restore the saved boot-flash image / boot DSM again (`synofurb-cli recover`, `docs/RECOVERY.md`); uninstall of the app (`install.sh --uninstall`). | No "factory reset" of the management plane or of an installed unit short of reinstalling; after the point of no return DSM's system partition is gone and Synology's own reinstall is needed (documented). |

### Part I (2)(c): security updates, automatic by default with opt-out, notification, postponement

| Component | Status | Evidence | Gap |
|---|---|---|---|
| U / A | **Partly met** | Update check against GitHub Releases, channel `stable`/`prerelease`, minisign + SHA-256 verification, atomic replace with `.old` rollback (`crates/synofurb-core/src/updater.rs`, `docs/UPDATES.md`); the GUI worker and `synofurb self-update` call the updater (`crates/synofurb/src/ui/worker.rs`, `update_cmd.rs`). | **Installs are refused today: `UPDATE_PUBLIC_KEY` is a placeholder** (`updater.rs:41`, test `shipped_key_is_still_the_placeholder_and_that_blocks_updates`). Updating is manual and "no background polling" (`docs/UPDATES.md`, Limits): there is no automatic update, no default-on setting and so no opt-out to offer. A distro-package install (`.deb`/`.rpm`) needs `pkexec` or a manual command. |
| M | **Partly met** | Update path pushes a verified bundle from the desktop app (`python3 synofurb_mgmt.py update --from DIR`, atomic swap, health check, automatic rollback; `apps/mgmt-plane/README.md`); `health --json` reports pending apt/security updates. | Manual. The bundle carries only a SHA-256 manifest (`MANIFEST.sha256`), no signature: its integrity relies on the SSH channel and on the app it came from. |
| P | **Met** (for Debian packages) | `unattended-upgrades` installed and enabled by default, security updates, `Automatic-Reboot "false"` (`payload/files/etc/apt/apt.conf.d/52synofurb-unattended`). Debian packages are PGP-verified by apt in the target. | Opt-out is "edit an apt config file" (documented in [USER-INFORMATION.md](USER-INFORMATION.md)), not a clear and easy switch in the UI. Kernel updates from `trixie-backports` rewrite the small boot flash (hook `synofurb-dom-update`); a kernel that does not fit is refused and the previous entry kept. No notification of pending updates in the web UI beyond `health`. |
| B | **N/A** | The agent is a RAM-only, per-install image rebuilt from the release; it is replaced, not updated. | The agent's own Debian components cannot be patched in place; a new release is needed. |

### Part I (2)(d): protection from unauthorised access; authentication; report unauthorised access

| Component | Status | Evidence | Gap |
|---|---|---|---|
| A | **Partly met** | SSH host-key verification before any password or key signature is produced: pinned fingerprint, trust-on-first-use with a human confirm callback and a 180 s timeout, hard error on a changed key (`crates/synofurb-core/src/hostkey.rs`, tests in the same file); sudo password via stdin only and redacted from logs (`remote.rs` test `privileged_uses_sudo_with_password_on_stdin_only`, `logs.rs` `add_secret`). | The app itself has no user authentication (it is a local desktop program running as the user; acceptable but note it). The CLI without a confirm callback pins an unknown key automatically (`accept-new` behaviour) and `--insecure-host-key` disables checking. The per-unit agent client key is written unencrypted to the app data directory and `chmod 0600` **after** creation (`session.rs` `agent_key_for`; no restrictive creation mode, no effect on Windows). The SSH password is not persisted by the app (no password field in `crates/synofurb/src/ui/settings.rs`; it is held in memory for the job, and in the CLI it comes from the `SYNOFURB_PASSWORD` environment variable, readable by the same user's other processes). No test asserts that nothing is persisted. |
| B | **Partly met** | Per-unit ed25519 client key baked into the overlay; dropbear password auth only with a one-time hash revoked after first use; no unauthenticated service except SSH and the send-only beacon (`boot/agent/docs/safety-model.md`, `overlay-contract.md`); random host key per boot (the app therefore uses `HostKeyPolicy::KeyAuthOnly`). | Console root shell (see (b)). The overlay holds the key/hash in clear in the boot flash ("treat the DOM contents as confidential"). The agent's host key is not verified by the client, so a man in the middle on the LAN can impersonate the agent to the app (no password is sent, but commands and the payload offered to it could be intercepted or altered; mitigated by SHA-256 pinning of what it downloads). |
| M | **Partly met** | Authentication against the system's shadow file with timing equalisation for unknown users, root and non-`sudo` members refused, expired accounts refused, exponential lockout after 5 failures (5 min doubling to 1 h) and a 0.4 s delay, server-side sessions with 256-bit ids (only the hash is stored), 30 min idle / 12 h absolute timeout, max 64 sessions, CSRF token with constant-time compare, Origin check, audit log of logins (`synofurb_mgmt.py`: `ShadowAuth`, `LoginThrottle`, `SessionStore`, `AuditLog`, `api_login`; tests `test_bad_login_and_lockout`, `test_csrf_enforced`, `test_origin_mismatch_rejected`, `test_session_idle_and_absolute_expiry`). | **No second factor**, no passkeys, no password-strength policy beyond 8 to 128 characters (`valid_password`), no notification of failed logins to anyone, throttle state is in memory (lost on restart), password reset only via SSH/`passwd`. TOTP/MFA is not implemented (no such code in `synofurb_mgmt.py`). |
| P | **Partly met** | Root locked, key login preferred, `MaxAuthTries 4`, `LoginGraceTime 30`, sudo group for the admin user. | Password SSH login on by default; no fail2ban-style protection; no reporting of unauthorised access except `journalctl`. |

### Part I (2)(e): confidentiality of data (encryption at rest and in transit)

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | In transit: SSH (russh) for everything sent to the NAS; HTTPS to GitHub and Debian with host allow-lists for the updater (`updater.rs`, `docs/UPDATES.md`); the management plane is HTTPS-only with TLS 1.2+ (`make_ssl_context`, the `--dry-run` is the only way to run without TLS: test `test_no_tls_requires_dry_run`). | **The install server is plain HTTP on the LAN** (`server.rs`): the NAS fetches `stage2.sh`, `payload.tar.gz`, `preseed.cfg` and the installer kernel/initrd over HTTP with a 128-bit secret path; the preseed contains the user's password **hash** (SHA-512 crypt). Integrity of the payload is checked on the NAS against a SHA-256 the app gives it (`script.rs`), but the stage-2 script itself and the preseed are trusted on the token alone. At rest: the installer does not encrypt data volumes ("Secure erasure / encrypted volumes: not handled", `docs/RECOVERY.md`); the agent overlay and the per-unit private key are stored unencrypted (file mode only); the management-plane TLS key is generated on the NAS with 0600 in `/var/lib/synofurb-mgmt/tls`; the audit log is 0600. Debian installer PGP verification of `InRelease` on the desktop is "best effort" and only happens if `gpgv` and the Debian keyring exist (`debian.rs`); on Windows/macOS the chain then rests on HTTPS to `deb.debian.org` plus the SHA-256 chain. |

### Part I (2)(f): integrity of data, commands, programs and configuration; report corruptions

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | Updater: `SHA256SUMS` signed with minisign, digest compared while streaming, host allow-list on every redirect, size caps, no path extraction (`updater.rs`, tests `signature_mismatch_stops_before_the_asset_is_downloaded`, `checksum_mismatch_is_rejected_and_nothing_is_left_behind`, `wrong_host_and_unsafe_names_are_rejected_before_any_request`). Installers verify the same chain and refuse tampered or unlisted assets (`tests/install/run-tests.sh`). Boot flash edits: write to a copy, `fsck`, re-read check, `conv=fsync`, read-back checksum, automatic rollback (`boot/dom`, `docs/BOOT.md`). Kernel/initrd uploads and the payload are SHA-256 verified on the NAS. Boot-agent build verifies InRelease (PGP if available) -> `Packages.xz` -> every `.deb` (`boot/agent/build.sh`). Installer apt runs with the Debian keyring and refuses unsigned packages (`payload/chroot-setup.sh:32`). Management-plane update verifies a manifest before running new code, then rolls back on failure. | **No signature is produced yet** (placeholder key; unsigned first releases; installers carry the placeholder too, `install.sh:24`, `install.ps1:81`). Binary signing (Authenticode, Apple notarisation) is optional and unset. No build provenance attestation (`docs/RELEASING.md`, Known limitations); installers from WiX/NSIS/dmg are not bit-for-bit reproducible. `ci.yml` uses unpinned actions (`@v4`, `@stable`) although `release.yml` pins by SHA. The agent's PGP check is a warning, not an error, unless `--strict-pgp` is passed (the release workflow does not pass it). The one-line installers are fetched from the `main` branch over HTTPS (trust in the repository head). The management-plane bundle has no signature (see (c)). Corruption is reported (checksum mismatch errors), not logged centrally. |

### Part I (2)(g): data minimisation

| Status | Evidence | Gap |
|---|---|---|
| **Met** (for what the project controls) | No telemetry, no accounts, no analytics in the code (a repository-wide search found none); the app writes logs and a known-hosts file locally; the compatibility report template tells people not to paste serials, MACs, IPs or passwords (`.github/ISSUE_TEMPLATE/compat-report.yml`); logs redact registered secrets. Update checks send no token (`docs/UPDATES.md`). | A DOM backup image of the NAS boot flash is kept on the user's computer (needed for recovery; may contain unit-specific data). The management plane's diagnostics ("Copy diagnostics") are stated to contain no secrets; not independently checked. |

### Part I (2)(h): availability of essential functions, also after an incident; DoS resilience

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | Fail-safe boot design: default entry DSM, one-shot boots, agent watchdog reboots to DSM, installer failure handlers, manual recovery routes ([docs/RECOVERY.md](../RECOVERY.md), `boot/agent/docs/safety-model.md`); the management plane restarts on failure (`Restart=on-failure`), network changes have a 90 s automatic rollback, RAID/volume operations refuse unsafe states. | **Nothing is verified on real hardware** (synthetic boot-flash images and QEMU only). Documented uncovered case: if the agent never boots, nothing resets the default and a power cycle boots it again. After the point of no return there is no rescue shell/SSH in the installer. No DoS protection for the management plane beyond login throttling and a 64 KiB body limit (`MAX_BODY`); `ThreadingHTTPServer` has no connection limit. |

### Part I (2)(i): limit negative impact on other devices and networks

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | The beacon is a 5-second UDP broadcast on one port, only from the agent; the LAN scan is a discovery protocol (mDNS) and targeted probes (`crates/synofurb-core/src/discovery`); install download uses hash-pinned files. | The scan and beacon traffic have not been measured on a busy network; no rate-limit documentation. The app's temporary HTTP server and log ports listen on the LAN for the duration of an install. |

### Part I (2)(j): limit attack surfaces, including external interfaces

| Component | Status | Evidence | Gap |
|---|---|---|---|
| B | **Met** | Only SSH (dropbear) and a send-only UDP beacon; no `ca-certificates`, no other daemons (`boot/agent/README.md`). | Console shell (see (b)). |
| M | **Partly met** | One port, JSON API behind sessions, strict validators on every external value, argv-only subprocesses with a fixed `PATH` and timeouts, secrets via stdin, allow-listed service names (`synofurb_mgmt.py` header, `docs/MANAGE.md`). | All interfaces by default; the file is 6128 lines of single-file Python running as root, so the surface is large; no external review. |
| P | **Partly met** | Minimal Debian package set with `--no-install-recommends` (`payload/chroot-setup.sh`). | Installed network-facing or convenience packages include `avahi-daemon`, `openssh-server`, `gpm`, `curl`, `wget`; no firewall. |

### Part I (2)(k): exploitation mitigation

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | Rust for the app, CLI and updater (memory-safe; 9 `unsafe` occurrences in `crates/`); `cargo clippy -D warnings`; release profile with LTO and `strip`; mgmt unit sets `NoNewPrivileges`, `RestrictSUIDSGID`, `LockPersonality`, `RestrictRealtime`, `SystemCallArchitectures=native`, `RestrictAddressFamilies`; Python for the management plane (memory-safe language). | The management plane runs as root. No claim is made for the Debian kernel/busybox/dropbear hardening beyond Debian's defaults. Compiler hardening flags (stack protector, PIE, RELRO) are toolchain defaults and were not verified on the produced binaries. No fuzzing of the parsers (`debian.rs`, `offline.rs`, `updater.rs`, `domboot.rs`). |

### Part I (2)(l): security-related logging and monitoring, with opt-out

| Status | Evidence | Gap |
|---|---|---|
| **Partly met** | Management plane: JSON-lines audit log of logins and mutating actions with secret redaction, mode 0600 (`AuditLog`, `redact`; test `test_redact_and_audit`); installed Debian has journald; the app keeps per-run logs with secrets replaced by `***` (`logs.rs`) and a log viewer. | No opt-out for logging in the management plane (and none is needed for privacy: it logs actions, not content, but the CRA wording asks for an opt-out). No log rotation policy for the audit log is documented; no remote log forwarding; no alerting. |

### Part I (2)(m): secure and easy removal of all data and settings

| Status | Evidence | Gap |
|---|---|---|
| **Gap** | Uninstall of the app leaves settings by design (`install.sh --uninstall`, "your settings stay"); the known-hosts file and logs are in the app data directory and can be deleted by hand; `synofurb-cli host-key forget`. | **No secure-erase or factory-reset feature** for a NAS (the product creates and destroys partitions but does not wipe or crypto-erase; "Secure erasure / encrypted volumes: not handled", `docs/RECOVERY.md`). No documented wipe of the per-unit agent key and DOM backups. [USER-INFORMATION.md](USER-INFORMATION.md) describes manual steps with standard tools that were not tested. |

## Part II: vulnerability handling requirements

| # | Requirement (paraphrased) | Status | Evidence | Gap |
|---|---|---|---|---|
| 1 | Identify and document vulnerabilities and components, including an SBOM in a common machine-readable format covering at least the top-level dependencies | **Partly met** | `scripts/gen-sbom.py` (CycloneDX 1.5 + SPDX 2.3; every one of the 953 Cargo packages with version, purl, licence, SHA-256 and the full dependency graph, plus non-Cargo parts), `scripts/gen-licenses.py`, tests `tests/sbom` (29), instructions [SBOM.md](SBOM.md). | The SBOM is **not yet attached to any release** (workflow change pending, see SBOM.md). Boot-agent Debian package versions are only pinned when the build manifest is supplied; the release workflow does not upload it yet. System libraries the GUI links dynamically and the Debian packages installed on the NAS are not in the SBOM. No continuous vulnerability identification (no scanner in CI). |
| 2 | Address and remediate vulnerabilities without delay, with security updates; separate security updates from functionality updates where technically feasible | **Gap** | Process drafted in [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md); update mechanism exists. | No update can be installed today (placeholder key, no release); no triage owner or on-call; releases mix features and fixes (no security-only release line). |
| 3 | Apply effective and regular tests and reviews of the security of the product | **Partly met** | About 530 Rust `#[test]` functions (290 in `synofurb-core`, 240 in `synofurb`), 258 management-plane tests (run and passing on 2026-11-02), 41 release-gate tests, installer tests against a fake release server, QEMU smoke test for the agent, `shellcheck` and clippy in CI, the SBOM tests. Security-relevant tests exist for host keys, the updater, CSRF/login/session logic and secret redaction. | **No external penetration test, no code audit, no fuzzing, no SAST/dependency scan in CI, no tests on real hardware.** Test counts are static greps, not coverage. |
| 4 | After a security update is available, publicly disclose fixed vulnerabilities (description, affected product, impact, severity, remediation) | **Gap** | Channel chosen: GitHub Security Advisories + CVE through GitHub ([../../SECURITY.md](../../SECURITY.md), VULNERABILITY-HANDLING.md). | No advisory has ever been published; private reporting and advisories must be enabled by the owner. |
| 5 | Put in place and enforce a coordinated vulnerability disclosure policy | **Partly met** | [../../SECURITY.md](../../SECURITY.md) (draft). | Draft only; contact address and support period are `<OWNER>` placeholders; "enforce" has no process owner yet. |
| 6 | Facilitate sharing of information about potential vulnerabilities (also in third-party components), provide a contact address for reports | **Gap** | Placeholders in SECURITY.md. | `<OWNER: security contact email>` and private vulnerability reporting are not set up. Upstream-reporting procedure for third-party components is described but untested. |
| 7 | Mechanisms to securely distribute updates, automatic where applicable | **Partly met** | Signed `SHA256SUMS` design, pinned workflow actions, least-privilege permissions, a release job with required reviewers if configured, installers verifying signatures ([docs/RELEASING.md](../RELEASING.md), [docs/UPDATES.md](../UPDATES.md)). | Key not generated; macOS/Windows not signed; no provenance attestation; no automatic updates (see Part I (2)(c)). |
| 8 | Disseminate security patches without delay, free of charge, with advisory messages | **Partly met** | Everything is MIT and free; release notes come from `CHANGELOG.md`; the updater shows notes. | Depends on 2 and 4; advisories do not exist yet. |

## Gaps that block any honest conformity statement

1. No signing key and no signed release (Part I 2(f), Part II 7).
2. No vulnerability scanning of dependencies and boot-agent contents (Part I 2(a), Part II 1).
3. No contact point, advisory channel or process owner (Part II 4 to 6).
4. Weak defaults: console root shell, SSH password login, all-interface management plane, no firewall (Part I 2(b), (d), (j)).
5. No external security testing and no real-hardware testing (Part I (1), 2(h); Part II 3).
6. No secure data removal feature (Part I 2(m)).

Each item has an owner action in [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md).
