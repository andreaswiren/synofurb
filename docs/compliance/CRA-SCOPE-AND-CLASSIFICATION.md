# CRA scope and classification

Status: **DRAFT recommendation, needs a lawyer** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Legal reviewer: `<LAWYER: name>`
Engineering self-assessment, not legal advice. Article numbers marked **verify** are from memory. Regulation (EU) 2024/2847 = "CRA".

## 1. Is synofurb a "product with digital elements"? (Art. 2, Art. 3(1))

The CRA covers a software or hardware product, and its remote data processing solutions, whose intended purpose or reasonably foreseeable use includes a direct or indirect logical or
physical data connection to a device or network (Art. 2(1), Art. 3(1)). Software is in scope; components placed on the market separately are in scope too.

| Component | Where | What it is | Network connection | Likely a product with digital elements? |
|---|---|---|---|---|
| Desktop app and CLI | `crates/synofurb`, `crates/synofurb-core` | Windows/macOS/Linux software that scans the LAN, opens SSH sessions to a NAS, serves files to it over HTTP, downloads from Debian and GitHub, updates itself | yes | Yes |
| Boot agent | `boot/agent` | Small Linux (Debian kernel + busybox + dropbear) that lives in the NAS boot flash, reachable over SSH, kexecs other kernels | yes | Yes (firmware-like software) |
| Boot-flash tool | `boot/dom/synofurb-dom` | Shell script that edits the NAS boot flash menu and kernels | indirectly (driven over SSH) | Yes, as a component of the above |
| NAS management plane | `apps/mgmt-plane` | Python HTTPS web UI and API on the NAS (port 8443), runs as root | yes | Yes |
| Installer payload | `payload/`, installers | Config files and scripts copied onto the installed Debian | n/a | Part of the products above, not a separate product |
| Debian system on the NAS | installed from Debian's archive | Debian's own product | yes | Debian is the manufacturer of its packages; synofurb is not (see 3) |

Not a product with digital elements (out of scope as products): the website/GitHub repository itself; Synology DSM (Synology's).

## 2. Free and open-source software and "commercial activity"

The CRA applies to products made available on the market **in the course of a commercial activity** (Art. 2(1) with Art. 3; recitals about free and open-source software:
the authors recall recitals 15 and 18 to 19, **verify the recital numbers**). Software that is supplied free of charge and not monetised is described as not being a commercial
activity; the Regulation lists monetisation examples (a price, technical support as a service, a platform through which the manufacturer monetises other services, requiring
personal data for purposes other than security/compatibility). Accepting donations without making a profit is described as not enough on its own.

Facts for synofurb (all from the repository; the owner must confirm the current and planned ones):

| Fact | Evidence | Points to |
|---|---|---|
| MIT licence, free download from GitHub Releases | `LICENSE`, `docs/INSTALL.md` | non-commercial |
| No price, no paid support, no paid tier in the repository | none found | non-commercial |
| No telemetry, no accounts, no personal data collected by the project | grep of `crates/` found none | non-commercial |
| Donation / sponsor links, employer, company behind the project | `<OWNER: state>` | open |
| The project is run by `<OWNER: individual / company / association>`; if a company releases it as part of its business, "commercial activity" is likely | `<OWNER>` | open |
| It is a tool for owners of Synology hardware (a commercial product), but it does not sell or bundle that hardware | n/a | neutral |

**Engineering reading:** a hobbyist maintainer who publishes free, un-monetised MIT software is probably outside the CRA. The moment synofurb is offered by a business, bundled in a paid
service or offered with paid support, it is probably a product with digital elements and the maintainer is a "manufacturer" (Art. 3, Art. 13). The documents in this directory are written so that
the project can take either path. **Decision for the owner and lawyer: D-01 in [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md).**

### Open-source software steward (Art. 24, **verify**)

If the project is not a manufacturer but "systematically provides support" to development of FOSS intended for commercial activities by others, it may be an **open-source software
steward**, a lighter role: put in place a verifiable cybersecurity policy that fosters a secure product and effective vulnerability handling, cooperate with market surveillance authorities,
and report actively exploited vulnerabilities and severe incidents (Art. 14 applies to stewards to the extent they are involved in development). The `SECURITY.md` and
[VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md) drafts serve that role too. A single-maintainer GitHub project that has no foundation and no downstream commercial users may be neither; ask the lawyer.

## 3. Roles and separate products

* **Manufacturer (Art. 3, Art. 13):** whoever develops or has developed the product and markets it under their name. If the project is in scope, `<OWNER: legal name and postal address>` is the manufacturer
  of the desktop app, CLI, boot agent, boot-flash tool and management plane. The recommendation is to treat them as **one product family released together** (one version, one SBOM, one signed `SHA256SUMS`),
  with the management plane and boot agent documented as separately updatable components (the management plane has its own version, `apps/mgmt-plane/VERSION`, and its own update path).
  Whether the NAS-side parts are separate "products" matters for classification (section 4) and for the declaration of conformity; ask the lawyer.
* **Components from others:** the Rust crates, Debian packages inside the boot agent and the fonts are third-party components. The manufacturer must exercise due diligence when integrating them (Art. 13, **verify** paragraph (5)):
  keep them up to date, report vulnerabilities found in them upstream, and keep an SBOM. See [SBOM.md](SBOM.md).
* **Debian:** the installed operating system is Debian's product. synofurb selects, installs and configures Debian packages; it does not re-distribute modified Debian binaries on the NAS. It *does* ship unmodified Debian binaries
  (kernel, busybox, dropbear, kexec-tools, e2fsprogs, dosfstools, netcat-traditional) inside the boot agent; for those synofurb is the distributor of someone else's component (copyleft implications: [SBOM.md](SBOM.md)).
* **Importer, distributor, authorised representative (Art. 19 to 21, **verify**):** not applicable as long as the project itself is the only party making the software available. A third party that repackages synofurb (a Linux distribution, a NAS vendor) would take on those roles for its own offering.

## 4. Category: default, important (Annex III class I or II) or critical (Annex IV)

CRA category lists are in Annex III (important) and Annex IV (critical); the Commission's implementing act with the technical descriptions of the categories was adopted in late 2025 (the authors recall Implementing Regulation (EU) 2025/2392; **verify the number and read the descriptions**, they decide the borderline cases below).

Annex III class I includes (**verify against the text**): identity management systems and privileged access management software and hardware, standalone and embedded browsers, password managers, malware scanners, VPNs, **network management systems**, SIEM, **boot managers**, PKI and certificate issuance software, physical and virtual network interfaces, operating systems not in class II, routers/modems/switches, and others. Class II includes hypervisors and container runtimes, firewalls and intrusion detection/prevention systems, and tamper-resistant microprocessors/microcontrollers. Annex IV (critical) covers hardware devices with security boxes, smart-meter gateways and smart cards.

| Component | What argues for a higher category | What argues for default | Recommended working assumption |
|---|---|---|---|
| Desktop app / CLI | Runs with the user's rights, handles SSH credentials, updates itself, **destroys data** on a NAS. None of this is an Annex III function. | Not an identity manager, VPN, browser, OS, network device. | **Default category** |
| Boot agent + boot-flash tool | The product itself calls this **"boot manager mode"** (`docs/BOOT.md`): it edits a boot menu, chooses which kernel boots, and `kexec`s other kernels. Annex III class I lists "boot managers". | It is a recovery/installer mechanism for one family of appliances, not a general bootloader. | **Probably Annex III class I ("boot managers")**. The Commission's technical description decides. |
| Management plane | Runs as root, creates users, sets passwords, manages disks, network and SSH keys, authenticates against system accounts (resembles privileged access management and network management in function, not in the Annex III sense). | A web UI for administering one Linux box; it manages storage and local network settings of that box only. | **Default category, with risk of class I** ("network management systems" or "privileged access management"), lawyer to confirm against the description |
| Whole family released together | A manufacturer must classify the product as a whole; if one core component is class I, the product may be class I. | Components could be placed on the market separately. | **Treat the family as class I until the lawyer says otherwise** |

**What would change the recommendation:** (a) the implementing act's description of "boot manager" being limited to firmware/OS bootloaders that run on every boot (then the agent is default), (b) splitting
the NAS-side parts into a separate product with its own declaration, (c) the management plane ceasing to administer users and disks, (d) a decision that the project is not in scope at all (section 2).
**Why it matters:** the conformity route (section 6) and whether a notified body is needed depend on it.

## 5. Dates (Art. 71; all **verify**)

| Date | What starts | Consequence for synofurb |
|---|---|---|
| 10 December 2024 | Entry into force (20 days after publication on 20 November 2024) | none |
| 11 June 2026 | Chapter IV (notification of conformity assessment bodies, Art. 35 to 51) | passed: notified bodies can now be designated; none is needed for the default category |
| 11 September 2026 | **Art. 14 reporting obligations** (actively exploited vulnerabilities, severe incidents) | **already passed (today is 2 November 2026).** If the project is a manufacturer (or steward) and has a product on the market, it must be able to report to the CSIRT/ENISA now. No release exists yet, so nothing is on the market; the procedure must be ready before the first public release. |
| 11 December 2027 | Main obligations (Annex I, conformity assessment, CE marking, technical documentation, declaration of conformity) | Everything in [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md) |

Products placed on the market before 11 December 2027 are subject to the main obligations only if substantially modified afterwards (transitional rule, Art. 69, **verify**), but the Art. 14 reporting duty is understood to
apply to products placed on the market before as well. A version-0.x project that releases frequently will be "placed on the market" anew with each release, so plan to comply from the first public release.

## 6. Conformity assessment route (Art. 32, Annex VIII; **verify**)

| Category | Allowed routes |
|---|---|
| Default | **Module A, internal control** (Annex VIII Part I): the manufacturer assesses itself, keeps the technical documentation, signs the EU declaration of conformity, affixes the CE marking. No notified body. |
| Important class I | Module A **only if** harmonised standards / common specifications / an EU cybersecurity certification scheme (assurance level at least substantial) covering all Annex I requirements are applied in full; otherwise EU-type examination + conformity to type (Module B + C) or full quality assurance (Module H) by a notified body. **For free and open-source software the CRA allows Module A for class I if the technical documentation is made public** (Art. 32(5), **verify**; this is the route most relevant to an open-source project). |
| Important class II | Module B + C or Module H with a notified body (Module A not allowed). Open-source relief **verify**. |
| Critical | European cybersecurity certification (not relevant here). |

**Harmonised standards:** the standardisation request was issued in 2025 and the European standards (the horizontal "40000-1-x" series on principles for cyber resilience and vulnerability handling, and vertical standards)
were still drafts when this was written; **verify the current state**. Until references are published in the Official Journal there is no presumption of conformity through standards. EN 18031-1/-2/-3 belong to the Radio Equipment Directive, not the CRA. Candidate material that can be used as engineering guidance without any presumption:
ISO/IEC 29147 and 30111 (vulnerability disclosure and handling), IEC 62443-4-1 (secure development lifecycle), ETSI EN 303 645 (consumer IoT baseline, for the properties), ISO/IEC 27034 (application security), NIST SSDF, OWASP ASVS (for the web plane). See [TECHNICAL-FILE.md](TECHNICAL-FILE.md).

**Recommendation:** plan for Module A with public technical documentation (this directory is that start), and have the lawyer decide whether class I applies and whether the open-source relief is available. Budget for a notified body only if the lawyer says class II or class I without the relief.

## 7. Manufacturer obligations (Art. 13) and where they are covered

| Obligation | Where in this repository |
|---|---|
| Design, develop, produce to meet Annex I Part I (Art. 13(1)) | [CRA-ANNEX-I-MAPPING.md](CRA-ANNEX-I-MAPPING.md) |
| Cybersecurity risk assessment, documented, kept up to date (Art. 13(2) to (3)) | [RISK-ASSESSMENT.md](RISK-ASSESSMENT.md) |
| Due diligence for third-party components, including open source (Art. 13(5)) | [SBOM.md](SBOM.md), `deny.toml`, `.github/dependabot.yml` |
| Handle vulnerabilities per Annex I Part II for the support period (Art. 13(6) to (8)) | [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md) |
| Support period of at least 5 years unless the product is expected to be used for less (Art. 13(8)); security updates available at least 10 years or for the support period, whichever is longer (Art. 13(9)) (**verify** paragraphs) | Decision D-03 in [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md) |
| Technical documentation, EU declaration of conformity, CE marking; keep 10 years or the support period (Art. 13(12), 28, 29, 31 to 32) | [TECHNICAL-FILE.md](TECHNICAL-FILE.md), [EU-DECLARATION-OF-CONFORMITY.template.md](EU-DECLARATION-OF-CONFORMITY.template.md) |
| Report actively exploited vulnerabilities and severe incidents (Art. 14) | [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md), section "Art. 14 reporting" |
| Inform users (Art. 13(14) and following, Annex II) | [USER-INFORMATION.md](USER-INFORMATION.md) |
| Identification on the product, contact (Art. 13(15) to (19), **verify**) | [USER-INFORMATION.md](USER-INFORMATION.md); version in `synofurb --version` |
| Cooperate with market surveillance authorities on request | `<OWNER: contact>` |
