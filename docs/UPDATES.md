# Updates

The desktop app can update itself (`synofurb self-update`, or the update button in the app once it is wired up). It does
this only for releases that are **signed by the project key**, and it never installs anything it could not verify.

**Status: the signing key is still a placeholder. Until the maintainer sets it up (below), `check` works but every
install is refused with an explanation.** Nothing is silently skipped.

## How an update works

1. `updater::check` asks the GitHub REST API (`https://api.github.com/repos/andreaswiren/synofurb/releases`) for the
   release list. Requests send a `User-Agent`, use `ETag`/`If-None-Match` (a 304 costs no rate limit and is served from
   the cache), and report GitHub's rate limit as a normal error with a "try again in N minutes" hint.
2. It picks the newest release on the chosen **channel**:
   * `stable` (default): not marked "pre-release" on GitHub **and** the tag has no pre-release part (`-rc.1`);
   * `prerelease`: anything that is not a draft.
   The highest *version* wins, not the newest upload. Tags are semver, `v` prefix optional (`v0.2.0`), build metadata
   (`+abc`) is ignored, `0.2.0-rc.1 < 0.2.0`. Tags that are not versions are skipped.
3. It picks the asset for this OS and CPU by name (contract below) and notes the sizes, release notes and publish date.
4. `updater::download_verified` (a) downloads `SHA256SUMS` and `SHA256SUMS.minisig`, (b) verifies the minisign signature
   of `SHA256SUMS` against `UPDATE_PUBLIC_KEY`, (c) looks up the asset's digest in `SHA256SUMS`, (d) streams the asset to
   disk while hashing it, and only then (e) compares the digest. A mismatch deletes the partial file. The asset is not
   even requested if the signature check fails.
5. `updater::apply` installs the verified file:

| OS | File | What happens |
|---|---|---|
| Windows | `.msi` | `msiexec /i <file> /passive` is started detached, then the app exits. |
| Windows | `.exe` | The installer is started detached, then the app exits. |
| macOS | `.dmg`, `.pkg` | Opened with `open`; you finish the installation. |
| Linux | `.AppImage`, `.tar.gz` | The running executable (`$APPIMAGE` if set) is replaced atomically; see below. |
| Linux | `.deb`, `.rpm` | The command is printed. It is run through `pkexec` only if `pkexec` exists; otherwise you run it. |
| any | `.zip` | Not installed automatically; the verified file is left in the download folder. |

Linux replacement: the new file is written next to the executable (same directory, so the rename cannot cross a file
system), `chmod 755`, synced; the old executable is hard-linked (or copied) to `<name>.old`; then the new file is
renamed over the executable. The executable path never disappears, and a failure at any step leaves the old binary
alone. `<name>.old` is kept until the new version has started successfully: call `updater::mark_start_ok(exe)` once
the app is up (the CLI wiring does this), or `updater::rollback(exe)` to go back.

## Asset naming contract (for the release workflow)

Every release must contain, **attached to the release** (not only as workflow artifacts):

```
synofurb-<version>-<os>-<arch>.<ext>     the installable files
SHA256SUMS                                 sha256sum output for every file above
SHA256SUMS.minisig                         minisign signature of SHA256SUMS
```

* `<version>`: the tag without the leading `v` and without build metadata (tag `v0.2.0` gives `0.2.0`; `v0.3.0-rc.1`
  gives `0.3.0-rc.1`). A file whose embedded version differs from its release is ignored.
* `<os>`: `windows`, `macos` or `linux`. `<arch>`: `x86_64` or `aarch64`.
* `<ext>`: one of `msi`, `exe`, `dmg`, `pkg`, `deb`, `rpm`, `AppImage` (exactly this capitalisation), `tar.gz`, `zip`.
* Preference when several exist: Windows `msi`, `exe`, `zip`; macOS `dmg`, `pkg`, `tar.gz`, `zip`; Linux `AppImage`,
  `tar.gz`, `deb`, `rpm` (or `deb`, `rpm`, `AppImage`, `tar.gz` when the running binary is under `/usr` or `/opt` and not
  an AppImage).
* A `tar.gz` for Linux must contain a regular file named `synofurb` somewhere in the archive; only that member is used.
* `SHA256SUMS` format: `<64 hex>  <file name>` per line, file names without directories (create it from inside the
  directory that holds the files). File names may only contain letters, digits and `. - _ +`.
* Example for 0.2.0: `synofurb-0.2.0-windows-x86_64.msi`, `synofurb-0.2.0-macos-aarch64.dmg`,
  `synofurb-0.2.0-linux-x86_64.AppImage`, `synofurb-0.2.0-linux-aarch64.tar.gz`.

Sketch of the signing step in the release workflow:

```yaml
- name: Checksums and signature
  working-directory: dist
  env:
    MINISIGN_SECRET_KEY: ${{ secrets.MINISIGN_SECRET_KEY }}
  run: |
    sha256sum synofurb-* > SHA256SUMS
    printf '%s\n' "$MINISIGN_SECRET_KEY" > "$RUNNER_TEMP/minisign.key"
    minisign -S -s "$RUNNER_TEMP/minisign.key" -m SHA256SUMS -x SHA256SUMS.minisig \
      -t "synofurb ${GITHUB_REF_NAME}"
    rm -f "$RUNNER_TEMP/minisign.key"
```

(`minisign -S` needs a key without a password in CI, or feed the password on stdin. Install minisign on the runner first,
for example `sudo apt-get install -y minisign`.)

## Setting up the signing key (maintainer, once)

```sh
minisign -G -p synofurb-update.pub -s synofurb-update.key
```

1. Keep `synofurb-update.key` secret and backed up offline. Add its **whole contents** as the repository secret
   `MINISIGN_SECRET_KEY` (Settings > Secrets and variables > Actions). If you set a password, also add it as a secret and
   pipe it to `minisign -S`.
2. Open `synofurb-update.pub`. The second line (starts with `RW`) is the public key. Put it in
   `crates/synofurb-core/src/updater.rs`:

   ```rust
   pub const UPDATE_PUBLIC_KEY: &str = "RWQ...your key...";
   ```

   (replace the `= UPDATE_PUBLIC_KEY_PLACEHOLDER;` right-hand side).
3. Delete the unit test `shipped_key_is_still_the_placeholder_and_that_blocks_updates`; it exists to remind you.
4. Publish a release, then test `synofurb self-update --check` and a real update from the previous version.
5. Key rotation means shipping a new app version that contains the new public key; older apps cannot be moved to a new
   key remotely. Keep the old key until the old versions are gone.

Never commit the secret key. Never put it in a workflow file.

## Security model

What it protects against:

* **A tampered or swapped download** (compromised CDN, man in the middle, a modified asset): the SHA-256 is checked
  against a file that is itself signed.
* **A compromised GitHub account or release page**: whoever can edit a release can replace assets, but cannot produce a
  valid signature without the secret key, so the updater refuses the result.
* **Wrong hosts**: only `https`, and only `github.com` and `*.githubusercontent.com` (GitHub's release CDN) for downloads,
  `api.github.com` for the release list. This is enforced on every redirect hop, so a redirect to another host is
  refused. No credentials in URLs, no custom ports. No GitHub token is ever sent.
* **Path tricks**: asset names must be plain file names; archives are read for exactly one member name and nothing is
  extracted by path.
* **Size abuse**: API and checksum responses have hard caps; the asset may not exceed the size the release declares.
* **Interrupted updates**: the executable is never half-written in place, and `.old` allows rollback.

What it does not protect against:

* **A stolen signing key.** Whoever has it can sign anything. Protect it like a root credential.
* **A malicious maintainer or a malicious build.** The signature proves "the key holder released this", not that the
  code is good.
* **Downgrade/freeze games by someone who can block GitHub.** The updater only offers versions newer than the one
  installed, but a network attacker can make you miss an update.
* **OS-level trust.** The Windows installer and macOS app are not covered by Authenticode or notarization unless the
  release workflow adds them, and the OS may warn. This signature is separate from those.
* **Legacy-mode minisign signatures are accepted** (plain Ed25519 over the file) as well as the default pre-hashed mode.
  Both need the same secret key.

## Limits

* No background polling; checks happen when you ask (or when the app's UI asks).
* No delta updates; the whole file is downloaded.
* `.zip` is never installed automatically.
* Linux package installs need `pkexec` (polkit) for the privilege prompt; without it you get the command to run.
* On Windows the installer replaces the app after it exits; if the installer is cancelled, nothing changes.
* The `check` call works with a placeholder key, but any install is refused until the real key is in place.
