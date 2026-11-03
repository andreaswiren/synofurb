# Releasing synofurb

A release is a Git tag `vX.Y.Z` (or `vX.Y.Z-rc.1`). Pushing the tag runs `.github/workflows/release.yml`, which refuses to build unless the
**design gate** passes, then builds and packages the desktop app for six platforms, builds the NAS-side assets, writes and **signs** `SHA256SUMS`, and
creates the GitHub Release. The in-app updater and the one-line installers consume exactly what is attached to that release.

```
tag v0.2.0 ─► design-gate ─► meta (versions, key placeholder) ─► test ─┬► build x6 (Windows, macOS, Linux; x86_64 + aarch64)
   or manual                (scripts/release-gate.py)                  ├► agent x3 (boot agent: amd64, arm64, armhf)
   dispatch                                                            └► static (mgmt plane tarball, synofurb-dom, compat-models.json)
                                                                              │
                                                       checksums: validate asset names, SHA256SUMS, minisign, release notes
                                                                              │
                                                       release: GitHub Release (only job with write access, needs approval if configured)
```

## One-time setup

### 1. The signing key (minisign)

```sh
minisign -G -p synofurb-update.pub -s synofurb-update.key        # choose a strong password
```

* Store `synofurb-update.key` offline and back it up. **Never commit it.** Losing it means shipping a new app version that embeds a new public key (old apps cannot be re-keyed remotely).
* Paste the **public key** (second line of the `.pub` file, starts with `RW`) in all of these places - they must be identical, and the release workflow cross-checks the signature
  against the one in `install.sh`:

  | File | Replace |
  |---|---|
  | `install.sh` | `MINISIGN_PUBKEY_BUILTIN="REPLACE_WITH_MINISIGN_PUBLIC_KEY"` (first occurrence only; the comparison further down stays) |
  | `apps/mgmt-plane/install.sh` | the same line |
  | `install.ps1` | `$BuiltinKey = 'REPLACE_WITH_MINISIGN_PUBLIC_KEY'` |
  | `crates/synofurb-core/src/updater.rs` | `UPDATE_PUBLIC_KEY` (and delete the reminder test, see `docs/UPDATES.md`) |
  | `docs/INSTALL.md`, this file | the "Public key" section below |

  A tag push fails in the `meta` job while any installer still has the placeholder.

### 2. GitHub repository settings

* **Actions secrets** (Settings > Secrets and variables > Actions; better: create an environment named **`release`** and put them there, then add *required reviewers* so every release needs a
  human click; the `checksums` and `release` jobs use that environment):

  | Secret | Required | Used for |
  |---|---|---|
  | `MINISIGN_SECRET_KEY` | **yes** for tags | the whole contents of `synofurb-update.key` (both lines) |
  | `MINISIGN_PASSWORD` | if the key has a password | fed to `minisign -S` on stdin |
  | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | optional | macOS code signing. Certificate = base64 of a "Developer ID Application" `.p12` (`base64 -i cert.p12`); identity e.g. `Developer ID Application: Name (TEAMID)` |
  | `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | optional | notarization (app-specific password from appleid.apple.com) |
  | `APPLE_INSTALLER_IDENTITY` | optional | signing the macOS `.pkg` ("Developer ID Installer: ...") |
  | `WINDOWS_CERTIFICATE_PFX_BASE64`, `WINDOWS_CERTIFICATE_PASSWORD` | optional | Authenticode signing of the exe/msi (base64 of the `.pfx`; the thumbprint is read after import; timestamp server `http://timestamp.digicert.com`, override with env `WINDOWS_TIMESTAMP_URL`) |

  **Without the optional secrets the builds still work, unsigned**, with a visible warning annotation (SmartScreen / Gatekeeper will warn users). Without `MINISIGN_SECRET_KEY` a
  tag push **fails**; only a manual dry run proceeds, loudly, without `SHA256SUMS.minisig`.
* **Tag protection**: restrict who can create `v*` tags (Settings > Rules). Require *signed* tags/commits if you can.
* Workflow permissions: the workflow declares its own least-privilege `permissions:`; only the final `release` job has `contents: write`.

### 3. Pinned actions

Third-party actions are pinned by commit SHA with the version in a comment. Bump them deliberately (Dependabot for `github-actions` works with this format). `cargo-packager` is pinned to
`0.11.8` in `CARGO_PACKAGER_VERSION`; it is installed with `cargo install --locked` and cached.

## Per release

1. **Pick the version** (SemVer; `-rc.N` for release candidates, which become GitHub "pre-releases" and are not offered on the `stable` update channel).
2. **Bump it** in:
   * `Cargo.toml` `[workspace.package] version` - **the single source of truth** for the app, the asset names and the installers. Then refresh the lockfile (`cargo update -p synofurb -p synofurb-core` or just `cargo check`) and commit `Cargo.lock`.
   * `apps/mgmt-plane/VERSION` **and** the `DEFAULT_VERSION` fallback in `apps/mgmt-plane/synofurb_mgmt.py` - only when the management plane changed; it has its own version. `scripts/package-mgmt.sh` fails if the two disagree.
   * `boot/dom/synofurb-dom` (`VERSION=`) only when that script changed; the boot agent is built with the app version (`--version`).
   * `CHANGELOG.md`: add a `## [X.Y.Z] - YYYY-MM-DD` section (Keep a Changelog style). The release notes are taken from it; if it is missing they fall back to the git log since the previous tag.
3. **Check locally**: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, `python3 -m unittest discover -s apps/mgmt-plane/tests`,
   `python3 -m unittest discover -s tests/gate`, `tests/install/run-tests.sh`, `tests/install/test-package-scripts.sh`.
3a. **SBOM and security checks** (until the workflow does it): `python3 -m unittest discover -s tests/sbom`; `python3 scripts/gen-sbom.py --out dist/sbom --timestamp "$(date -u +%Y-%m-%dT%H:%M:%SZ)"` and read the summary (licence unknown, copyleft, unresolved agent versions); `python3 scripts/gen-sbom.py --check dist/sbom/*.cdx.json dist/sbom/*.spdx.json --cargo-lock Cargo.lock`;
   `python3 scripts/gen-licenses.py --out dist/sbom/THIRD-PARTY-LICENSES.txt`; `cargo deny --locked check` once cargo-deny is installed (`deny.toml`). For the exact boot-agent versions pass the agent build's `MANIFEST-<arch>.json` (`--agent-manifest DIR --require-agent-manifest`). Details and the workflow steps: [compliance/SBOM.md](compliance/SBOM.md).
4. **Run the critics** (`docs/DESIGN-GATE.md`): commit design changes and screenshots, run `ux-critic` and `ui-critic`, fix every blocking finding, re-run, commit `design/approvals/*.json`.
   `python3 scripts/release-gate.py` must print `PASSED`.
5. **Dry run**: Actions > *release* > *Run workflow* with *dry_run* ticked (the default) on the commit you intend to tag. It builds and packages everything, validates the asset set and
   uploads the `release-final` artifact, but **creates no release** and only *reports* the gate. Download the artifacts and install-test them.
6. **Tag and push**: `git tag -s v0.2.0 -m "synofurb 0.2.0" && git push origin v0.2.0`. The tag must equal `v` + the `Cargo.toml` version or the `meta` job fails.
7. **Approve** the `release` environment if you configured reviewers, then watch the run. A failing `design-gate` job means nothing else ran.
8. **Verify the published release** (next section), then announce.

To (re)publish for an existing tag without a new push: *Run workflow*, untick *dry_run*, enter the tag (`v0.2.0`). The gate is enforced.

## What the workflow produces

Attached to the GitHub Release (names are a contract - `docs/UPDATES.md` - the updater ignores anything else):

| Asset | Built by |
|---|---|
| `synofurb-<v>-windows-x86_64.msi` (WiX 3 MSI), `.exe` (NSIS setup), `.zip` (portable) | cargo-packager `wix,nsis` + `scripts/package-portable.sh` on `windows-latest` |
| `synofurb-<v>-windows-aarch64.{msi,exe,zip}` | the same on `windows-11-arm`; **experimental**: if it fails the release proceeds without them |
| `synofurb-<v>-macos-aarch64.{dmg,tar.gz}` (+ `.pkg`) | `macos-14`; cargo-packager `app,dmg`, `scripts/package-macos-pkg.sh` (optional) |
| `synofurb-<v>-macos-x86_64.{dmg,tar.gz}` (+ `.pkg`) | `macos-15-intel` (macOS 13 runners were retired by GitHub) |
| `synofurb-<v>-linux-x86_64.{deb,rpm,AppImage,tar.gz}` | `ubuntu-24.04`; cargo-packager `deb,appimage`, `scripts/package-rpm.sh` (rpmbuild), portable archive |
| `synofurb-<v>-linux-aarch64.{deb,rpm,AppImage,tar.gz}` | `ubuntu-24.04-arm` |
| `synofurb-agent-<amd64\|arm64\|armhf>-<v>.vmlinuz` and `.initrd` | `boot/agent/build.sh <arch>` via `scripts/package-agent.sh` (verified against the build's own checksums) |
| `synofurb-dom` | `boot/dom/synofurb-dom` |
| `compat-models.json` | `compat/models.json` |
| `synofurb-mgmt-<mgmt-version>.tar.gz` | `scripts/package-mgmt.sh` from `apps/mgmt-plane/` (flat archive for `/opt/synofurb-mgmt`: the four base files plus `synofurb_addons.py`, `addons.pack.json` and `synofurb_edge.py`). It is also what the **Stable** channel of the management plane's own updater reads (web UI page Updates and `update --from-github`, `docs/MANAGE.md`): the highest *mgmt* version among the non-pre-release releases that carry it plus `SHA256SUMS` wins, so bump `apps/mgmt-plane/VERSION` (and `DEFAULT_VERSION`) in the release that should reach units |
| `synofurb-<v>-sbom.cdx.json`, `synofurb-<v>-sbom.spdx.json`, `THIRD-PARTY-LICENSES.txt` | `scripts/gen-sbom.py`, `scripts/gen-licenses.py` (not in the workflow yet: [compliance/SBOM.md](compliance/SBOM.md) section 5 lists the exact steps; `package-collect.sh` already accepts them as optional) |
| `SHA256SUMS` | every file above, `sha256sum` format, sorted |
| `SHA256SUMS.minisig` | minisign signature of `SHA256SUMS`, trusted comment `synofurb <v>` |

`<v>` = the tag without `v` (`0.2.0`, `0.3.0-rc.1`). The portable archives are flat: `synofurb[.exe]`, `LICENSE`, `README.txt` (+ `synofurb.png` on Linux), no top-level directory.
`scripts/package-collect.sh` fails the run if a required asset is missing or any file does not match the contract (stale files from another version are rejected too).
Optional (a warning only): `.pkg`, Windows aarch64.

Each build leg also smoke-tests its result: the binary prints its version; Linux installs the `.deb`, validates the desktop file, runs the AppImage and inspects the `.rpm`; Windows installs the MSI
silently, runs the binary and uninstalls.

### Signing

* **macOS**: with `APPLE_CERTIFICATE`+`APPLE_SIGNING_IDENTITY` cargo-packager imports the certificate into a temporary keychain and signs the `.app` and `.dmg`; with `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` it also notarizes.
  The `.pkg` is signed/notarized with `APPLE_INSTALLER_IDENTITY` (+ the same Apple ID secrets). Without them the artifacts are unsigned (the bare `tar.gz` binary installed by `install.sh` is not quarantined by macOS).
* **Windows**: with `WINDOWS_CERTIFICATE_PFX_BASE64` the certificate is imported into the runner's user store and cargo-packager signs the binary and installers with `signtool`
  (SHA-256, RFC 3161 timestamp). Cloud HSM / Azure Trusted Signing can be used instead by adding a `sign-command` to `packaging/Packager.toml`.
* **Everything**: `SHA256SUMS.minisig` is independent of OS code signing and is what the updater and install scripts verify.

## Verifying a published release

```sh
mkdir v && cd v
gh release download v0.2.0                                   # or download from the releases page
minisign -Vm SHA256SUMS -P RWQ...                            # the public key printed below
sha256sum -c SHA256SUMS                                      # every asset
```

then on clean machines: `curl -fsSL .../install.sh | sh -s -- --version 0.2.0 --run --help` (Linux and macOS), `install.ps1 -Version 0.2.0` (Windows), and from the **previous** app version
`synofurb self-update --check` followed by a real update (the upgrade path is the part nobody tests until it breaks).

### Public key

`RWQ...` *(paste the real key here when it exists, see "One-time setup")*

## How the in-app updater consumes a release

`synofurb self-update` / the update button (`crates/synofurb-core/src/updater.rs`, `docs/UPDATES.md`) lists releases through the GitHub API, picks the highest version on its channel (`stable`: not a
pre-release; `prerelease`: anything not a draft), then selects the asset by **exact name** `synofurb-<version>-<os>-<arch>.<ext>`, downloads `SHA256SUMS` and `SHA256SUMS.minisig`, verifies the signature
against `UPDATE_PUBLIC_KEY`, looks up the file's digest, streams the asset while hashing, and only then installs it. Consequences for releasing: never rename or re-upload an asset by hand (the sums would no
longer match), never publish a file whose embedded version differs from its release, and keep `SHA256SUMS` free of directories in file names. The agent, DOM, compat and mgmt assets do not match the updater's
`synofurb-<version>-<os>-<arch>` pattern and are ignored by it; the installers and the app fetch them by their own names.

## If something goes wrong

* **A bad release is already public**: edit it on GitHub and mark it *pre-release* (or delete the release and its tag), then ship `vX.Y.Z+1` quickly. The updater picks the highest version, so fixing forward is the only reliable remedy.
* **Signing key suspected stolen**: revoke by shipping an app version with a new public key through a channel users trust (the old key cannot be rotated remotely); tell users to reinstall from a fresh download.
* **`design-gate` fails on a hotfix**: re-run the critics. If that is impossible, a maintainer may use the signed, time-limited override described in `docs/DESIGN-GATE.md`; the release notes will say so.
* **cargo-packager builds failing on download steps**: it fetches WiX 3, NSIS, `linuxdeploy` and AppRun from GitHub at build time; re-run the leg.

## Known limitations

* Linux binaries are built on Ubuntu 24.04, so they need **glibc >= 2.39** (Debian 13, Ubuntu 24.04, Fedora 40+). Older distributions need a source build. The `.deb` declares `libc6 (>= 2.39)`.
* The AppImage depends on the host's GPU/windowing libraries (bundling `libvulkan` breaks drivers) and on FUSE 2 unless run with `APPIMAGE_EXTRACT_AND_RUN=1`.
* `windows-11-arm` is a newer runner image; its leg is allowed to fail. macOS Intel builds use `macos-15-intel`, the last Intel image GitHub offers (announced end of life: August 2027).
* cargo-packager 0.11.8 ignores its own `--binaries-dir` flag; `scripts/package-cargo-packager.sh` therefore writes the directory into a generated copy of `packaging/Packager.toml`.
* Reproducibility: the portable archives, the mgmt tarball and `SHA256SUMS` are reproducible; the installers made by WiX/NSIS/dmg tooling are not bit-for-bit.
* No SLSA/GitHub build-provenance attestation yet (would need `id-token: write` and `attestations: write` on the `checksums` job and `actions/attest-build-provenance`).

## Doing a local packaging dry run

```sh
cargo build --release --locked -p synofurb --target x86_64-unknown-linux-gnu
cargo install cargo-packager --version 0.11.8 --locked
scripts/package-cargo-packager.sh --target x86_64-unknown-linux-gnu --formats deb,appimage --out dist/release
scripts/package-rpm.sh            --target x86_64-unknown-linux-gnu --out dist/release     # needs `rpm`
scripts/package-portable.sh       --target x86_64-unknown-linux-gnu --out dist/release
scripts/package-cargo-packager.sh --target x86_64-pc-windows-msvc --formats wix --render-only   # just render + inspect the config
```
