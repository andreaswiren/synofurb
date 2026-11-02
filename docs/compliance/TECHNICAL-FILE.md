# Technical documentation index (Annex VII)

Status: **DRAFT index; the technical file is incomplete** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER: name>`
Engineering self-assessment, not legal advice. Annex VII item numbering is from memory (**verify**). The CRA expects the manufacturer to keep this documentation, and the
EU declaration of conformity, for at least 10 years or the support period, whichever is longer (Art. 13, **verify** paragraph), and to hand it to market surveillance on request.
Because the project is open source, publishing it in the repository is the natural way to keep it (and the class I open-source route in Art. 32(5) may require it to be public: [CRA-SCOPE-AND-CLASSIFICATION.md](CRA-SCOPE-AND-CLASSIFICATION.md)).

## 1. General description of the product

| Annex VII item | Where | Status |
|---|---|---|
| (a) Intended purpose | [USER-INFORMATION.md](USER-INFORMATION.md) section 3, `README.md` ("What it does") | Partly: needs the owner's wording and the "security environment" statement |
| (b) Software versions affecting compliance | `Cargo.toml` (`[workspace.package] version`), `apps/mgmt-plane/VERSION`, `boot/dom/synofurb-dom` (`VERSION=`), agent version = app version (`scripts/package-agent.sh`), `Cargo.lock` | Met (single sources of truth, release workflow cross-checks them) |
| (c) Hardware photos, markings | Not applicable: software only. The product is used with third-party hardware (Synology NAS); no hardware is sold. | N/A |
| (d) User information and instructions (Annex II) | [USER-INFORMATION.md](USER-INFORMATION.md), `docs/INSTALL.md`, `docs/UPDATES.md`, `docs/RECOVERY.md`, `docs/MANAGE.md`, `docs/BOOT.md` | Draft: manufacturer name, address, contact point and support end date are placeholders |

## 2. Design, development, production and vulnerability handling

| Annex VII item | Where | Status |
|---|---|---|
| (a) Design and development, system architecture | `docs/GUI.md`, `docs/BOOT.md`, `docs/RAID.md`, `docs/OFFLINE.md`, `docs/kexec.md`, `boot/agent/docs/architecture.md`, `boot/agent/docs/safety-model.md`, `boot/agent/docs/overlay-contract.md`, `boot/dom/README.md`, `apps/mgmt-plane/README.md`, the module comments in `crates/synofurb-core/src/*.rs`; design system under `design/` (UI only) | Partly: no single architecture overview with data-flow diagrams and trust boundaries (the risk assessment has a first one: [RISK-ASSESSMENT.md](RISK-ASSESSMENT.md) section 3) |
| (b) Vulnerability handling: processes, **SBOM**, coordinated disclosure policy, contact address evidence, secure update distribution | [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md), [../../SECURITY.md](../../SECURITY.md), [SBOM.md](SBOM.md), `scripts/gen-sbom.py`, `deny.toml`, `.github/dependabot.yml`, [../UPDATES.md](../UPDATES.md), [../RELEASING.md](../RELEASING.md) | Draft: contact address is a placeholder; SBOM not yet attached to a release; no release has been signed |
| (c) Production and monitoring processes, validation | `docs/RELEASING.md` (release procedure), `.github/workflows/release.yml` and `ci.yml`, `docs/DESIGN-GATE.md` and `scripts/release-gate.py` (UI approval gate, not a security gate), `scripts/package-*.sh` | Partly: production is documented; there is no security gate and no monitoring of deployed instances (the project has no telemetry by design) |

## 3. Cybersecurity risk assessment

[RISK-ASSESSMENT.md](RISK-ASSESSMENT.md) (threats, mitigations, residual risk) and [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md) (how each Annex I Part I requirement is applied or why not). **Status: first draft, no independent review.**

## 4. Support period

Not decided. **Decision D-03** in [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md). The CRA expects at least 5 years unless the product is expected to be used for a shorter time. Inputs for the decision:
the NAS hardware the product targets is typically used for 5 to 10 years; Debian 13 "trixie" is supported by Debian's security team for about 3 years after release and by LTS for about 2 more (`<OWNER: check Debian's published dates>`); the update channel does not work until a key exists.

## 5. Standards, specifications and certification schemes applied

| Item | Status |
|---|---|
| Harmonised standards (OJ references) | **None applied; none claimed.** The CRA standardisation work was still in draft when this was written (**verify** the current state). No presumption of conformity is available. |
| Common specifications / EU cybersecurity certification scheme | None |
| Guidance used as engineering reference only (no claim of conformity) | ISO/IEC 29147 and 30111 (vulnerability disclosure / handling) for the SECURITY.md and triage workflow; IEC 62443-4-1 and NIST SSDF for the development process; OWASP ASVS for the management plane; ETSI EN 303 645 for the baseline properties; CycloneDX 1.5 and SPDX 2.3 for the SBOM; Keep a Changelog and SemVer for releases. (EN 18031-1/-2/-3 apply to the Radio Equipment Directive, not the CRA.) |
| Where standards are not applied: description of the solutions adopted for Annex I | [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md) |

## 6. Test reports

There are no formal test reports. The following automated tests exist; counts were taken on 2026-11-02, "static" = counted in the source, not executed here.

| Suite | How to run | Count | Notes |
|---|---|---|---|
| Rust unit and integration tests | `cargo test --workspace` | about 530 `#[test]` functions (static): 290 in `crates/synofurb-core`, 240 in `crates/synofurb` (including `crates/synofurb/tests/cli.rs`) | Not executed during this assessment (disk and GUI build requirements). CI runs them (`ci.yml`, job `rust`). Security-relevant: `hostkey.rs`, `updater.rs`, `remote.rs`, `domboot.rs`, `recovery.rs` tests |
| Management plane | `PYTHONPATH=apps/mgmt-plane python3 -m unittest discover -s apps/mgmt-plane/tests` | 258 tests, **all passed on 2026-11-02** (41 s) | Includes login/lockout, CSRF, Origin, session expiry, redaction, validators, RAID planning |
| Release gate | `python3 -m unittest discover -s tests/gate` | 41 tests, passed (1 skipped) | UI approval gate logic |
| Installer scripts | `tests/install/run-tests.sh`, `tests/install/test-package-scripts.sh` | shell test harness against a fake release server | Not run during this assessment; covers bad signature, checksum mismatch, hostile archives |
| Boot flash tool | `boot/dom/tests/run-tests.sh` | shell tests on synthetic images | Not run during this assessment |
| Boot agent | `boot/agent/test/check-structure.sh <arch>`, `boot/agent/test/smoke.sh` (QEMU/TCG) | structural checks and boot smoke test | Not run during this assessment; **no real hardware** |
| SBOM generators | `python3 -m unittest discover -s tests/sbom -v` | 29 tests, passed | Fixture workspace plus a run against the real repository (`cargo metadata --offline`) |
| CLI end-to-end | `tests/cli/smoke.sh`, `tests/cli/wizard_pty.py` | scripts | Not run during this assessment |

Not performed: penetration test, security code review by a third party, fuzzing, dependency vulnerability scan, tests on real Synology hardware, accessibility or usability tests of the security-relevant dialogs.

## 7. EU declaration of conformity

[EU-DECLARATION-OF-CONFORMITY.template.md](EU-DECLARATION-OF-CONFORMITY.template.md): **an unsigned template**. No declaration exists and none may be issued until the conformity route is confirmed (D-02) and the P0 gaps are closed.

## 8. SBOM (on request by market surveillance)

[SBOM.md](SBOM.md); generated at release time into `synofurb-<version>-sbom.cdx.json` and `.spdx.json` (not committed).
