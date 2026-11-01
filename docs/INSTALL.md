# Installing synofurb

Three things can be installed, on three different machines:

| What | Where | How |
|---|---|---|
| **The desktop app** (`synofurb`) | your Windows / macOS / Linux PC | [one-liner](#the-desktop-app-one-liners) or [packages](#packages-by-hand) |
| **The whole conversion, headless** | any Linux/macOS box on the NAS's network | [one command](#convert-a-nas-headless-from-a-linuxmacos-box) |
| **The NAS management plane** | an *existing* Debian box (the installer puts it on converted NAS units for you) | [one-liner](#the-nas-management-plane-on-an-existing-debian-box) |

> Reinstalling a NAS repartitions its disks. **Make a complete, verified backup somewhere else first.** The app and the CLI
> refuse to proceed without explicit confirmations; see the [README](../README.md).

Every release publishes, next to the files, `SHA256SUMS` and `SHA256SUMS.minisig` (a [minisign](https://jedisct1.github.io/minisign/)
signature of `SHA256SUMS`). The installer scripts below verify both. How to verify by hand: [Verifying a download](#verifying-a-download).

## The desktop app: one-liners

### Linux and macOS (`install.sh`, POSIX sh)

```sh
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh
```

With options (everything after `-s --` goes to the script):

```sh
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- --version 0.2.0 --dir ~/.local/bin --run scan
```

What it does: detects OS and CPU (`x86_64`, `aarch64`), asks the GitHub API for the latest release (or uses `--version`), downloads
`synofurb-<version>-<os>-<arch>.<ext>` plus `SHA256SUMS` and `SHA256SUMS.minisig`, verifies the **minisign signature** (needs the `minisign`
tool) and the **SHA-256**, then installs the `synofurb` binary to `~/.local/bin` and, on Linux, a desktop entry and icon. It never executes or
evaluates anything it downloaded other than the verified program you asked it to run with `--run`.

| Option | Meaning |
|---|---|
| `--version X.Y.Z` | install this release instead of the latest (a `v` prefix is accepted) |
| `--dir DIR` | install the binary into `DIR` (absolute path; default `~/.local/bin`) |
| `--system` | system-wide: `/usr/local/bin` (+ `/usr/local/share` entries), using `sudo` unless you are root |
| `--format tar\|appimage\|dmg` | asset to install. Default `tar` = the bare binary (`.tar.gz`). Linux: `tar`, `appimage`. macOS: `tar`, `dmg` (copies `synofurb.app` to `~/Applications`, or `/Applications` with `--system`) |
| `--no-desktop-entry` | skip the Linux desktop entry and icon |
| `--pubkey KEY` | minisign public key to verify with (the release's key is built into the script) |
| `--insecure-skip-signature` | accept the SHA-256 alone. **Integrity, not authenticity.** Required if `minisign` is not installed |
| `--uninstall` | remove the binary, desktop entry, icon and app bundle this installer created (your settings stay) |
| `--dry-run` | show the resolved version, asset, URLs and destinations; download and change nothing |
| `--run ARGS...` | must be last: after installing, run `synofurb ARGS...` with the terminal attached, and exit with its status |
| `-h`, `--help` | usage |

Without `minisign` installed the script stops and tells you how to install it (`brew install minisign`, `sudo apt install minisign`,
`sudo dnf install minisign`, ...). To proceed without it you must say so explicitly with `--insecure-skip-signature`.

Requirements: `curl` or `wget`, `tar`, `sha256sum` or `shasum` or `openssl`; Linux with **glibc 2.39 or newer** (Ubuntu 24.04, Debian 13, Fedora 40+;
the Linux builds are made on Ubuntu 24.04) and a Vulkan-capable GPU stack (`libvulkan1`, `libxkbcommon`, Wayland or X11 libraries; the `.deb`/`.rpm`
declare them). macOS 12 or newer. Alpine/musl is not supported. If `~/.local/bin` is not on your `PATH` the script prints the line to add.

Uninstall: `curl -fsSL .../install.sh | sh -s -- --uninstall` (add `--system` if you installed with it).

### Windows (`install.ps1`, PowerShell 5.1 or 7)

```powershell
irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1 | iex
```

A piped script cannot take parameters, so for options build a script block:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1))) -Version 0.2.0 -Run scan
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1))) -User -AddToPath
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1))) -Uninstall
```

| Parameter | Meaning |
|---|---|
| `-Version X.Y.Z` | release to install (default: latest) |
| *(default)* | downloads the **MSI** and installs it silently (`msiexec /i ... /qn /norestart`) for all users. The script itself runs unelevated; only `msiexec` is started with a UAC prompt, and only if you are not already an administrator. The MSI adds Start Menu and desktop shortcuts |
| `-User` | **per-user, no administrator rights**: downloads the portable zip, extracts it to `%LOCALAPPDATA%\Programs\synofurb`, adds a Start Menu shortcut and an "Apps & features" entry for your account. (The MSI is machine-wide only, so this is not an `ALLUSERS=""` MSI install.) |
| `-Portable` | download the zip and just extract it to `-Dir` (default `%LOCALAPPDATA%\Programs\synofurb-portable`); no shortcuts, no registry |
| `-Dir PATH` | install location (MSI: passed as `INSTALLDIR`) |
| `-AddToPath` | with `-User`/`-Portable`: add the folder to your user `PATH` |
| `-Run scan` / `-Run install,--host,192.168.1.20` | run `synofurb ...` after installing (array; separate arguments with commas) |
| `-Uninstall` | `msiexec /x` the MSI product and/or remove the per-user folder, shortcut and entry |
| `-DryRun` | show what would happen, change nothing |
| `-PublicKey KEY`, `-RequireSignature`, `-InsecureSkipSignature` | minisign handling, see below |

The script verifies the SHA-256 against `SHA256SUMS`. When `minisign` is on `PATH` (`scoop install minisign`, `choco install minisign`) it also verifies the
signature; without it you get a loud warning that only integrity was checked. Add `-RequireSignature` to make that an error. TLS 1.2 or newer is forced.
Unsigned builds (before code-signing certificates exist) trigger Microsoft SmartScreen: *More info -> Run anyway* is expected until then.

## Packages by hand

Download from the [releases page](https://github.com/andreaswiren/synofurb/releases), [verify](#verifying-a-download), then:

| Platform | File | Install |
|---|---|---|
| Windows | `synofurb-<v>-windows-x86_64.msi` (or `-aarch64`) | double-click, or `msiexec /i file.msi /qn` |
| Windows | `...-windows-<arch>.exe` | NSIS setup; per-user or per-machine; silent: `file.exe /S /CURRENTUSER` or `/S /ALLUSERS` |
| Windows | `...-windows-<arch>.zip` | extract anywhere; run `synofurb.exe` |
| macOS | `synofurb-<v>-macos-aarch64.dmg` (Apple silicon) / `-x86_64.dmg` (Intel) | open, drag to Applications. Unsigned builds: right-click -> Open the first time |
| macOS | `...-macos-<arch>.pkg` (if present) | double-click |
| macOS | `...-macos-<arch>.tar.gz` | the bare binary; `tar -xzf file.tar.gz synofurb` |
| Linux | `synofurb-<v>-linux-x86_64.deb` / `-aarch64.deb` | `sudo apt install ./synofurb-<v>-linux-x86_64.deb` |
| Linux | `...-linux-<arch>.rpm` | `sudo dnf install ./file.rpm` |
| Linux | `...-linux-<arch>.AppImage` | `chmod +x file.AppImage && ./file.AppImage` (needs FUSE 2: `libfuse2`/`fuse-libs`; or `APPIMAGE_EXTRACT_AND_RUN=1 ./file.AppImage`) |
| Linux | `...-linux-<arch>.tar.gz` | the bare binary (+ icon); `tar -xzf file.tar.gz` |

Once installed, `synofurb self-update --check` finds newer releases; it verifies the same SHA-256 and signature (see [UPDATES.md](UPDATES.md)).

## Convert a NAS headless from a Linux/macOS box

Everything the wizard does is available as CLI subcommands, so one command line on any computer on the NAS's network installs the app and runs the
conversion. First look (read-only), then do it. The SSH password is read from an environment variable so that it does not land in the process list;
type it without echo and hand it only to `sh`:

```sh
read -rs SYNOFURB_PASSWORD; export SYNOFURB_PASSWORD        # the NAS admin's SSH password (nothing is echoed)

# 1. find it
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- --run scan

# 2. read-only: inspect the unit and print the plan with the exact confirmation phrase
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- --run plan --host 192.168.1.20 --user admin --mode keep

# 3. install. DESTROYS DATA. Needs all three explicit confirmations; --confirm takes the phrase printed by `plan`
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- \
  --run install --host 192.168.1.20 --user admin --mode keep \
  --hostname nas --username nasadmin --have-backup --accept-data-loss --confirm "ERASE DS918+"
```

After the first run `synofurb` is installed in `~/.local/bin`, so later you can call it directly. `--run` hands the terminal to the program (prompts, typed
confirmations and the live log work) and exits with the program's exit status, so it can be scripted. Add `--version 0.2.0` to pin the release, and
`--insecure-skip-signature` only if you cannot install `minisign`. Use `synofurb install --help` for every option (RAID level, system size, kernel, method
`dom`/`kexec`/`kit`, `--agent-dir`). The boot agent files are also attached to every release for offline use as `synofurb-agent-<amd64|arm64|armhf>-<version>.vmlinuz`
and `.initrd`; the CLI looks for them as `agent-<arch>.vmlinuz` / `agent-<arch>.initrd` in `--agent-dir`.

## The NAS management plane on an existing Debian box

Converted NAS units get the management plane automatically. To put it on any other Debian/Ubuntu machine with systemd (it needs `python3`, `tar`,
`curl`, and `minisign` to verify the download):

```sh
sudo apt-get install -y minisign python3 curl        # once
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/apps/mgmt-plane/install.sh | sudo sh
```

It downloads `synofurb-mgmt-<version>.tar.gz` from the latest release (the management plane has its own version, `apps/mgmt-plane/VERSION`), verifies the signature
and SHA-256, installs it to `/opt/synofurb-mgmt`, installs and enables `synofurb-mgmt.service` and starts it: **https://\<host\>:8443** (self-signed certificate; compare
`curl -k https://localhost:8443/api/tls-fingerprint`). It is idempotent: running it again upgrades or re-applies. State (certificate, sessions) in `/var/lib/synofurb-mgmt`
survives upgrades and uninstall.

| Option | Meaning |
|---|---|
| `--release vA.B.C` | take the management plane shipped with that synofurb release (default: latest release) |
| `--version X.Y.Z` | take that management-plane version (searches the published releases) |
| `--root DIR` | stage everything under `DIR` instead of `/` (images, chroots, tests); no `systemctl` is run |
| `--no-start` | install the files and unit but do not enable/start the service |
| `--pubkey KEY`, `--insecure-skip-signature` | as for `install.sh` |
| `--uninstall` / `--purge` | remove the service and `/opt/synofurb-mgmt` / also delete `/var/lib/synofurb-mgmt` and `/var/log/synofurb-mgmt` |
| `--dry-run` | print the plan only |

```sh
curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/apps/mgmt-plane/install.sh | sudo sh -s -- --uninstall
```

## Verifying a download

```sh
# 1. authenticity: the checksum list was signed by the project key (public key: see docs/RELEASING.md)
minisign -Vm SHA256SUMS -P RWQ...the-public-key...
# 2. integrity: the file you downloaded is the one in the list
sha256sum --ignore-missing -c SHA256SUMS            # macOS: shasum -a 256 -c SHA256SUMS 2>/dev/null | grep -v FAILED
```

Both must pass. Windows PowerShell: `(Get-FileHash .\synofurb-0.2.0-windows-x86_64.msi -Algorithm SHA256).Hash` and compare with the line in `SHA256SUMS`.

## Troubleshooting

* `could not query ... releases/latest`: no release has been published yet, you are offline, or the GitHub API rate limit was hit - use `--version X.Y.Z`.
* `no asset for linux/aarch64`: that platform was not built for the release; the message links the release page.
* `SHA-256 MISMATCH` / `SIGNATURE VERIFICATION FAILED`: the download is corrupt or not authentic. Nothing was installed. Do not retry with `--insecure-skip-signature`; report it.
* `this installer has no minisign public key built in yet`: you are running a development copy of the script (the release procedure pastes the key in); pass `--pubkey`.
* Behind a proxy: `curl` honours `https_proxy`; set it for both `curl` and `sh`.
* Mirrors and tests: `SYNOFURB_API_BASE` and `SYNOFURB_DL_BASE` point the scripts at another server (`tests/install/run-tests.sh` uses a local fake one).
