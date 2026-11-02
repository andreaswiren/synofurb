# EU declaration of conformity: DRAFT TEMPLATE, NOT SIGNED, NOT VALID

> **THIS IS A TEMPLATE. IT IS NOT A DECLARATION. DO NOT SIGN, DO NOT PUBLISH, DO NOT ATTACH TO A RELEASE.**
> synofurb has **not** been assessed as conforming with Regulation (EU) 2024/2847. The gaps in [GAPS-AND-ACTIONS.md](GAPS-AND-ACTIONS.md) are open, the conformity route is undecided
> (decision D-02), and it is not even settled whether the project is in scope (D-01). Signing a declaration without the underlying assessment would be a false statement. A lawyer must
> approve the final text. The CE marking may not be affixed on the basis of this file.

Status: DRAFT TEMPLATE | Date: 2026-11-02 | Owner: `<OWNER: name>` | Legal reviewer: `<LAWYER: name>`

The structure follows the content list the authors remember for the EU declaration of conformity (Annex V of the Regulation; the simplified declaration is Annex VI. The owner's brief called the
declaration "Annex VI": **verify the annex numbers and the exact required content in the Official Journal text** before using this).

---

## EU declaration of conformity (Regulation (EU) 2024/2847)

1. **Product with digital elements (name, type, version, allowing identification and traceability):**
   synofurb `<OWNER: 0.2.0 or the released version>` (desktop app, command line `synofurb-cli`, boot agent, boot-flash tool `synofurb-dom`, NAS management plane `synofurb-mgmt <version>`)
   Release tag `<vX.Y.Z>`, SHA-256 of the artefacts: see `SHA256SUMS` (signed with minisign) of that release.
   `<LAWYER: one declaration for the whole family, or one per product/component (see CRA-SCOPE-AND-CLASSIFICATION.md section 3)?>`

2. **Name and address of the manufacturer (or authorised representative):**
   `<OWNER: legal name>`, `<OWNER: postal address>`, `<OWNER: contact point, security contact email>`

3. **This declaration of conformity is issued under the sole responsibility of the manufacturer.**

4. **Object of the declaration:** the product described in point 1 (SBOM: `synofurb-<version>-sbom.cdx.json` / `.spdx.json` attached to the release; technical documentation: `docs/compliance/` and the repository at the release tag).

5. **The object of the declaration described above is in conformity with:** Regulation (EU) 2024/2847 of the European Parliament and of the Council of 23 October 2024 on horizontal cybersecurity requirements
   for products with digital elements (the Cyber Resilience Act). `<LAWYER: other Union harmonisation legislation that applies to the product, if any; the authors know of none>`

6. **References to harmonised standards, common specifications or cybersecurity certification schemes used:** `<none. Fill in only OJ-listed references that were applied in full. As of 2026-11-02 the authors know of none that were applied; do not invent any.>`

7. **Notified body / conformity assessment procedure:** `<ROUTE DECISION D-02: "Conformity assessment procedure: internal control (Annex VIII Part I, Module A)" if allowed; otherwise name, number and certificate of the notified body>`

8. **Additional information:**
   Support period: until `<OWNER DECISION D-03: date>`.
   Signed for and on behalf of: `<OWNER: legal name>`
   Place and date of issue: `<place>`, `<date>`
   Name, function: `<name, function>`
   Signature: `<NOT SIGNED>`

---

## Preconditions before anyone signs this (checklist)

| # | Precondition | State on 2026-11-02 |
|---|---|---|
| 1 | The project is a manufacturer in the CRA sense (D-01) | undecided |
| 2 | Product category and conformity route confirmed by a lawyer (D-02) | undecided |
| 3 | Every Annex I requirement is met or has a documented, justified exception | **no**: see CRA-ANNEX-I-MAPPING.md |
| 4 | Technical documentation complete (Annex VII) | **no**: see TECHNICAL-FILE.md |
| 5 | Cybersecurity risk assessment reviewed by someone independent | no |
| 6 | Signed releases with a real key; vulnerability handling running; SBOM published | no |
| 7 | Manufacturer name, address, contact point filled in (D-04) | no |
| 8 | Support period decided (D-03) | no |
| 9 | Declaration reviewed by a lawyer; where to publish it (Annex II item 6: the internet address where users can access it) decided | no |

When all are true, the declaration must be re-dated, re-issued for every substantial modification, and kept with the technical documentation for the retention period (Art. 13, **verify**).
