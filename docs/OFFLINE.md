# Offline install mode

synofurb can install Debian 13 (trixie) on a Synology unit in two ways:

| | Online (default) | Offline |
|---|---|---|
| Installer kernel/initrd | downloaded by this computer when you start the install | read from the bundle |
| Boot agent (internal-flash units) | downloaded from the synofurb release | read from the bundle |
| Debian packages | the NAS downloads them from `deb.debian.org` (debootstrap + apt) | the NAS downloads them **from this computer** over the LAN |
| Internet needed | on this computer and on the NAS, during the install | only while you build the bundle |

In offline mode you first build a **bundle** while you have Internet. Later this computer serves the bundle to the
NAS and the whole install works without any Internet connection, on either side. The bundle is a plain directory:
build it on one machine, copy it to another by USB stick, and install from there.

```
synofurb bundle create --arch amd64 --out ./bundle     # needs Internet, resumable; newest kernel, like the install default
synofurb bundle info   ./bundle
synofurb bundle verify ./bundle                        # re-hashes everything, no network
synofurb install ... --offline-bundle ./bundle         # no Internet needed from here on
```

(The `install` flag name is chosen by the CLI/GUI; the engine takes `InstallJob.offline_bundle: Some(dir)`.)

## What is in a bundle

```
bundle/
  manifest.json            versions, suites, archs, kernel channel, creation date, tool version,
                           every file with size + sha256, warnings
  SHA256SUMS               the same hashes for `sha256sum -c`
  installer/<arch>/        kernel, initrd.gz (+ DTBs), and the InRelease + SHA256SUMS the hashes were checked against
  installer/firmware.cpio.gz   optional; non-free firmware for the installer, shared by all architectures
  agent/agent-<arch>.{vmlinuz,initrd}   boot agent for units with an internal boot flash (DOM), optional
  mgmt/                    management-app update bundle: MANIFEST.sha256, synofurb_mgmt.py, VERSION, README.md, synofurb-mgmt.service
  mirror/debian/
    dists/<suite>/{InRelease,Release,Release.gpg}
    dists/<suite>/<comp>/binary-<arch>/Packages.{gz,xz}   (+ binary-all, + debian-installer/binary-<arch>)
    dists/<suite>-backports/...                            (only with the newest kernel)
    pool/...                                               only the .deb / .udeb files that are needed
```

The `dists/` files are copied **verbatim** from `deb.debian.org`. Debian's own signatures therefore stay valid and
`debootstrap`, `apt` and the installer's `anna` verify the packages exactly as they do online. The pool keeps the
upstream relative paths. A package that is not part of the bundle simply answers `404`.

### The management app (`mgmt/`)

`bundle create` also writes `mgmt/`: the web management app (`apps/mgmt-plane`, see [MANAGE.md](MANAGE.md)) in exactly the layout `synofurb_mgmt.py update --from DIR` takes
(`MANIFEST.sha256` in `sha256sum` format, `synofurb_mgmt.py`, `VERSION`, `README.md`, `synofurb-mgmt.service`; identical to what
`python3 apps/mgmt-plane/tools/make-update-bundle.py` writes, which a unit test compares). The files are the copy built into the synofurb that built the bundle, so the bundle
can later update a unit's web UI with no Internet and no repository checkout:

```
synofurb manage update --host <ip> --from-bundle ./bundle          # or ./bundle/mgmt
```

It is a few hundred KiB and is always rewritten, also when you resume or extend a bundle. The **installer does not use it**: a fresh install always gets the management app that is built into the
synofurb doing the install (the payload), with its `VERSION`. Only `manage update` reads `mgmt/` from a bundle.

Checks: `bundle verify` hashes every `mgmt/` file against `manifest.json` (like all files) and additionally loads the folder as the update would: `mgmt/MANIFEST.sha256` must exist, list and match every
file, `VERSION` must be a plain version number, no link may stand in for a file, and every file plus `MANIFEST.sha256` must be exactly what `manifest.json` recorded. A script edited together with
`mgmt/MANIFEST.sha256` is therefore still caught. Problems print as `MGMT  <reason>` and exit 1. `bundle info` prints `Mgmt app:  synofurb-mgmt <version>` (`mgmt_version` in `--json`),
an install with `--bundle` refuses a bundle whose `mgmt/` is damaged, and `manage update --from-bundle` runs the same checks before it connects.

**Older bundles** (built before `mgmt/` existed) have no such folder in `manifest.json`. They verify, serve and install exactly as before; `bundle verify` adds the line
`note: the bundle has no mgmt/ folder ...`, `bundle info` says `Mgmt app:  not included`, and `manage update --from-bundle <bundle>` fails with exit 2 and the hint to run
`synofurb bundle create` again with the same `--out` (only `mgmt/` and the manifest are rewritten; downloaded files are kept). Without `--from-bundle`, `manage update` uses the copy built into synofurb as always.
A `mgmt/` folder that someone copied into such a bundle by hand is reported as `EXTRA` and ignored.

Components: `main`, `contrib`, `non-free-firmware` (what the installer asks `debootstrap` for).

### Which packages are included

The set is a dependency closure computed from what the installer really installs, in this order:

1. **debootstrap base** (`--variant=minbase --include=ca-certificates,locales,dbus`): every package with
   `Priority: required` or `Essential: yes`, `apt`, `usr-is-merged`, the includes, and their dependencies.
2. **Everything `payload/chroot-setup.sh` installs**: keyring, tzdata, locales, initramfs-tools, the kernel
   (`linux-image-<arch>`; with the newest kernel from `<suite>-backports`, resolved like `apt -t <suite>-backports`
   so dependencies that only backports can satisfy come from backports too), the bootloader packages of **every**
   firmware type of that architecture (`grub-pc`, `grub-efi-*`, `efibootmgr`, `u-boot-tools`), both CPU microcode
   packages (`intel-microcode`, `amd64-microcode`; the CPU of the unit is not known when the bundle is built),
   `haveged` on armhf, the storage/network/admin tools, `firmware-*`, and `--extra` packages. A unit test
   (`closure_seeds_match_chroot_setup_script`) fails when the script and the lists in `offline.rs` drift apart.
3. **Installer udebs** (what the installer's `anna` downloads before the stage-2 script runs): the
   `ANNA_PACKAGES` of the generated script (virtual `*-modules` are resolved to the udebs matching the installer
   kernel version, read from the `lib/modules/` directory of the bundled initrd), everything the udeb archive
   marks `required`, `important` or `standard` (this includes `base-installer`, which pulls `debootstrap-udeb`), and
   every kernel-module udeb of the installer's kernel (what hardware detection loads for your disks and NICs).
   Module udebs of other kernel ABIs are not included; the archive keeps two ABIs side by side for a while.

Not followed: `Recommends` and `Suggests` (the install uses `--no-install-recommends`).

### Dependency resolution: what is simplified

The resolver is deliberately small and its limits are documented in the code (`offline::Resolver`):

* One candidate per name and suite. Version constraints decide *between suites* (a `>=` that only backports meets
  selects backports) but there is no backtracking. If both the suite's and the backports version of a package are
  needed (an early step installs the suite version, the kernel step needs the newer one), both are included.
* Alternatives (`a | b`): something already selected wins, otherwise the first one that exists.
* Virtual packages: a provider that is already selected wins, otherwise the one with the highest priority
  (`required` first), then the preferred suite, then the alphabetically first name.
* A package named explicitly (a seed) is the real package of that name even if another selected package merely
  provides the name (`base-files` provides `usr-is-merged`; debootstrap installs the real `usr-is-merged` too).
  This one was found by the integration test.
* A missing dependency fails the build and names the full chain (`libfoo (>= 2) | libbar` required by `a <- b <- [seed]`).

These simplifications are checked, not assumed: see "How completeness is proven".

## Sizes

Measured on 2026-11-01 with Debian 13.7 and the newest (backports 7.2) kernel; no installer firmware, no boot agent.
Sizes are what the bundle needs for that architecture alone; files shared between architectures
(`all` packages, the indexes of `binary-all`) are counted for each.

| Architecture | Packages | Installer udebs | Size |
|---|---|---|---|
| amd64 | 269 | 163 | 405.7 MiB (439 MiB bundle in total, 466 files) |
| arm64 | 265 | 153 | 342.4 MiB |
| armhf | 267 | 152 | 233.5 MiB |

(arm64 + armhf built into one bundle: 608 MiB, 818 files.)

Add about 470 MiB once for `installer/firmware.cpio.gz` (all architectures share it), and the boot agent
(a few MiB per architecture) for units with an internal boot flash. Building several architectures into one bundle
shares the `all` packages and indexes, so the total is less than the sum.

## How the bundle is built (`synofurb bundle create`)

* Hosts: only `https://deb.debian.org`, `https://cdimage.debian.org` and the GitHub release hosts (for the boot
  agent, through the existing signed-release downloader). Every redirect hop is checked against the same list.
  The operating system's trust store and `HTTPS_PROXY` are honoured.
* Order: `InRelease` (and `Release`, `Release.gpg`) -> package indexes (fetched by hash, so a mirror update while
  you build cannot give you an index that disagrees with `InRelease`) -> installer images -> closure -> a size
  estimate in the log -> firmware -> packages -> boot agent -> manifest.
* Verification while building: indexes against `InRelease`; the installer `SHA256SUMS` against `InRelease` and each
  installer file against `SHA256SUMS`; every `.deb`/`.udeb` against the SHA-256 and size of its (verified) `Packages`
  entry. If `gpgv` and a current `debian-archive-keyring` are installed the `InRelease` signature is checked too
  (the manifest records whether that happened). Without it the chain rests on HTTPS; the NAS still verifies Debian's
  signatures itself with the keyring it carries.
* Resumable: a download goes to `<name>.part` and is renamed only after it verified, so Ctrl-C never leaves a bad
  file in place. Run the same command again: files that already verify are skipped, a `.part` is continued with an
  HTTP `Range` request. `.building` marks an unfinished bundle; `Bundle::open` refuses it.
* A directory that is not empty and not a bundle is refused. Pool files that the new build does not need are
  pruned; `dists/` is always rewritten.

## How the bundle is verified (`synofurb bundle verify`)

Everything is re-hashed against `manifest.json` (no network): missing, changed and unexpected files are reported,
each `Packages` index is compared with the `InRelease` that lists it, and the management app in `mgmt/` is checked as described above. Before an install the engine additionally
re-hashes the installer kernel, initrd and firmware it is about to serve.

## How the app serves it to the NAS

The existing install server (`server.rs`) gets one more read-only route:

```
GET|HEAD /<token>/mirror/<path>      serves <bundle>/mirror/<path>
```

* The token is the same 128-bit random token as the other routes; a wrong token, a missing token or any other
  method (`POST`, `PUT`, ...) is refused.
* Path traversal is rejected: percent-decoding happens first, then empty segments, `.`, `..`, `/` or `\` hidden in an
  encoded segment, control characters and NUL are refused, and the resolved file must lie inside the mirror
  directory (symlinks leading out are refused). No directory listings.
* Files are streamed from disk with a correct `Content-Length`; single `Range` requests are supported
  (`206`/`416`). Each download runs on its own thread so a slow NAS never blocks the log stream.
* `.../by-hash/SHA256/<hash>` URLs (which apt tries first) are answered from the file that the signed `InRelease`
  lists under that hash, so the bundle does not need a second copy of every index.

During `session::stage` an offline job:

1. opens the bundle, checks it against the job (architecture, suite, kernel channel, extra packages; a mismatch is a
   clear error, e.g. "the bundle was built for the Stable kernel but the install asks for the Newest kernel. Either install with `--kernel stable` ..., or build a bundle for the kernel you want: `synofurb bundle create --kernel newest --out <dir>`"),
2. sets `mirror_host = <desktop-ipv4>:<port>` and `mirror_dir = /<token>/mirror/debian` for this job only, so the
   preseed (installer `anna`), the stage-2 script (`wget` preflight, `debootstrap`) and `env.sh` (apt) all use it,
3. serves the installer kernel/initrd from the bundle (hashes verified), and the boot agent from the bundle for
   internal-flash units (never from the Internet),
4. passes `OFFLINE=1` and `OFFLINE_SUITES` to `chroot-setup.sh`.

`chroot-setup.sh` with `OFFLINE=1`:

* writes apt sources only for the suites that exist in the bundle (no `-updates`, no `security`),
* for the duration of the install adds `Acquire::Check-Valid-Until "false"`, `Acquire::Check-Date "false"` and
  `Acquire::Languages "none"` in `/etc/apt/apt.conf.d/99synofurb-offline`. The reasons: `trixie-backports` Release
  files are valid for only **one week**, a bundle is a snapshot, and the NAS clock is not synchronized yet.
  Signatures are still verified.
* as its last step removes that file and rewrites `/etc/apt/sources.list.d/debian.sources` to the official mirrors
  (`deb.debian.org` `trixie`, `trixie-updates`, `trixie-security`, and `trixie-backports` when the kernel channel is
  Newest) so the installed system updates normally later. Online installs run exactly the code they ran before.

## What stays online / what is not bundled

* **Nothing** during the install: all package, installer and agent downloads come from this computer.
* The **installed** system uses the official Debian mirrors for updates from the first boot on. Packages that are
  newer than the bundle (security updates, point-release updates released after the snapshot date shown by
  `synofurb bundle info`) are not in the bundle; the first `apt upgrade` online brings them. For the same reason the
  bundle has no `-updates` or `-security` suite.
* Other architectures than the ones you chose, `contrib`/`non-free` packages beyond the closure, packages you did
  not name with `--extra`, `Recommends`.
* **One kernel per bundle, no fallback.** `bundle create` builds for the Newest kernel by default (`--kernel newest`, the same default as `install`);
  `--kernel stable` builds the 6.12 LTS kernel instead (`--kernel-newest` is the old spelling of the default). If the backports kernel fails to install,
  `chroot-setup.sh` online falls back to the suite's kernel; an offline bundle built for Newest does not contain it (the install then fails instead of
  silently using another kernel). The Stable kernel is deliberately not added to Newest bundles: it would add a second kernel and its modules (roughly 100 MiB per
  architecture, not measured) to make a fallback that has not been exercised offline. A mismatch between bundle and install is caught before the NAS is touched,
  with the flags for both ways out (see below).

## Limits

* **This computer has to stay on the LAN and awake until the installer reports it is done.** The NAS fetches about
  a quarter to half a gigabyte from it; a laptop that suspends or leaves the network breaks the install after the
  point of no return. The server binds the IPv4 address the NAS can reach; IPv6-only networks are not supported
  in offline mode.
* The firewall of this computer must allow the HTTP, log TCP and syslog UDP ports shown in the log (as online).
* A bundle is tied to the Debian snapshot it was built from. A newer synofurb that changes the package lists
  (`chroot-setup.sh`) needs a new bundle: `check_for_install` catches kernel channel, suite, architecture and
  extra-package mismatches; an old bundle with a new script that wants a package the bundle lacks fails at apt with
  a 404 for that package, and the log names it.
* Installer and archive can be out of sync for a few hours after a kernel update (netboot images vs. module udebs).
  The build then warns ("the archive has no module udebs for the installer kernel ..."). Build again later. The
  same limit applies online.
* On armhf the archive has no `xfs-modules` udeb , so `partman-xfs` cannot be installed there and is
  left out with a warning; the stage-2 script formats with `mkfs.xfs` from `xfsprogs-udeb` and does not use partman,
  and the same happens online. Whether the armhf installer kernel has XFS built in is not something the bundle can check.
* `crc-modules` is no longer in `script::ANNA_PACKAGES`: trixie has no such udeb, and a single unknown name makes
  `anna-install` fail the whole call (found on a real RS812+ in 0.2.7: nothing was loaded, so `parted` was missing).
  The script now calls `anna-install` once per udeb, so a name Debian drops later costs a warning, not the install.
* `Valid-Until`: the temporary apt settings above only exist during the install; they are removed afterwards.

## How completeness is proven (`tests/offline/run.sh`)

`SYNOFURB_OFFLINE_ARCHS=amd64,arm64,armhf tests/offline/run.sh` builds a real bundle and then serves it with the
production server code on `127.0.0.1` (only `/<token>/mirror/` is reachable) and runs:

1. **debootstrap**, with the arguments of the generated stage-2 script
   (`--variant=minbase --components=main,contrib,non-free-firmware --include=ca-certificates,locales,dbus`) and
   Debian's own archive keyring (downloaded from the official site as an independent trust anchor, not taken from
   the bundle). `gpgv` first checks the `InRelease` and `Release.gpg` of both suites. No `--no-check-gpg`.
2. **the real `payload/chroot-setup.sh`** (everything up to the storage configuration) inside the debootstrap chroot
   of the host architecture, with `OFFLINE=1`, apt pointed at the bundle only, and no other route to the Internet:
   the real kernel, the bootloader, microcode and tools are installed with the real `apt-get install` (backports
   kernel with `-t`). Then the extra bootloader/microcode packages of the other firmware types are resolved and
   fetched, and the closing offline block of the script is run to check the sources of the installed system.
3. **for the other architectures** (`debootstrap --foreign` downloads and unpacks the whole base from the bundle) and
   an `apt-get` simulation of every apt step of the script with apt's own resolver, on a synthetic dpkg
   status for the kernel step; every file apt would download is requested from the served bundle.
4. **the udebs** (`live_udeb_closure_is_served`): the resolver is run over the *served and signature-chained*
   udeb index; every `ANNA_PACKAGES` closure and the whole installer set is fetched over HTTP and hashed.

Result of the run that produced the numbers above (2026-11-01, Debian 13.7 snapshot, backports kernel 7.2.11): amd64
full chroot install passed (264 packages installed from the bundle alone, kernel `7.2.11+deb13-amd64`; `debootstrap`
used Debian's real keyring and a valid signature, no `--no-check-gpg`); arm64 and armhf passed `debootstrap --foreign`
plus the apt simulation (all 265 and 267 files apt would download are served); udeb closures served and hashed for all
three architectures.

### What this does not prove

* The Debian installer itself (`anna`, `hw-detect`, `base-installer`) was not run: that needs the NAS or a VM with
  the netboot images. The udeb set is therefore "everything anna would load on its own plus every module udeb", not a
  trace of a real run. Hardware detection that loads a udeb outside that set would fail the same way it does against
  a partial mirror.
* The package postinst scripts ran in a chroot of an x86-64 host. arm64/armhf are proven by download, unpacking and
  apt resolution, not by running their maintainer scripts.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `unfinished bundle` | the build was interrupted. Run `synofurb bundle create --out <same dir> ...` again. |
| `bundle verify` prints `MGMT ...` | `mgmt/` was changed or is incomplete; copy it again or run `bundle create` on the same directory (it rewrites `mgmt/`). |
| `bundle verify` prints `note: the bundle has no mgmt/ folder` | the bundle comes from an older synofurb. Nothing is wrong; run `bundle create` on it again to add `mgmt/`. |
| `bundle verify` prints `CHANGED`/`MISSING` | the copy (USB stick) is damaged; copy again, or rebuild (only broken files are fetched). |
| `the bundle has no arm64 support` | build it with `--arch arm64` (you can add architectures to a bundle by running `create` again with all of them). |
| `the bundle was built for the ... kernel but the install asks for ...` | install with `--kernel <the bundle's kernel>`, or build a bundle for the install's kernel with `bundle create --kernel newest\|stable` (default newest). |
| `these extra packages are not in the bundle` | add them with `bundle create --extra <pkg>`. |
| `checksum mismatch ... archive may have changed` while building | Debian published an update during the build. Run the command again. |
| `not found on the server ... superseded packages are removed` | same: the archive changed under the index; run again. |
| NAS log: `wget: server returned error: HTTP/1.1 404` for a `.deb` | a package outside the closure was requested (the script changed after the bundle was built). Rebuild the bundle with the current synofurb. |
| NAS log: `Release file ... is not valid yet` / `expired` | should not happen (the install disables those checks); if you see it, `99synofurb-offline` was not written: look at the `[chroot]` lines of the log. |
| the installer cannot reach the mirror | firewall on this computer; wrong LAN address chosen (`--bind`); the computer left the network. |
| `no Debian archive keyring in the target` | debootstrap did not install `debian-archive-keyring` from the bundle; run `bundle verify`. |
