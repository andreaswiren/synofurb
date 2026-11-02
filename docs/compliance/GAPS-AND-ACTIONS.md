# Gaps and actions

Status: **DRAFT working list** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER: name>`
Engineering self-assessment, not legal advice. One row per gap, each traceable to a row in [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md) or [RISK-ASSESSMENT.md](RISK-ASSESSMENT.md). Close a gap by changing its row in the same commit as the fix.
Priorities: **P0** before the first public release (even a release candidate is published software; Art. 14 reporting already applies from 11 September 2026 if the project is in scope), **P1** before 11 December 2027 (main CRA obligations), **P2** afterwards or opportunistic.

## A. Decisions only the owner or a lawyer can make

| ID | Decision | Who | Why it blocks | Inputs prepared |
|---|---|---|---|---|
| D-01 | Is the project a manufacturer (commercial activity), an open-source software steward, or outside the CRA? Who is the legal entity, and is anything monetised or planned to be (donations, support, a hosted service, bundling by a company)? | Owner + lawyer | Everything else depends on it. If out of scope the compliance burden drops sharply, but the documents remain good practice. | [CRA-SCOPE-AND-CLASSIFICATION.md](CRA-SCOPE-AND-CLASSIFICATION.md) sections 2 and 3 |
| D-02 | Product category (default, Annex III class I, class II) and conformity route (Module A internal control, with public technical documentation for open source, or notified body). Is the boot agent a "boot manager"? Is the management plane a "network management system" or "privileged access management"? One product family or several? | Lawyer (and possibly a notified body for advice) | Decides whether a third party must assess it and who may sign the declaration. | CRA-SCOPE-AND-CLASSIFICATION.md section 4 and 6; read the Commission's implementing act with the category descriptions |
| D-03 | Support period (CRA expects at least 5 years from placing on the market, unless expected use is shorter) and its end date; how long security updates stay downloadable (10 years or the support period, whichever is longer; **verify**) | Owner (lawyer to confirm) | Required in Annex II (item 7) and SECURITY.md; a promise the owner must be able to keep as a single maintainer. | TECHNICAL-FILE.md section 4 |
| D-04 | Manufacturer identity: legal name, postal address, contact points; member state of main establishment (Art. 14 reporting) | Owner | Placeholders in six documents | USER-INFORMATION.md section 1 |
| D-05 | How GPL/LGPL/MPL obligations are met: source offer for the Debian components in the agent images (kernel, busybox, kexec-tools, e2fsprogs, dosfstools), the static `kexec` if bundled, MPL-2.0 notices for 3 crates, "mere aggregation" of MIT scripts in the same initramfs | Lawyer | Release assets include GPL binaries | SBOM.md section 4 |
| D-06 | Whether to pursue CE marking and a declaration at all, or to restrict distribution/positioning (for example "hobby tool, no commercial use, not for sale") | Owner + lawyer | Cost/benefit of the whole exercise | CRA-SCOPE-AND-CLASSIFICATION.md |

## B. P0: before the first public release

| ID | Action | Owner | Evidence of done | Related |
|---|---|---|---|---|
| G-01 | **Generate the minisign key** (offline, passphrase, backup), put the public key into `updater.rs`, `install.sh`, `apps/mgmt-plane/install.sh`, `install.ps1`, docs; remove the placeholder reminder test; store the secret in the `release` environment; write the key-loss and key-theft procedures; decide on Authenticode and Apple notarisation (or state that Windows/macOS builds are unsigned) | Owner | A signed `SHA256SUMS.minisig` verifies with the embedded key; `synofurb self-update` installs a test release | Part I 2(f), Part II 7, R-04, R-17 |
| G-02 | **Test on real hardware**: at least the DS923+ (the only model with an outside report), plus one Intel and one ARM/non-boot-flash unit if available; record results through the compat template; keep every other model "Untested" | Owner | Hardware test log; compat entries updated | Part I (1), 2(h), R-01, R-02 |
| G-03 | **Turn on vulnerability scanning**: `cargo deny --locked check` in CI and before release (use `deny.toml`), Dependabot version updates (`.github/dependabot.yml`) and Dependabot/secret-scanning alerts in repository settings; read and resolve the first findings (the result today is **unknown**); scan the agent's Debian packages (SBOM -> OSV-Scanner/Grype or the Debian Security Tracker) | Owner / orchestrator for CI | Clean or justified `cargo deny` run; alerts enabled | Part I 2(a), Part II 1, R-06 |
| G-04 | **Contact and advisories**: enable GitHub private vulnerability reporting, create the security mailbox, fill every `<OWNER: ...>` in `SECURITY.md` and VULNERABILITY-HANDLING.md, add a security link to the issue-template config | Owner | The "Report a vulnerability" button works; a test report reaches the owner | Part II 4 to 6 |
| G-05 | **Publish the SBOM with every release**: apply the workflow changes in [SBOM.md](SBOM.md) section 5 (upload the agent manifests, `sbom` job, `checksums` needs), move the three names to `required+=` in `scripts/package-collect.sh`; obtain the licence texts for the 79 crates without a licence file (or accept and document the gap) | Orchestrator / owner | Release has `synofurb-<v>-sbom.cdx.json`, `.spdx.json`, `THIRD-PARTY-LICENSES.txt` in the signed `SHA256SUMS` | Part II 1 |
| G-18 | **Be ready for Art. 14**: register on the ENISA single reporting platform, name the CSIRT, prepare the report templates, do a written dry run | Owner + lawyer | Dry-run record in VULNERABILITY-HANDLING.md section 8 | Art. 14 |
| G-19 | **Fill the Annex II information** with real values and put it where users see it (README, About page, release notes); add the "no data guarantee, back up first" text wherever the install is started (already present in the app and README) | Owner | USER-INFORMATION.md without placeholders | Annex II |

## C. P1: before 11 December 2027

| ID | Action | Owner | Related |
|---|---|---|---|
| G-06 | Pin `ci.yml` actions by SHA (Dependabot keeps them current), verify branch and tag protection and required reviewers on the `release` environment, add build-provenance attestation (`actions/attest-build-provenance`, needs `id-token: write` and `attestations: write` on the signing job) | Owner / orchestrator | Part I 2(f), R-05 |
| G-07 | **Secure defaults**: make `AGENT_CONSOLE_SHELL` default to off or opt-in; default the management plane `bind` to a documented safe value or ship a firewall profile (nftables/ufw) in the payload; decide on `PasswordAuthentication` (key-only after the key is installed); optional second factor for the management plane; reconsider `avahi` advertisement | Owner (design decision) | Part I 2(b), (d), (j); R-09, R-10, R-13 |
| G-08 | Integrity of the install server: pin the stage-2 script and preseed hashes through the SSH channel or serve them over TLS; document "trusted LAN only" on the install screen | Owner | Part I 2(e), R-08 |
| G-09 | Make PGP verification of the Debian `InRelease` mandatory on the desktop (embed the archive keyring fingerprint, verify without needing `gpgv`), and use `--strict-pgp` in the agent build | Owner | Part I 2(e)/(f), R-07 |
| G-10 | **Independent security review**: penetration test or code review of the management plane (root, 6128 lines), updater and installers; fuzz the parsers (`debian.rs`, `offline.rs`, `updater.rs`, `domboot.rs`, the mgmt API) | Owner (budget) | Part II 3, R-11 |
| G-12 | **Secure data removal**: a documented tested procedure (or a feature) for wiping disks and the boot flash and for deleting keys/backups; add to the UI where a unit is released | Owner | Part I 2(m) |
| G-14 | Update behaviour: decide whether the app should check for updates by default with a clear off switch; a notification and opt-out UI for unattended upgrades on the unit; separate security-only releases from feature releases | Owner | Part I 2(c), Part II 2 |
| G-16 | Sign the management-plane update bundle (minisign over `MANIFEST.sha256`) so the NAS verifies it independently of the app | Owner | Part I 2(f) |
| G-17 | Complete the technical file: architecture and data-flow overview, production description, test reports with dates and results (run the full suites in CI and archive the logs), keep it per release | Owner | TECHNICAL-FILE.md |
| G-20 | Security gate in the release workflow (cargo deny, SBOM validation, signature test) in addition to the design gate | Orchestrator | Part II 1 to 3 |

## D. P2: opportunistic

| ID | Action | Related |
|---|---|---|
| G-11 | Create key files with mode 0600 atomically (not `write` then `chmod`), restrict ACLs on Windows, consider encrypting the per-unit key at rest | R-12 |
| G-13 | Audit-log rotation and an opt-out switch for the management plane; optional remote forwarding | Part I 2(l) |
| G-15 | Montserrat is credited in the management-plane About page but no font is bundled (remove the credit or add font + licence); consolidate the duplicate font versions (Inter 4.000/4.001, JetBrains Mono 2.304/2.211); record the Lucide version used for the management-plane sprite | SBOM.md section 7 |
| G-21 | Reproducible builds for the installers (WiX/NSIS/dmg are not bit-for-bit reproducible today) | RELEASING.md |
| G-22 | Review the `gpui-pre-*` / `zed-font-kit` fork dependencies: track upstream, have a plan if they are abandoned | R-06 |

## E. Order of work (suggested)

1. D-01 and D-06 (a conversation with a lawyer decides how much of the rest is needed). 2. G-04 and G-18 (cheap, and the reporting duty may already apply). 3. G-03 (find out what is in the dependency set). 4. G-01 and G-02 (the two things that turn a code base into a releasable product). 5. G-05 (so the first release already carries an SBOM). 6. D-02 to D-05, then the P1 list in the order G-07, G-10, G-06, G-08.
