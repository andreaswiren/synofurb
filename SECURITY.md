# Security policy

Status: **DRAFT proposal, not yet in force** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER: name>`
Details, internal workflow and the CRA reporting procedure: [docs/compliance/VULNERABILITY-HANDLING.md](docs/compliance/VULNERABILITY-HANDLING.md).
Items marked `<OWNER: ...>` must be filled in or decided by the maintainer before this file is true.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.** Use either channel:

1. **GitHub private vulnerability reporting**: <https://github.com/andreaswiren/synofurb/security/advisories/new>
   (`<OWNER: switch this on under Settings > Advanced Security > Private vulnerability reporting, otherwise this link does not work>`).
2. **E-mail**: `<OWNER: security contact email>` (`<OWNER: optional OpenPGP key fingerprint>`).

Please include: the affected component and version (`synofurb --version`, `synofurb-cli version`, `apps/mgmt-plane/VERSION`, agent version), what an attacker needs
(network position, credentials, physical access), steps or a proof of concept, and the impact you see. Do not include real NAS serial numbers, MAC addresses, public IP
addresses or passwords. If your report is about data being destroyed by a design flaw (the product repartitions disks on purpose), say so: that is a safety issue we treat as
seriously as a security issue.

## What to expect (proposed targets, not guarantees)

| Step | Target |
|---|---|
| Acknowledge your report | within 5 business days |
| First assessment (accepted, need more information, or not a vulnerability) | within 10 business days |
| Fix or mitigation, actively exploited or critical | as fast as possible, target 14 days |
| Fix, high | target 30 days |
| Fix, medium / low | target 90 days / next regular release |
| Coordinated public disclosure | 90 days after your report by default, earlier once a fix is released, later only by agreement |

This is a project run by `<OWNER: individual / organisation>`; the targets depend on maintainer availability. We will tell you if we expect to miss one.
Reporters are credited in the advisory unless they ask not to be. `<LAWYER: safe-harbour wording for good-faith research, if wanted>`.

## Supported versions

| Version | Security fixes |
|---|---|
| latest released minor (currently none: **no release has been published yet**; the code is `0.2.0`) | yes |
| older releases | no, update to the latest (the in-app updater and `synofurb self-update` do this) |

`<OWNER DECISION: support period. The EU Cyber Resilience Act expects at least 5 years from placing a version on the market unless the product is expected to be used for a shorter time; this file proposes "5 years after the first non-release-candidate release of a major version, ending <date>". Not decided.>`

## How fixes and advisories are published

* Fixes ship as a new release with a note in [CHANGELOG.md](CHANGELOG.md). The in-app updater only installs releases whose `SHA256SUMS` is signed with the project minisign key
  (see [docs/UPDATES.md](docs/UPDATES.md)). **The signing key is still a placeholder, so no signed release exists yet.**
* Advisories are published as GitHub Security Advisories on this repository, with a CVE requested through GitHub as CVE Numbering Authority where applicable.
* Every release is intended to carry an SBOM (CycloneDX and SPDX) and `THIRD-PARTY-LICENSES.txt` ([docs/compliance/SBOM.md](docs/compliance/SBOM.md)).

## Scope

In scope: the desktop app and CLI (`crates/`), the in-app updater, the boot agent (`boot/agent`) and boot-flash tool (`boot/dom`), the NAS management plane (`apps/mgmt-plane`),
the installer scripts (`install.sh`, `install.ps1`), the files installed on the NAS (`payload/`) and the release pipeline (`.github/workflows`, `scripts/`).

Out of scope here (report upstream, and tell us if synofurb makes it worse): Debian packages and the Debian installer, Synology DSM, third-party Rust crates, GitHub itself.

## Known limitations you should not report as new findings

They are tracked in [docs/compliance/GAPS-AND-ACTIONS.md](docs/compliance/GAPS-AND-ACTIONS.md): nothing has run on real Synology hardware, the first releases are unsigned
(no Authenticode / Apple notarisation, placeholder update key), the boot agent gives a root shell on a physical console unless `AGENT_CONSOLE_SHELL=0`, and the management plane
has password authentication only (no second factor).
