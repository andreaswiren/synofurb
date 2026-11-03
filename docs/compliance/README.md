# synofurb and the EU Cyber Resilience Act: compliance documentation

Status: **DRAFT engineering self-assessment** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Legal reviewer: `<LAWYER: name>` | Version assessed: 0.2.0 (unreleased)

> **These documents are an engineering self-assessment written by the project's developers (with AI assistance). They are not legal advice, they claim no conformity with
> Regulation (EU) 2024/2847 (the Cyber Resilience Act, "CRA"), and they do not authorise CE marking.** Whether synofurb falls under the CRA, in which role the
> project acts, which product category applies and which conformity route is allowed are legal questions that need a lawyer (and, for some routes, a notified body).
> Article and Annex references are quoted from the engineers' memory of the Regulation and the public text; anything marked **verify** must be checked against the
> Official Journal text (OJ L, 20.11.2024) before anyone relies on it.

## What is here

| File | Purpose |
|---|---|
| [CRA-SCOPE-AND-CLASSIFICATION.md](CRA-SCOPE-AND-CLASSIFICATION.md) | Is synofurb a "product with digital elements"? Free/open-source and "commercial activity". Components, category (default vs Annex III), dates, conformity route, manufacturer obligations. **Recommendation, needs a lawyer.** |
| [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md) | Annex I Part I (security properties) and Part II (vulnerability handling), line by line, with evidence in the repository and the exact gap. |
| [RISK-ASSESSMENT.md](RISK-ASSESSMENT.md) | The cybersecurity risk assessment (Art. 13(2)): assets, trust boundaries, threats, mitigations, residual risk; includes data loss as a safety risk. |
| [TECHNICAL-FILE.md](TECHNICAL-FILE.md) | Index of the Annex VII technical documentation: what exists, where, what is missing. |
| [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md) | Coordinated disclosure, triage workflow, Art. 14 reporting procedure, dependency monitoring. Root policy: [../../SECURITY.md](../../SECURITY.md). |
| [SBOM.md](SBOM.md) | What the SBOM contains, how it is generated, the exact CI steps to add, limits, copyleft findings. Generators: `scripts/gen-sbom.py`, `scripts/gen-licenses.py`. |
| [USER-INFORMATION.md](USER-INFORMATION.md) | Draft of the Annex II user information (contact, intended purpose, known risks, support end, updates, secure configuration, decommissioning). |
| [EU-DECLARATION-OF-CONFORMITY.template.md](EU-DECLARATION-OF-CONFORMITY.template.md) | **Unsigned draft template** for the EU declaration of conformity. Must not be signed or published until a lawyer confirms the route. |
| [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md) | The prioritised list of what must happen, with owner and lawyer decisions explicit. |

## How to read the status words

| Word | Meaning here |
|---|---|
| **Met** | The repository contains a mechanism that does what the requirement asks, for the components named, and a test or document shows it. Not "audited", not "certified". |
| **Partly met** | A mechanism exists but is incomplete, off by default, unverified on real hardware, or covers only some components. The gap column says exactly what is missing. |
| **Gap** | Nothing in the repository meets the requirement (yet). |
| **N/A / decision** | Depends on a legal or owner decision, or the requirement does not apply to the component. |
| **verify** | A legal or numerical detail that the authors are not certain of. |

## The honest summary

**Where the project is strong**

* Careful engineering of the dangerous parts: host-key pinning before any password is sent, a signature-verifying updater that refuses to install anything it cannot verify, typed
  confirmations before destructive actions, secrets over stdin rather than argv, strict input validators in the management plane, fail-safe boot design with a watchdog and documented recovery.
* A sizeable automated test base (static count: about 530 `#[test]` functions in the Rust crates, 258 management-plane tests that pass, 41 release-gate tests, installer tests against a fake release server, QEMU boot smoke tests for the agent).
* Releases are designed to carry `SHA256SUMS` and a minisign signature, with pinned GitHub Actions and least-privilege workflow permissions.

**Where it is not (yet) compliant: the gaps that matter most**

1. **There is no signed release and no signing key.** The updater's public key is a placeholder, so by design every in-app install is refused; the installers contain the placeholder too.
   Windows (Authenticode) and macOS (notarisation) signing are optional and unset. (Annex I Part I 2(f), Part II 7.)
2. **No real-hardware testing.** Nothing has run on a real Synology unit; every compatibility entry except one third-party report is "Untested". For a product that repartitions disks,
   this is the biggest safety and availability gap. (Annex I Part I 1, 2(h); risk assessment R-01.)
3. **No vulnerability-management process was running until this documentation:** no `cargo audit`/`cargo-deny` in CI, no Dependabot, no SECURITY.md, no advisory channel, no
   contact point. Drafts and configuration now exist (`SECURITY.md`, `deny.toml`, `.github/dependabot.yml`) but are not enabled or wired into CI. Whether the dependency set has known
   vulnerabilities **is unknown**: no scanner was run. (Annex I Part I 2(a), Part II 1 to 6.)
4. **SBOM:** the generator and tests exist, but nothing publishes the SBOM yet, and the boot-agent Debian package versions are only known if the build manifest is passed in
   (the release workflow does not upload it yet). 79 of 920 shipped Rust crates ship no licence file in their published source. (Part II 1.)
5. **Weak defaults in places:** the boot agent gives a root shell on a physical console unless `AGENT_CONSOLE_SHELL=0`; SSH password login is enabled on installed units; no host firewall
   is configured; the management plane listens on all interfaces with password-only authentication (no second factor) and a self-signed certificate (optionally behind a HAProxy front with Let's Encrypt certificates: written, not yet run against the real services); no opt-out for logging; no secure-erase feature. (Part I 2(b), (d), (j), (l), (m).)
6. **No independent security testing:** no penetration test, no code audit, no fuzzing. (Part II 3.)
7. **Legal and organisational items nobody has decided:** manufacturer identity and address, the support period, whether the project is a "manufacturer" or an "open-source software steward",
   the product category (a "boot manager" is listed in Annex III class I, see the classification file), the conformity route, the licence-compliance position on GPL components inside the boot agent. These are in [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md) as lawyer/owner decisions.

**What nobody can claim today:** conformity with the CRA, a CE mark, an EU declaration of conformity, or that the product is free of known exploitable vulnerabilities.

## Scope of the assessment

Version 0.2.0 of the monorepo at the commit that introduced these files, on branch `claude/synofurb-debian-reinstall-34mpul`. Components: desktop app (`crates/synofurb`),
engine and updater (`crates/synofurb-core`), command line (`synofurb-cli`), boot agent (`boot/agent`), boot-flash tool (`boot/dom`), NAS management plane (`apps/mgmt-plane`),
installers and payload (`install.sh`, `install.ps1`, `payload/`), release pipeline (`.github/workflows`, `scripts/`). Not assessed: Debian (installed from Debian's archive), Synology DSM,
GitHub as a platform, the third-party crates beyond their licence and version.

## Maintaining these documents

Re-run the generators and re-read the mapping whenever a release is cut; every row's evidence column names files so that it can be re-checked. When a gap is closed, change its
row and `GAPS-AND-ACTIONS.md` in the same commit. Do not change a status to "Met" without a test or a document that a reviewer can open.
