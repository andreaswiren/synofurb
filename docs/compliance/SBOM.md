# Software bill of materials (SBOM)

Status: **DRAFT; generator and tests exist, nothing is attached to a release yet** | Date: 2026-11-02 | Owner: `<OWNER: name>` | Reviewer: `<LAWYER: name>`
Engineering artefact, not legal advice, no conformity claim. Annex I Part II (1) asks for an SBOM in a commonly used machine-readable format covering at least the top-level dependencies. This one covers the full Cargo graph.

## 1. What is in it

`scripts/gen-sbom.py` writes three files (names are the release asset names):

| File | Content |
|---|---|
| `synofurb-<version>-sbom.cdx.json` | **CycloneDX 1.5** JSON |
| `synofurb-<version>-sbom.spdx.json` | **SPDX 2.3** JSON (same components, `DEPENDS_ON` / `BUILD_DEPENDENCY_OF` / `DEV_DEPENDENCY_OF` relationships) |
| `synofurb-<version>-sbom-report.json` | licence-unknown list, copyleft findings with dependency chains, unresolved versions, counts (workflow artefact; attaching it to the release is optional) |
| `THIRD-PARTY-LICENSES.txt` | from `scripts/gen-licenses.py`: licence texts of the crates that ship in a build, the fonts and icons, and a pointer for the Debian components |

Components (measured 2026-11-02 on this repository with no boot-agent manifest: **975 components**):

| Part | Count | How it is obtained |
|---|---|---|
| Rust workspace members: `synofurb` (application), `synofurb-core` (library); the SBOM's `metadata.component` is the product `synofurb` 0.2.0 (`pkg:github/andreaswiren/synofurb@v0.2.0`) with the version read from `Cargo.toml` | 2 | `cargo metadata --format-version 1 --locked` |
| Other Cargo packages, each with name, version, `pkg:cargo/...` purl, licence (SPDX expression from the crate metadata), crates.io download URL, repository URL and the **SHA-256 from `Cargo.lock`** | 951 (all from crates.io, all with a checksum; none from git or a path) | metadata + `Cargo.lock` |
| Cargo packages by dependency kind (from the resolve's `dep_kinds`): `normal` 922 (includes the 2 members), `build` 27 (scope `excluded`), `dev` 4 (scope `excluded`) | 953 | property `synofurb:dependency-kind` |
| Cargo packages that exist only for some target platforms (cfg dependencies, e.g. Windows or macOS crates) | 402 | property `synofurb:platform-conditional` |
| Management plane `synofurb-mgmt` 0.5.0 (from `apps/mgmt-plane/VERSION`), SHA-256 of `synofurb_mgmt.py`, **imports checked with `ast`: standard library only** (`non-stdlib-imports: none`), runtime `python3` (Debian 13 ships 3.13; CI tests with 3.12; no lower minimum was tested), list of Debian packages it shells out to | 2 | repository files |
| Fonts: Inter 4.000 (OTF, source of the management-plane subsets and the wordmark) and 4.001 (TTF in the GUI), JetBrains Mono 2.304 and 2.211, all SIL OFL 1.1; version read from each font's `name` table, SHA-256 per file | 4 | repository files |
| Lucide icons (ISC; Feather-derived icons MIT), version from `lucide.json` of the `gpui-kit-assets` crate (1.43.0 for 0.8.1) | 1 | local cargo registry |
| Installers (`install.sh`, `install.ps1`, `apps/mgmt-plane/install.sh`), payload (`payload/**`, 12 files), `synofurb-dom` | 3 | file digests |
| `kexec-tools` 2.0.32 source pinned by `scripts/build-kexec.sh` (SHA-256 pinned), scope `optional` | 1 | script constants |
| Boot-agent build inputs (Debian packages `busybox-static`, `dropbear-bin`, `kexec-tools`, `e2fsprogs`, `dosfstools`, `netcat-traditional`, kernel `linux-image-{amd64,arm64,armmp}` and, **when a manifest is supplied**, every package the build pulled in) | 9 placeholders without a manifest | `boot/agent/build.sh` `MANIFEST-<arch>.json` |
| Build tools (excluded from the product): `cargo-packager` 0.11.8, the Rust toolchain (minimum 1.85) | 2 | workflow file, `Cargo.toml` |

**Boot-agent versions.** `boot/agent/build.sh` resolves the newest Debian version of each package at build time (suite `trixie-backports` by default for the kernel, 7.2.x at the time of writing) and writes the exact
names, versions, SHA-256 and source URLs to `MANIFEST-<arch>.json`. If the manifests are passed with `--agent-manifest`, the SBOM lists the exact `pkg:deb/debian/<name>@<version>?arch=<arch>&distro=debian-13` components
with hashes. Without them the agent components appear as version `UNRESOLVED` with the resolution rule in the properties, and `--require-agent-manifest` makes that an error. **The manifests are currently produced but not preserved by the
release workflow** (`scripts/package-agent.sh` copies only the kernel and initrd), so a release built today would have unresolved agent versions: workflow change 1 below fixes that.

**Runtime-downloaded inputs are references, not components** (top-level `externalReferences` in the CycloneDX file): the Debian archive (installer netboot kernel/initrd, packages for the install and for the offline bundle),
Debian packages the management plane installs on the unit on demand (`docker.io`, `docker-cli`, `docker-compose` for add-ons; `lego` and `haproxy` for certificates and the front; all from the configured Debian mirror, not redistributed), Debian security and firmware archives, the GitHub release API used by the updater, the kexec-tools tarball, and Synology DSM on the target unit (not redistributed).

**Determinism.** Components, edges and properties are sorted; there is no wall-clock time. `--timestamp` (or `SOURCE_DATE_EPOCH`) is the only time recorded; without it CycloneDX has no timestamp and SPDX uses the required-field placeholder
`1970-01-01T00:00:00Z` and says so in its comment. `serialNumber` and the SPDX namespace are `uuid5` of the document content. Two runs with the same inputs are byte-identical (tested).

## 2. Generating and checking

```sh
python3 scripts/gen-sbom.py --out dist/sbom --timestamp 2026-11-02T00:00:00Z --commit "$(git rev-parse HEAD)" \
        [--agent-manifest dist/agent-manifests --require-agent-manifest] [--strict] [--offline]
python3 scripts/gen-sbom.py --check dist/sbom/synofurb-0.2.0-sbom.cdx.json dist/sbom/synofurb-0.2.0-sbom.spdx.json [--cargo-lock Cargo.lock]
python3 scripts/gen-licenses.py --out dist/sbom/THIRD-PARTY-LICENSES.txt [--strict]
python3 -m unittest discover -s tests/sbom -v          # 30 tests; the last class runs against this repository (needs cargo + the registry cache; SBOM_SKIP_REAL=1 skips it)
```

`--check` is the "schema-lite" validator (it is also run automatically on everything the generator writes): required document fields and RFC 3339 timestamp, unique `bom-ref`s/`SPDXID`s,
every `dependsOn`/relationship target exists, every component has a name, version, valid type/scope/hashes, a well-formed SPDX licence expression (or is explicitly marked `synofurb:licence-unknown`). It is **not** a full JSON-Schema validation; run a
full validator (for example `cyclonedx-cli validate`, or the SPDX tools-java validator) in addition if the owner wants one: none was installed where this was written.
`--cargo-lock` additionally cross-checks that every crates.io component's SHA-256 equals the `Cargo.lock` checksum and that no lock entry is missing.

Exit codes: 0 fine, 1 a validation problem (or `--strict` and a gap), 2 usage. `--strict` also fails on unknown licences, unrecognised SPDX ids and unresolved versions; the release should start without `--strict` and add it as the gaps close.
The generators only use the Python standard library; `cargo` must be available (or pass `--metadata FILE` with the output of `cargo metadata --format-version 1 --locked`).

## 3. Findings on the real repository (2026-11-02)

| Measure | Result |
|---|---|
| Components / Cargo packages | 975 / 953 (`Cargo.lock` has 953 `[[package]]`, 951 with a checksum; the 2 without are the workspace crates) |
| **Licence unknown** | **1: `netcat-traditional` (boot-agent placeholder; Debian copyright not read).** Every Cargo package declares an SPDX licence in its metadata. |
| Licences declared by this script from general knowledge, not verified against the packages (`synofurb:licence-verified=false`) | 12 (Debian packages, python3, kexec-tools, cargo-packager, Rust toolchain) |
| SPDX ids not in the script's list | none |
| Shipped crates (kind `normal`) without any licence file in their published source | **79 of 920** (for example `russh` 0.65.0, `gpui-kit` 0.8.1, `dwrote` 0.11.5, `taffy`, `objc2*`, `accesskit*`); they are listed with their repository URLs in Part 4 of `THIRD-PARTY-LICENSES.txt`. The MIT and Apache-2.0 licences require the text to accompany redistribution: someone must fetch the texts from upstream. 841 crates have licence text, 472 distinct texts. |
| Unavoidable copyleft in the Rust dependencies | **3 crates under MPL-2.0, none under GPL/LGPL/AGPL** (details below) |

## 4. Copyleft, and what it means for an MIT distribution

Not legal advice: the reading below is the engineers' and needs a lawyer (decision D-05).

### Rust dependencies

| Crate | Licence | Dependency chain | Where it ends up | Obligation |
|---|---|---|---|---|
| `option-ext` 0.2.0 | **MPL-2.0** | `synofurb-core`/`synofurb` > `directories` 6.0.0 > `dirs-sys` 0.5.0 > `option-ext` | every build, GUI and headless CLI, all platforms | File-level copyleft. Using it unmodified in an MIT binary is fine: ship the licence text, say it is MPL-2.0, and offer the source of the MPL files (unmodified: point at <https://crates.io/crates/option-ext/0.2.0> and its repository). Any modification of those files must be published under MPL-2.0. Does not affect the MIT licence of synofurb's own code. |
| `dwrote` 0.11.5 | **MPL-2.0** | `synofurb` > `gpui-kit` 0.8.1 > `gpui-pre-web` > `gpui-pre-wgpu` > `zed-font-kit` 0.14.1-zed > `dwrote` | Windows builds only (DirectWrite) | as above. The published crate has **no licence file**, so the text has to be added by hand. |
| `cbindgen` 0.28.0 | **MPL-2.0** | `synofurb` > `gpui-kit` > `gpui-pre-platform` > `gpui-pre-ios` > `gpui-pre-apple` > `cbindgen` | build-time tool on Apple targets, not linked into the product | none beyond keeping the dependency licence-clean; scope `excluded` |
| `r-efi` 5.3.0 and 6.0.0 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | via `getrandom` | UEFI targets only, never built for the supported platforms | **avoidable**: choose MIT or Apache-2.0 |
| `self_cell` 1.3.0 | `Apache-2.0 OR GPL-2.0-only` | via `gpui-kit` > ... > `cosmic-text` | some platforms | **avoidable**: choose Apache-2.0 |

`deny.toml` allows MPL-2.0 only for the three crates above, so a new copyleft dependency fails the check until someone decides on it.

### Components other than Cargo

| Component | Licence | Where | Consequence |
|---|---|---|---|
| Debian kernel (`linux-image-*`) | GPL-2.0-only WITH Linux-syscall-note | `synofurb-agent-<arch>-<version>.vmlinuz` (unmodified) | Distribution of GPL binaries: the **corresponding source** must be provided or offered (GPL-2.0 section 3). Debian's source packages are the source; a written offer or a link per version is needed. |
| `busybox-static`, `kexec-tools`, `e2fsprogs` (programs) | GPL-2.0-only (declared, **not verified**) | inside `synofurb-agent-<arch>-<version>.initrd` | same; plus licence texts. |
| `dosfstools` | GPL-3.0-or-later (declared, not verified) | same | same, GPL-3 terms. |
| `libc6` and other libraries | LGPL-2.1-or-later and others | same | notice, source availability, relinking possibility (dynamic libraries in an initramfs: ordinary). |
| `dropbear-bin` | MIT-style (declared, not verified) | same | notice. |
| `synofurb-dom` and the scripts (MIT) are placed in the same initramfs | MIT | same | Aggregation of separate programs in one archive; the GPL does not make the MIT scripts GPL (GPL-2.0 section 2, "mere aggregation"). **Lawyer to confirm.** |
| static `kexec` uploaded to DSM (if the maintainer builds and bundles it) | GPL-2.0-only | `assets/kexec/` (empty in the repository; `scripts/build-kexec.sh` has never been run by the author) | source offer if distributed. |

**Open decisions for the lawyer (D-05):** how the GPL source obligation is met for the agent images attached to a GitHub release (written offer text in the release notes? a `SOURCES` link per package version from the SBOM? mirroring the `.deb` sources?),
and whether the `THIRD-PARTY-LICENSES.txt` pointer is enough. Until decided, the release should not claim the obligation is met.

## 5. Adding it to the release: exact workflow steps

`.github/workflows/release.yml` was not edited by the author of this document. These are the changes to make there (action versions are the ones already pinned in that file; copy the SHAs from the existing steps).

**Change 1: keep the agent manifests.** In job `agent`, after the `package-agent.sh` step (the job already uploads `dist/release/*` as `release-agent-<arch>`), add:

```yaml
      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: agent-manifest-${{ matrix.arch }}          # NOT release-*: it must not land in dist/release
          path: dist/agent-${{ matrix.arch }}/MANIFEST-${{ matrix.arch }}.json
          if-no-files-found: error
          retention-days: 14
```

**Change 2: a new job `sbom`** (read-only, no secrets), placed before `checksums`:

```yaml
  sbom:
    name: SBOM + third-party licences
    needs: [design-gate, meta, test, agent]
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          ref: ${{ env.RELEASE_REF }}
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # master
        with:
          toolchain: stable
      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2
        with:
          pattern: agent-manifest-*
          path: dist/agent-manifests
          merge-multiple: true
      - name: Generate and validate the SBOM, collect licence texts
        env:
          VERSION: ${{ needs.meta.outputs.version }}
        run: |
          set -euo pipefail
          python3 -m unittest discover -s tests/sbom -v
          ts="$(date -u -d "$(git log -1 --format=%cI)" +%Y-%m-%dT%H:%M:%SZ)"
          python3 scripts/gen-sbom.py --out dist/sbom --timestamp "$ts" --commit "$(git rev-parse HEAD)" \
            --agent-manifest dist/agent-manifests --require-agent-manifest
          python3 scripts/gen-sbom.py --check "dist/sbom/synofurb-${VERSION}-sbom.cdx.json" "dist/sbom/synofurb-${VERSION}-sbom.spdx.json" --cargo-lock Cargo.lock
          python3 scripts/gen-licenses.py --out dist/sbom/THIRD-PARTY-LICENSES.txt
          mkdir -p dist/sbom-release
          cp "dist/sbom/synofurb-${VERSION}-sbom.cdx.json" "dist/sbom/synofurb-${VERSION}-sbom.spdx.json" dist/sbom/THIRD-PARTY-LICENSES.txt dist/sbom-release/
      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: release-sbom                                 # release-* is what the checksums job merges into dist/release
          path: dist/sbom-release/*
          if-no-files-found: error
          retention-days: 14
      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: sbom-report                                  # not released; for the maintainer to read
          path: dist/sbom/synofurb-${{ needs.meta.outputs.version }}-sbom-report.json
          retention-days: 14
```

**Change 3: `checksums` job.** Add `sbom` to its `needs:` (`[design-gate, meta, build, cli, agent, static, sbom]`). Nothing else: the existing `download-artifact` step merges `release-*`, so
`release-sbom` lands in `dist/release` and `scripts/package-collect.sh` (already extended with the three names as **optional**; the author of this document changed that script) then includes them in `SHA256SUMS`,
which is signed by the existing minisign step. Once the job is in place, move the three names from `optional+=` to `required+=` in `scripts/package-collect.sh` so a release cannot go out without them (and update `tests/install/test-package-scripts.sh` fixtures to match).

**Change 4 (recommended, not done): a vulnerability gate** in `ci.yml` and before `sbom`: install `cargo-deny` (pinned, `--locked`) and run `cargo deny --locked check advisories sources bans licenses`; see [VULNERABILITY-HANDLING.md](VULNERABILITY-HANDLING.md).

**Release notes line to add:** "SBOM (CycloneDX 1.5 and SPDX 2.3) and THIRD-PARTY-LICENSES.txt are attached and listed in the signed SHA256SUMS." Also the downloaded names are ignored by the in-app updater (they do not match `synofurb-<version>-<os>-<arch>.<ext>`).

## 6. How a downstream consumer verifies it

```sh
gh release download v0.2.0 -p 'SHA256SUMS*' -p 'synofurb-0.2.0-sbom*' -p THIRD-PARTY-LICENSES.txt
minisign -Vm SHA256SUMS -P <public key from docs/RELEASING.md>        # the signature covers the SBOM through the sums
sha256sum -c --ignore-missing SHA256SUMS
python3 scripts/gen-sbom.py --check synofurb-0.2.0-sbom.cdx.json synofurb-0.2.0-sbom.spdx.json --cargo-lock Cargo.lock   # Cargo.lock from the release tag
# optional: regenerate the Rust part and compare; the agent components need the same manifests, which are not published today
```

Feed the CycloneDX or SPDX file to a vulnerability scanner of your choice (OSV-Scanner, Grype, Dependency-Track). The SBOM is a statement by the maintainer; it is not signed on its own (the release signature covers it), and no
SLSA/GitHub build-provenance attestation exists yet.

## 7. Limits

* **Coverage:** system libraries the GUI links dynamically on the user's computer (freetype, fontconfig, Vulkan, X11/Wayland, OpenSSL where used) and the macOS/Windows system frameworks are not components. The Debian packages the installer puts on the NAS (the list in `payload/chroot-setup.sh`, plus dependencies and whatever the Debian installer selects) are not in the SBOM: ask the unit (`dpkg-query`), they are Debian's. Packages fetched at run time are references only.
* **Cargo scope:** all target platforms and default features of the workspace (GUI on). A headless build (`--no-default-features --bin synofurb-cli`) is a subset: the `gui` feature is not separated per binary in the metadata; filter with the properties if needed.
* **proc-macro and build crates:** `normal` includes proc-macro crates (they run at compile time and their output ships); `build` is the set only reachable through build-dependencies.
* **Licence data** is what the crates declare. It is not scanned from the code. 79 shipped crates have no licence file; 12 non-Cargo licences are declared by the generator without reading the packages.
* **Montserrat** is credited in the management-plane About page and mentioned in `assets/brand/README.md` as an optional wordmark typeface, but **no Montserrat font file is bundled**, so it is not in the SBOM. Either remove the credit or add the font and its licence text (GAP G-15).
* **Two versions of each font** are in the repository (Inter 4.000 OTF vs 4.001 TTF; JetBrains Mono 2.304 vs 2.211); the SBOM lists both.
* The Lucide subset in the management plane was copied from whichever `gpui-kit-assets` was in the builder's cargo registry; its version was not recorded at the time.
* The boot-agent build verifies PGP only if `gpgv` and the Debian keyring are installed (`pgp_verified` in the manifest says which); the SBOM carries that flag as a property.
* The SBOM describes the repository at one commit. It does not prove that the shipped binary was built from it (no provenance attestation).
