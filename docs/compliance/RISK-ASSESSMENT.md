# Cybersecurity risk assessment (Art. 13(2))

Status: **DRAFT, first version, written by the developers; no independent review** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER/SECURITY: name>`
Assessed: synofurb 0.2.0 (unreleased). Not legal advice; makes no conformity claim. Scales: likelihood L (low, medium, high) x impact I (low, medium, high). "Residual" is the authors' judgement after
the mitigations that exist in the repository today (not after the ones merely planned). It must be reviewed at every release and after every security incident or vulnerability report.

## 1. Intended purpose and foreseeable misuse

Intended purpose ([USER-INFORMATION.md](USER-INFORMATION.md)): a technically capable owner of a Synology NAS uses the desktop app or CLI on their own computer, on a network they trust, to replace DSM with
Debian, to manage boot entries of the NAS boot flash, and afterwards to administer the NAS through a web page. Foreseeable misuse: running it on an unbacked-up unit, on a hostile or shared network, with
the wrong unit selected, skipping host-key checks (`--insecure-host-key`), leaving the management plane open to the internet, leaving a physical console accessible.

## 2. Assets

| ID | Asset | Why it matters |
|---|---|---|
| A1 | The NAS owner's data on the disks | Destroyed by repartitioning if the tool or the user errs. The product erases disks on purpose. |
| A2 | The NAS's ability to boot (boot flash, DSM loader, Debian) | An unbootable NAS is a total loss of availability and may need hardware access. |
| A3 | Credentials: NAS admin SSH password and sudo password (in memory), per-unit agent private key (on disk), the new user's password hash (in the preseed and boot overlay), management-plane logins | Root on the NAS and every file on it. |
| A4 | The update signing key (minisign secret) and the release integrity | A forged release runs on every user's computer. |
| A5 | The user's computer | The app runs with the user's rights, serves files to the LAN and replaces its own binary. |
| A6 | The LAN | Discovery, beacon and the temporary install server run on it. |
| A7 | Third-party code: Rust crates, Debian packages in the agent, fonts | Inherited vulnerabilities and licence obligations. |

## 3. Trust boundaries

```
 [A] desktop app/CLI --SSH (russh, host-key policy)--> [NAS: DSM or agent or installed Debian]
        |  \--HTTP on the LAN (token path)--> NAS fetches stage2, payload, preseed, installer kernel/initrd   (TB2)
        |--HTTPS--> api.github.com / github.com / *.githubusercontent.com (updater, allow-listed hosts)       (TB3)
        |--HTTPS--> deb.debian.org / security.debian.org / cdimage.debian.org (installer assets, hash chain)  (TB4)
 [NAS boot flash]  menu + kernels edited over SSH by synofurb-dom; read by the NAS firmware/bootloader       (TB5)
 [browser on the LAN] --HTTPS 8443--> management plane (root, password login)                               (TB6)
 [browser] --HTTPS 443--> HAProxy front (optional, docs/CERTIFICATES.md) --loopback--> management plane / add-ons   (TB6)
 management plane --HTTPS--> Let's Encrypt, challenges.addr.tools (certificate issue and renewal; lego as a child process)  (TB3)
 [GitHub Actions + crates.io + Debian archive] --> release artefacts --> GitHub Releases --> users          (TB7)
 [serial/VGA console, USB, the boot flash module itself] physical access                                    (TB8)
```

## 4. Threats

Categories: S spoofing, T tampering, R repudiation, I information disclosure, D denial of service / destruction, E elevation of privilege, plus **Safety** (data loss).

| ID | Threat (category) | Boundary / asset | L | I | Mitigations in the repository | Residual | Open gap (see GAPS-AND-ACTIONS.md) |
|---|---|---|---|---|---|---|---|
| R-01 | The tool destroys data it should not: wrong unit selected, planner bug, a model that behaves differently from the assumptions, interrupted repartitioning (Safety, D) | A1 | M | High | Read-only inspect/plan first; "Keep volumes" mode never includes data partitions in a write command; typed phrase `ERASE <model>` plus two acknowledgements; DOM image backup before the first write; writes to copies with verify and rollback; after-install RAID superblock re-check; `docs/RECOVERY.md` states plainly that no data guarantee exists | **High until tested on real units**; nothing has run on a real Synology | G-02 real-hardware test programme; keep all models "Untested" until reports exist |
| R-02 | Unit left unbootable: corrupted boot flash, agent never boots, power loss mid-write (D) | A2 | M | High | Default boot entry stays DSM; one-shot boots; agent resets default first; watchdog; DOM backup + `synofurb-dom restore`; manual recovery routes incl. plain `dd`; `DEBIAN_OLD` kernel entry; kernel that does not fit is refused | Medium-high | Documented uncovered case "agent never boots"; real-hardware verification (G-02) |
| R-03 | Man in the middle on first SSH connection to the NAS or the agent: password or commands exposed (S, I) | TB1, A3 | L | High | Host key checked before any password or key signature is sent; pin by fingerprint; TOFU with an explicit confirm (180 s) in the GUI; changed key is a hard error (`hostkey.rs`) | Medium: TOFU depends on the user comparing fingerprints; the CLI pins unknown keys silently unless told otherwise; the agent's key is never verified (client key auth only) | Make the CLI ask by default; document fingerprint comparison; consider agent host-key pinning via the overlay |
| R-04 | Malicious or forged update replaces the app (T, E) | TB3, A4, A5 | L | High | `SHA256SUMS` + minisign, host allow-list on every redirect, size caps, asset not downloaded if signature fails, atomic replace + rollback, tests ([docs/UPDATES.md](../UPDATES.md)) | Today: **installs are refused** (placeholder key); after the key exists: stolen key = total compromise | G-01 generate and protect the key (offline, passphrase, backup, rotation plan); sign macOS/Windows binaries |
| R-05 | Compromised CI: malicious third-party action, runner, or pull request builds a backdoored release (T) | TB7, A4 | L | High | `release.yml`: `permissions: {}`, per-job least privilege, actions pinned by SHA, `persist-credentials: false`, `release` environment with required reviewers (if configured), signing in a separate job, tag must equal the Cargo version | Medium | `ci.yml` uses unpinned actions; no provenance attestation; branch protection and tag protection are settings the owner must verify (G-06) |
| R-06 | Vulnerable or malicious Rust crate in the dependency tree (T, E, I) | A7 | M | Med-High | `Cargo.lock` committed and `--locked` builds; checksums recorded in the SBOM; most crates are widely used | **Unknown**: no scan has ever run; the GUI depends on pre-release forks (`gpui-pre-*`, `zed-font-kit`) | G-03 turn on `cargo deny`/Dependabot/advisory alerts and gate releases |
| R-07 | Tampered Debian installer or packages (T, E) | TB4 | L | High | HTTPS; `InRelease` -> installer `SHA256SUMS` -> file; PGP verified when `gpgv` and the Debian keyring are present; boot agent build verifies the same chain and each `.deb`; installed system apt verifies with the Debian keyring and refuses to continue without it | Low-medium: on Windows/macOS the desktop PGP check is usually skipped | Ship the Debian archive keyring fingerprint or make the check mandatory (G-09) |
| R-08 | Attacker on the LAN abuses the temporary install server or inserts themselves between app and NAS during an install (T, I, E) | TB2, A3, A1 | L | High | 128-bit random token in the URL path; started only during an install; payload and kernel/initrd SHA-256 checked on the NAS; log ports only for the job | Medium on a hostile LAN: `stage2.sh` and `preseed.cfg` travel over plain HTTP and no hash pin for them was found; preseed contains the password hash | Document "trusted LAN only" prominently; investigate pinning the stage-2 hash through the SSH channel (G-08) |
| R-09 | Physical or console access to the agent gives root (E) | TB8, A3 | L | High | Documented: "physical access equals root anyway"; `AGENT_CONSOLE_SHELL=0` disables | Medium (default is on) | G-07 default off or opt-in |
| R-10 | Brute force or session theft against the management plane (S, E) | TB6, A3 | M | High | Exponential lockout, delay, 256-bit sessions stored hashed, idle/absolute timeouts, CSRF, Origin check, CSP, secure cookies, TLS 1.2+, root web login refused, `sudo` group only | Medium: password-only, listens on all interfaces, no firewall, self-signed cert (users click through warnings unless they compare the fingerprint) | G-07 bind address default, firewall, optional second factor |
| R-11 | Injection into the management plane's privileged commands (T, E) | TB6 | L | High | Argv-only subprocess, fixed PATH, strict validators on every field, allow-listed services, typed confirmations for destructive RAID/LVM/mount actions, 258 tests | Low-medium: 6128 lines of root Python, no external review | G-10 independent review / fuzz the API |
| R-12 | Credential leakage: passwords in logs, process list, files (I) | A3 | L | Med-High | Secrets over stdin (sudo password, chpasswd), log redaction (`add_secret`, `redact`), CLI reads the password from an environment variable instead of argv, agent key `chmod 0600` | Low-medium: the agent private key and the DOM overlay sit unencrypted on disk; key file is created before it is `chmod`ed | Create key files with mode 0600 atomically; document at-rest handling (G-11) |
| R-13 | Unsafe default exposure of an installed unit: SSH password login, no firewall, mDNS advertisement (S, E) | TB6, P | M | Med | Root SSH disabled, `MaxAuthTries 4`, key auth preferred, security updates automatic | Medium | G-07 |
| R-14 | Vulnerable software on the unit after install is not patched (E) | P, A7 | M | Med-High | `unattended-upgrades` on; mgmt plane reports pending updates; app `manage update` | Medium | Update notification in the UI; support period (D-03) |
| R-15 | Silent denial of service on the LAN by scan or beacon (D) | A6 | L | Low | One UDP broadcast every 5 s from the agent only; discovery is mDNS and targeted probes | Low | Measure and document |
| R-16 | Licence or copyleft breach through bundled GPL components in the boot agent (legal) | A7 | M | Med | Components unmodified from Debian; SBOM lists them; THIRD-PARTY-LICENSES points to Debian sources | Medium | D-05 lawyer decision on the source-offer mechanism |
| R-17 | Release assets unsigned for Windows/macOS: users run a tampered copy believed genuine (S, T) | TB3, A5 | M | Med | SHA256SUMS + minisign for the updater and installers; honest warnings in `docs/RELEASING.md` | Medium until Authenticode/notarisation exist | G-01 |
| R-18 | Insecure recovery documentation leads users to overwrite the wrong device with `dd` (Safety) | A1 | L | High | Instructions say "check twice: the DOM is the 128 MB device"; `sha256sum -c` before writing; `lsblk` listing | Medium | Add a stronger warning/test; real-hardware walkthrough |
| R-19 | Certificates and the optional HAProxy front: a wrong front configuration locks the administrator out; the challenge secret or a private key leaks; a hostile name or value reaches lego or the HAProxy configuration (D, I, E) | TB3, TB6 | M | Med | Default off; `haproxy -c` on a private copy before anything is written, snapshot and a revert timer unless confirmed, default backend is the management plane, `proxy disable` from the console; secret and keys 0600, scrubbed from logs and API answers; strict name validation, no shell, golden and injection tests. **Open:** never run against the real Let's Encrypt, lego or HAProxy; the front's loopback PROXY port (127.0.0.1:8444) can be reached by local users; add-on ports stay open on the LAN |

## 5. Not modelled (state of the art for later versions)

Side channels on the NAS, a malicious NAS operating system attacking the app (the app treats DSM output as untrusted text only; not systematically tested), denial of service by a hostile DHCP/mDNS responder against the scan, attacks on the Debian installer's own network stack, and the OS-level installers (MSI, NSIS, DMG) beyond signing.

## 6. Review log

| Date | Reviewer | Result |
|---|---|---|
| 2026-11-02 | the developers (with AI assistance) | first version, no independent review |
