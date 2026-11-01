# synofurb on the command line

Everything the desktop app does is also a command. There are two programs built from the same code:

| Program | What it is | Needs |
|---|---|---|
| `synofurb` | the desktop app **and** every command below (`synofurb` alone opens the window; `synofurb gui --demo` opens it with a sample NAS) | the GUI system libraries (X11/Wayland, Vulkan, freetype, ...) |
| `synofurb-cli` | **only** the commands, no window | nothing but the C library: it starts on a headless server, over SSH, in a container, in CI |

Both share one engine (`synofurb-core`), one data directory (inventory, logs, keys, backups, caches) and the same behaviour: whatever you did in
the app shows up in the CLI and vice versa. Examples below say `synofurb-cli`; `synofurb` accepts the same words. Install it with
`curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- --cli` or, on Windows,
`irm https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.ps1 -OutFile i.ps1; .\i.ps1 -Cli` ([INSTALL.md](INSTALL.md)).

> Reinstalling a NAS repartitions its disks. **There is no guarantee that any data survives**, including volumes marked "preserved". The CLI will not
> start an install without three separate confirmations (a verified backup, acceptance of data loss, the typed phrase `ERASE <MODEL>`). No flag,
> environment variable or `--yes` skips them.

Contents: [Quick start](#quick-start) - [Conventions](#conventions) - [Exit codes](#exit-codes) - [Commands](#commands) - [JSON documents](#json-documents) -
[Event stream](#event-stream---log-format-json) - [The wizard](#the-wizard) - [Safety gate](#the-safety-gate) - [Headless, CI, jump hosts](#headless-ci-and-jump-hosts) -
[Offline use](#offline-use) - [Updating](#updating) - [GUI parity](#gui-parity) - [Troubleshooting](#troubleshooting) - [Testing](#testing)

## Quick start

```sh
synofurb-cli scan                                  # find units (saved to the inventory)
synofurb-cli compat DS923+                         # what is known about the model
export SYNOFURB_PASSWORD=...                       # the NAS admin's SSH password (or --key FILE, or answer the prompt)
synofurb-cli inspect --host 192.168.1.20           # read-only: disks, arrays, firmware, serial console, compatibility verdict
synofurb-cli plan --host 192.168.1.20 --mode keep  # what would be destroyed, what kept; prints the confirmation phrase
synofurb-cli install --host 192.168.1.20 --mode keep --dry-run --out ./review   # render everything, change nothing
synofurb-cli install --host 192.168.1.20 --mode keep --have-backup --accept-data-loss --confirm "ERASE DS918+"
# or all of it, guided:
synofurb-cli wizard
# afterwards, look after the web UI on the converted unit (docs/MANAGE.md):
synofurb-cli manage health --host 192.168.1.20     # arrays, disks, volumes, services; exit 0 ok, 8 warn, 9 crit
synofurb-cli manage update --host 192.168.1.20     # push the shipped web UI version; rolls back by itself on failure
```

No NAS at hand? `--facts demo` uses a built-in 4-bay DS918+ for every read-only command and `wizard --demo` rehearses the whole flow:

```sh
synofurb-cli plan --facts demo --json | jq .summary
synofurb-cli install --facts demo --dry-run --out ./review && less review/stage2.sh
synofurb-cli wizard --demo
```

## Conventions

**Global flags** (before or after the command): `--log-format text|json` (env `SYNOFURB_LOG_FORMAT`), `--no-color`, `-q/--quiet`, `-v/-vv`,
`--non-interactive` (alias `--no-input`), `-y/--yes`, `-h/--help`, `-V/--version`. Every command has `--help` with examples.

**stdout is data, stderr is for people.** Tables you asked for, `--json` documents and, with `--log-format json`, the NDJSON event stream go to stdout.
Banners, warnings, prompts, progress and error messages go to stderr. So `synofurb-cli plan ... --json | jq` and `... 2>/dev/null` do what you expect.

**Colour** only on a terminal, never when `NO_COLOR` is set (and non-empty), with `--no-color`, or with `TERM=dumb`. The progress bar is drawn only when stderr is
a terminal; otherwise there is one plain line per step and per 10 % of a transfer. All output is ASCII.

**Verbosity.** Default: application messages and installer warnings. `-v`: also the installer's own log lines and SSH warnings. `-vv`: every remote command.
`-q`: errors and the final result only. The full debug log of a run is always written to `<data dir>/logs/synofurb-<time>.log` (`logs show`).

**Prompts** appear only when stdin *and* stderr are terminals and neither `--non-interactive` nor `CI=true` is set. Otherwise a missing answer is an error that
names the flag to use. Prompts are line based and work on a serial console or `TERM=dumb`.

**Secrets are never arguments.** There is no `--password VALUE` anywhere. The NAS login password comes from `$SYNOFURB_PASSWORD` (name changeable with
`--ssh-password-env`), `--ssh-password-file FILE` (first line) or a prompt; with `--key FILE` an unencrypted OpenSSH private key is used instead. The sudo password
defaults to the SSH password, or `$SYNOFURB_SUDO_PASSWORD`. The password of the *new* account is `--password-file`, `--password-env VAR`, `--prompt-password` or generated
(default). Every secret is registered with the logger and replaced by `***` in the log file and on screen. The generated password is shown once at the end of an install (and
included in the `result` event); a password you chose is never printed back.

**Ctrl-C.** Before the point of no return the first Ctrl-C cancels at the next safe point ("no disk has been changed"), the second stops waiting. After it, Ctrl-C says
plainly that it is too late: the disks have been changed and the installer on the NAS carries on by itself; a second Ctrl-C only stops *watching*. SIGTERM and SIGHUP
(a dropped SSH session) count like Ctrl-C. Run long installs in `tmux`/`screen`, or with `nohup ... --log-format json > run.ndjson`.

**Files** (`synofurb-cli config paths`; Linux shown, `XDG_DATA_HOME` moves it; macOS `~/Library/Application Support/dev.synofurb.synofurb`; Windows `%APPDATA%\synofurb\synofurb\data`):

| Path | Content |
|---|---|
| `~/.local/share/synofurb/logs/` | one debug log per run, plus the generated `stage2-*.sh` and `plan-*.json` of installs |
| `.../inventory.json` | units found by `scan` / added by `inventory add` |
| `.../keys/` | SSH keys generated for new systems and for the boot agent (mode 0600) |
| `.../known_hosts.json` | pinned SSH host keys of the units, `host:port` -> `SHA256:` fingerprint (`host-key list`) |
| `.../dom-backups/`, `.../backups/` | boot flash images (`synofurb-dom-backup-<serial or address>-<time>.img` + `.sha256` + `.json` with the unit's serial/model), partition-table backups |
| `.../cache/` | downloaded Debian installer files, boot agent |
| `.../compat-models.json` | the compatibility list refreshed with `compat --refresh` |
| `~/.cache/synofurb/updates/` | downloaded updates |

**Environment:** `SYNOFURB_PASSWORD`, `SYNOFURB_SUDO_PASSWORD`, `SYNOFURB_LOG_FORMAT`, `SYNOFURB_AGENT_DIR` (boot agent files), `NO_COLOR`, `TERM`, `CI`, `XDG_DATA_HOME`, `HOME`.
Settings are not read from the environment beyond this list; there is no config file to edit (`config show` reports what is in effect).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | error (network, SSH, bad file, a job other than an install failed) |
| 2 | usage: wrong flags, a flag the unit cannot honour (for example `--method dom` without a boot flash), no terminal for a prompt, invalid value |
| 3 | **refused**: the safety gate was not satisfied, or a confirmation was declined. Nothing was changed |
| 4 | the plan has errors (`plan` and `install`); the plan is still printed. Nothing was changed |
| 5 | unsupported hardware: Debian 13 cannot run on this platform (`inspect`, `plan`, `install`, `wizard`), or `compat MODEL` for a model listed as unsupported |
| 6 | the install failed **after** the point of no return: the disks were changed. Read the log and [RECOVERY.md](RECOVERY.md) |
| 7 | the install failed or was cancelled **before** the point of no return: the disks are untouched |
| 8 | `manage health` only: the unit answered and its verdict is **warn** (something needs attention). The report is printed |
| 9 | `manage health` only: the unit answered and its verdict is **crit** (a degraded array, a failed disk, a full volume). The report is printed |
| 10 | the **boot agent never answered** after the reboot request (`install` on a boot-flash unit, `boot-manager enter`). The message states both outcomes (agent started: its watchdog resets the default to DSM and reboots, counted from the agent's start; agent never ran: the unit may boot the agent after every power cycle) and the manual recovery. It does **not** say DSM boots normally. Nothing was reset. See [RECOVERY.md](RECOVERY.md). In `install` no disk had been changed |
| 11 | the **SSH host key was refused**: it changed since it was pinned, does not match `--host-key`, or a first-time key was not trusted. The message names the fingerprints. **Nothing was sent**, not even the password |

With `--log-format json` the failure is also the last event on stdout: `{"event":"failure","kind":"refused","exit_code":3,...}`. Usage errors found by the argument parser itself
(unknown flag) print clap's message on stderr and exit 2.

## SSH host keys

Every command that logs in to a unit checks the unit's SSH host key **before** the password (or the key signature) is sent.

| Flag | Meaning |
|---|---|
| (none) | the key must match the one pinned for `host:port` in `known_hosts.json` (data directory). A **first** key is shown with its fingerprint and asked about on a terminal ("Trust this NAS and remember its key?"); without a terminal the connection is refused with exit 11 and the fingerprint is printed |
| `--host-key SHA256:...` / `@FILE` | the unit must present exactly this key (`ssh-keygen -l` output; nothing is remembered) |
| `--known-hosts FILE` | use another pin file |
| `--accept-new-host-key` | for scripts: pin a first-time key without asking. A **changed** key is still refused |
| `--insecure-host-key` | check nothing. Prints a loud warning on every run; lab networks only |

The first-contact question has 180 s to be answered (`timing::HOST_KEY_ANSWER_TIMEOUT_SECS`); the 20 s connect timeout does not run while you compare the fingerprint. No answer means refused, and an answer that arrives late pins nothing. A first-time key is written to the pin file only after the connection's handshake completed.
A changed key is a hard error (exit 11) that names the pinned and the presented fingerprint. If you reinstalled or replaced the unit on purpose, `synofurb-cli host-key forget --host H [--port P]`
removes the pin; `host-key list [--json]` shows them. A finished install gives the unit a new key (it is Debian now): the key the app saw while verifying the new system replaces the pin of that address (logged), unless you use `--host-key`.
The boot agent has a new key at every boot; it is reached with a per-unit client key and no password is ever sent to it, so its key is not checked. `wizard` asks the same first-contact question before it asks for a password.

## Commands

### scan, inventory

```sh
synofurb-cli scan [--timeout SECONDS] [--no-sweep] [--json | --ndjson] [--all] [--no-save]
synofurb-cli inventory list [--json] [--all]
synofurb-cli inventory add --ip ADDRESS [--name NAME]
synofurb-cli inventory forget (PATTERN | --all)       # PATTERN: address, MAC, serial, model or host name
```

`scan` runs the app's discovery methods at once (Synology Assistant broadcast, mDNS, SSDP and a /24 port sweep unless `--no-sweep`), prints units as they appear when stderr is a
terminal, then a summary, and merges the result into the saved inventory (`--no-save` to skip). `--json` prints `{"devices":[...]}` at the end; `--ndjson` one device object per
line as it is found (the last line per unit is the most complete). `-v` lists which methods worked. `--all` also lists hosts not identified as Synology.

### inspect

```sh
synofurb-cli inspect --host ADDRESS [--user admin] [--port 22] [--key FILE] [--json] [--save facts.json]
synofurb-cli inspect --facts FILE|demo [--json]
```

Read-only. Connects, runs the inspection script and prints: model, serial, DSM version, CPU and vendor, memory, firmware (BIOS/UEFI/U-Boot/boot flash), boot-flash details,
kexec availability, the serial console DSM uses (including raw `uart8250,mmio32,...` consoles), network ports, platform support, the **compatibility verdict** from the list,
disks, RAID arrays, logical volumes, mounted volumes, and the confirmation phrase. `--save` writes the facts to a file that every `--facts` option accepts later
(also accepted: the `--json` document and raw inspection output). Exit 5 when the hardware is unsupported.

### compat

```sh
synofurb-cli compat [MODEL] [--refresh] [--json]
synofurb-cli compat --list [--status verified|community|untested|known-issues|unsupported] [--json]
```

Looks `DS923+`, `ds923plus`, `RS3618xs` up in the embedded list (or the cached refresh). Without a model: a summary by status. `--refresh` fetches the newest list from the repository first
(network trouble is reported, the old list stays). A model that is not listed is "untested" (exit 0); a model listed as unsupported exits 5.

### plan

```sh
synofurb-cli plan (--host ADDRESS ... | --facts FILE|demo)
    [--mode wipe|keep] [--keep md2[,md3]] [--system-gib N] [--copies N] [--raid auto|none|0|1|5|6|10|shr1|shr2] [--free-percent 0-90] [--kernel newest|stable] [--json]
```

Every planner knob of the app's Storage step: `--mode keep` reclaims only DSM's system and swap and preserves the data arrays (`--keep` selects which; default all);
`--system-gib 0` means "recommended"; `--copies` is the number of disks carrying a copy of the system (default automatic); `--raid` the level of the new data volume (`shr1`/`shr2` = Synology-Hybrid-RAID-like slices of mdadm arrays in one LVM group that use disks of different sizes fully, tolerating 1 or 2 failed disks; they need at least 2 / 4 wiped disks and cannot be combined with preserved arrays, see [RAID.md](RAID.md));
`--free-percent` the share of the data volume group left unallocated; `--kernel` is only echoed (the kernel channel does not change the layout). Prints what will be destroyed, what is preserved, warnings, errors and the confirmation phrase. Exit 4 when the plan has
errors, 5 for unsupported hardware.

### install

```sh
synofurb-cli install --host ADDRESS [connection] [plan knobs] [system] [boot] [source] [gate] [--dry-run [--out DIR | --json]] [--save-logins FILE]
```

Plan knobs are the same as for `plan`. System settings:

| Flag | Default | |
|---|---|---|
| `--hostname`, `--username` | `nas`, `nasadmin` | host name and administrator account of the new system |
| `--password-file F` / `--password-env VAR` / `--prompt-password` / `--generate-password` | generate | where the new password comes from |
| `--authorize-key generate\|none\|FILE` | `generate` | SSH key for the account: a new ed25519 key saved under `keys/`, none, or a public key file |
| `--timezone`, `--locale`, `--keymap` | `UTC`, `en_US.UTF-8`, `us` | |
| `--mirror HOST`, `--mirror-dir PATH` | `deb.debian.org`, `/debian` | |
| `--package PKG[,PKG]` | none | extra Debian packages |
| `--serial TTY\|RAW`, `--serial-speed BAUD` | what DSM uses | `ttyS0`, or a raw console such as `uart8250,mmio32,0xfedc9000,115200n8` (AMD boards) |
| `--dtb FILE.dtb` | known boards | device tree for ARM U-Boot units |
| `--no-mgmt` | management plane on | do not install the web management plane (port 8443) |
| `--kernel newest\|stable` | `newest` | Linux 7.x from trixie-backports, or Debian's 6.12 LTS |

Boot and source:

| Flag | Default | |
|---|---|---|
| `--method auto\|dom\|kexec\|kit` | `auto` | auto = `dom` for units with an editable boot flash, else `kexec` if DSM has it, else `kit`. A method the unit cannot do is refused (exit 2) |
| `--agent-dir DIR`, `--backup-dir DIR` | `$SYNOFURB_AGENT_DIR`/downloaded, `<data>/dom-backups` | boot agent files; where the boot flash image goes |
| `--kexec FILE` | bundled/auto | static kexec binary for the NAS architecture ([kexec.md](kexec.md)) |
| `--kit-dir DIR` | `synofurb-kit` | output folder of `--method kit` |
| `--no-firmware` | firmware on | leave out non-free firmware packages |
| `--no-verbose-installer` | verbose | no debug logging inside the Debian installer |
| `--source online\|offline`, `--bundle DIR` | online | offline: serve everything from a bundle ([Offline use](#offline-use)); `--bundle` (alias `--offline-bundle`) implies offline |

What an install does, in order: connect and inspect, build the plan (exit 4/5 on errors or unsupported hardware), complete the [safety gate](#the-safety-gate), re-inspect and compare
the disks with the plan, back up the boot flash (units with one) and the first/last MiB of every disk the plan touches, stage and serve the installer, boot it, follow the installer's log
(`step n/12`), wait for the new system, then print the logins. Cancel with Ctrl-C until the point of no return.

**`--dry-run`** does everything up to rendering and prints (or with `--out DIR` writes) the generated artefacts without staging, serving or booting anything: the stage-2 installer script,
the payload manifest (every file with mode, size, SHA-256) and the extracted payload, the plan, the preseed, the installer command line and the install configuration. The NAS is only
*read* (or not contacted at all with `--facts FILE|demo`). The password hash and the SSH key in the output are placeholders. The script calls back to this computer's address towards the
NAS (a documentation address when offline) on placeholder ports; a real run picks free ones. `--json` prints one document; `--out` refuses a non-empty directory unless `--yes`.
No gate is needed for a dry run.

On success the logins are printed to stdout (and written to `--save-logins FILE`, mode 0600):

```
=== INSTALLED ===
hostname:   nas
address:    192.168.1.30
login:      nasadmin / Gen3ratedPw...        (only when generated; otherwise "(the password you chose)")
ssh:        ssh nasadmin@192.168.1.30   (host key SHA256:...)
ssh key:    ~/.local/share/synofurb/keys/nas_ed25519   (SHA256:...)   use: ssh -i ... nasadmin@192.168.1.30
management: https://192.168.1.30:8443/  (self-signed certificate)
serial console: ttyS0 at 115200 baud (also on USB serial adapters)
debug log:  ~/.local/share/synofurb/logs/synofurb-....log
```

### boot-manager, recover, backups

```sh
synofurb-cli boot-manager install --host H [--agent-dir DIR] [--backup-dir DIR] [--agent-hostname NAME] [--arm] [--yes]
synofurb-cli boot-manager enter   --host H [--yes]
synofurb-cli boot-manager status  --host H [--json]
synofurb-cli recover              --host H
synofurb-cli recover status       --host H [--json]
synofurb-cli recover wait         --host H [--timeout MINUTES] [--port 22] [--json]
synofurb-cli recover boot-dsm     --host H [--reboot] [--yes]
synofurb-cli recover remove       --host H --yes
synofurb-cli recover restore      --host H --backup FILE.img [--allow-other-unit] --yes
synofurb-cli backup-dom           --host H [--backup-dir DIR]
synofurb-cli disk-backup          --host H [--disk sata1,sata2] [--out-dir DIR]
```

*Boot manager mode* adds a small SSH-controllable boot environment to the internal boot flash of x86 units; DSM stays the default entry ([BOOT.md](BOOT.md)). `install` backs the whole flash up first
(verified against a hash computed on the NAS); `enter` arms the agent, reboots DSM and waits until the agent answers, then prints the `ssh -i` line to control it; `status` reports whether the
unit can do this and what the on-unit tool says. `install` and `enter` change the boot flash, so they need `--yes` or a terminal to confirm.
`recover` alone sets the default boot entry to DSM (works from DSM, an installed Debian, or the agent; it does not reboot) and prints what to do if DSM's system partition is gone ([RECOVERY.md](RECOVERY.md)). The subcommands:

* `recover status` **reads** the unit: running system, whether the boot menu could be read, the default entry, synofurb's entries, agent files, the flash backup on the unit, plus the images saved on this computer. Unreadable is printed as `NOT READ` / `NOT KNOWN` (JSON: `state.read.result` = `tool_missing|refused|failed`, `state.default_entry.kind` = `unknown`, `pristine: null`), never as a DSM default. `--json`: `{"schema":1,"host","pristine":true|false|null,"state":{read,source,running,default_entry,entries,our_entries,agent_present,flash_backup,agent,notes},"local_backups":[...]}`.
* `recover boot-dsm` sets the default to DSM and reads it back. With `--reboot` (which needs `--yes` or a terminal) it then asks the unit to reboot and reports **"reboot requested"** only; if the read-back did not show DSM no reboot is sent. Follow it with `recover wait`.
* `recover wait` probes the unit every few seconds (no login) and exits 0 only when DSM answers (SSH banner and web port 5000/5001); otherwise exit 1 with what was seen last (loader-only, still the agent, other system, nothing). `--timeout` is in minutes (default 15). Ctrl-C stops it.
* `recover remove` and `recover restore` change the boot flash and need `--yes` (or a terminal); both read the result back and say so only when it verified. `restore` checks the image first (size, its `.sha256` file) and **refuses an image made from another unit** (the `<image>.json` written next to every backup records serial, model and address; another serial, or another model when no serial is readable, is exit 1 `belongs to another unit` before anything is sent; `--allow-other-unit` overrides; images that cannot be attributed, such as old ones or a unit whose serial is unreadable, go ahead with a warning). Then it uploads it to a private directory on the unit, compares hashes there, then runs `synofurb-dom restore`, reads the flash back from the unit (`head -c <size> <device> | sha256sum`) and compares it with the image (`readback_verified` in the `--json` result is true only if that matched; a mismatch is an error, "could not read back" is reported as NOT VERIFIED); it works only while the unit answers over SSH (otherwise restore the flash from another computer: [RECOVERY.md](RECOVERY.md)).
`backup-dom` only takes the boot flash image. `disk-backup` saves the first and last MiB of the disks (partition tables, **not** your files) with a README on how to put them back.

### manage

Day two, after the install: health and updates of the web management UI that runs on the converted unit ([MANAGE.md](MANAGE.md) has the contract with the unit, the verdict rules and the update flow).

```sh
synofurb-cli manage health   --host H [--json]
synofurb-cli manage version  --host H [--json]
synofurb-cli manage update   --host H [--from-bundle DIR] [--force] [--dry-run] [--yes]
synofurb-cli manage rollback --host H [--yes]
synofurb-cli manage open     --host H
```

All but `open` take the same login options as `inspect` (`--user`, `--port`, `--key`, `--ssh-password-file`, `$SYNOFURB_PASSWORD`, `$SYNOFURB_SUDO_PASSWORD`); the user does not have to be
root. They read or change only `/opt/synofurb-mgmt`, its systemd unit and temporary files below `/tmp`: never disks, arrays or the boot flash.

* `health` asks the unit (`synofurb_mgmt.py health --json`) and prints the verdict (`OK`/`WARN`/`CRIT`), a problem list of at most five one-sentence lines, then arrays, disks, volumes, services
  and pending Debian updates. **Exit 0 ok, 8 warn, 9 crit** (1 when the unit cannot be asked: management plane missing, too old, unreachable). `--json` prints
  `{"schema":1,"host","overall","summary":{overall,reported,headline,bullets[{level,text}],problems,degraded_arrays,rebuilding_arrays,failed_disks,services_down,volumes_nearly_full,security_updates,pending_updates},"health":{...the unit's report...},"web_ui"}`.
* `version` compares the version on the unit with the one shipped in this program: `{"schema":1,"host","installed":{present,version,script_sha256},"bundled":{version},"relation":"same|nas_older|nas_newer|unknown","update_available","decision","message","health_error","overall"}`. Always exit 0.
* `update` reads the unit first and prints the plan (`up to date`, `update 0.3.0 -> 0.4.0`, ...). If nothing is to be done it says so and exits 0 without asking. Otherwise it asks once (this is **not** a data-loss
  gate, so `--yes`/`-y` skips it; without a terminal and without `--yes` it exits 3 and changes nothing), uploads the files, has the unit verify, swap and restart, checks the unit again and prints the
  result. `--dry-run` connects, prints the plan and stops. `--force` reinstalls identical files or installs an older version (without it a newer unit is left alone, exit 3).
  `--from-bundle DIR` uses a folder instead of the built-in copy: an offline bundle made by `bundle create` (or its `mgmt/` folder; `MANIFEST.sha256` and the bundle's `manifest.json` are both checked, and a bundle
  without `mgmt/` is refused with a hint), or a folder with the files themselves (`make-update-bundle.py`; its `MANIFEST.sha256` is verified). A mismatch is a usage error (exit 2) before anything is contacted. Works fully offline.
  A failed update exits **1** and the message says what state the unit is in (back on its old version, a different version, or "could not verify": see MANAGE.md). The web UI is unavailable for a few seconds.
* `rollback` returns to the version before the last update (confirmation as for `update`).
* `open` prints `https://H:8443` (IPv6 gets brackets) and connects to nothing. The certificate is self-signed.

With `--log-format json` the progress is the usual `step`/`log` events (`step 1/5 Reading ... 5/5 Cleaning up`; the unit's own step lines arrive as `log` events `NAS swap: ok - ...`) and the last event is a `result`
whose `data` is the full report (`outcome`, `method`, `from_version`, `to_version`, `rollback`, `remote_steps[]`, `failure`, `health_after`, `summary_after`). Ctrl-C before the update is applied cancels; afterwards
the unit finishes by itself.

### logs, config, version, completions

```sh
synofurb-cli logs list [--json] [--all]        # newest first; --all adds stage2-*.sh and plan-*.json
synofurb-cli logs show [N] [--tail LINES]      # N counts from the newest (1)
synofurb-cli logs path [--latest]              # the folder (or the newest file)
synofurb-cli config show [--json]              # version, paths, update channel, compat list, terminal, environment (secrets: set/unset only)
synofurb-cli config paths [--json]
synofurb-cli version [--json]                  # version, git commit if known, target triple, features
synofurb-cli completions bash|zsh|fish|powershell
```

### fetch-installer, bundle, self-update

```sh
synofurb-cli fetch-installer amd64|arm64|armhf [--suite trixie] [--firmware] [--dtb FILE.dtb] [--json]
synofurb-cli bundle create [--arch amd64[,arm64,armhf]|all] --out DIR [--suite trixie] [--kernel newest|stable] [--no-firmware] [--package PKG] [--no-agent]
synofurb-cli bundle verify DIR
synofurb-cli bundle info DIR [--json]
synofurb-cli self-update [--check [--json]] [--channel stable|prerelease]
```

`fetch-installer` downloads the Debian installer kernel and initrd and verifies them along the Release -> SHA256SUMS -> file hash chain. `bundle` is described under [Offline use](#offline-use) (sizes: amd64 about 440 MiB, plus about 470 MiB of installer firmware shared by all architectures).

## JSON documents

All `--json` documents carry `"schema": 1` and are printed with stable keys; new keys may be added, existing ones keep their meaning.

**`scan --json`, `inventory list --json`**: `{"devices":[{"ips":["192.168.1.20"],"mac":"00:11:32:..","hostname":"","model":"DS923+","serial":"","dsm_version":"","platform":"","http_port":5000,"https_port":5001,"synology":true,"synofurb_debian":false,"sources":["mdns","udp"],"last_seen":"2026-11-01 12:00:00"}]}`

**`compat MODEL --json`**: `{"schema":1,"query":"DS923+","found":true,"status":"community","status_label":"Community-reported","explanation":"...","blocking":false,"entry":{model, series, platform, cpu, arch, boot, status, evidence[], notes, ...}}`
(`found:false` and `entry:null` for unlisted models, status `untested`). Without a model: `{"schema":1,"models":272,"updated":"2026-11-01","origin":"embedded|cache","by_status":{"verified":0,"community":1,"untested":202,"known-issues":0,"unsupported":69}}`.

**`inspect --json`**: `{"schema":1,"facts":{...},"arch":"amd64","firmware":"legacy BIOS","cpu_vendor":"Intel","serial_console":{"tty","speed","raw","kernel_arg"},"platform":{"platform","arch","soc","support","note"},
"compat":{"status","status_label","headline","details":[],"blocking","entry"},"auto_boot_method":"kexec","confirm_phrase":"ERASE DS918+","supported":true}`. `facts` is the engine's `DeviceFacts`: `model, unique, dsm_version,
serial, hostname, arch, mem_kib, cpu, firmware, kexec, cpu_vendor, dom{...}|null, cmdline, is_root, nics[], disks[{name, model, serial, sectors, rotational, removable, table, parts[]}], arrays[{name, level, state, sectors, members, role, vg}], lvs[], mounts[]`.

**`plan --json`**: `{"schema":1,"model","serial","confirm_phrase","ok","plan_ok","supported","errors":[],"warnings":[],"summary":[],"destroyed":[],"preserved":[{what,detail}],"platform":{},"compat":{},"kernel":{"channel","label"},"plan":{...}}`; `plan` is the engine's full plan
(`mode, arch, firmware, request, system_disks, disks[] with the new partitions, arrays[], vgs[], boot, kept_arrays, sizes{min/recommended/max/chosen_system_mib, ...}`). `ok` is true only when the plan has no errors **and** the hardware is supported.

**`version --json`**: `{"schema":1,"name":"synofurb-cli","version":"0.2.0","git":"abc1234"|null,"target":"x86_64-unknown-linux-gnu","os":"linux","arch":"x86_64","profile":"release","features":["cli","bundle"],"gui":false}`

**`install --dry-run --json`**: `{"schema":1,"dry_run":true,"model","confirm_phrase","method","placeholders":[],"plan":{},"install_config":{},"installer_cmdline","offline_bundle":null|"DIR","payload":{"sha256","bytes","files":[{path,mode,size,sha256}]},"preseed","stage2"}`

**`self-update --check --json`**: the updater's `UpdateInfo` (`current, latest, tag, update_available, channel, prerelease, asset{name,url,size}, checksums, signature, problems[], notes`) plus `program` and `installable`.

## Event stream (`--log-format json`)

For long jobs (`install`, `boot-manager`, `recover`, `backup-dom`, `disk-backup`, `fetch-installer`) `--log-format json` turns stdout into NDJSON, one object per line, each with `"event"` and an
RFC 3339 `"ts"`. Human text then goes to stderr (warnings and errors, phases and steps; everything with `-v`).

| `event` | Fields |
|---|---|
| `start` | `command`, `host`/`model`, `method`, `version` |
| `phase` | `phase` (`connecting`, `inspecting`, `staging`, `booting`, `installing`, `waiting_for_reboot`, `verifying`, `finished`, `failed`), `label`, `index`, `of` (7) |
| `step` | `n`, `total` (12), `name`: the installer's own steps (partitioning, RAID, LVM, filesystems, base system, packages, boot) |
| `progress` | `what`, `fraction` 0..1 (uploads, downloads) |
| `log` | `time`, `level` (`debug|info|warn|error`), `source` (`app|remote|installer|syslog`), `msg` (secrets redacted). Info and up; `-v` adds debug |
| `point_of_no_return` | the disks are being changed from now on |
| `installer_failed` | `message` (the installer on the NAS reported a failure) |
| `done` | the installer finished |
| `failure` | `kind` (`usage|refused|plan_errors|unsupported|failed_before_point_of_no_return|failed_after_point_of_no_return|error`), `message`, `exit_code`, `after_point_of_no_return` |
| `result` | `command`, `ok`, `data`: for `install` the logins `{hostname, address, username, password (only if generated), ssh_command, ssh_host_key, ssh_private_key, management_url, serial_console, log_path}` |

```sh
synofurb-cli --log-format json install --host 192.168.1.20 ... 2>/dev/null | tee run.ndjson | jq -c 'select(.event=="step" or .event=="failure" or .event=="result")'
```

## The wizard

`synofurb-cli wizard` (also `synofurb wizard`) walks the nine steps of the app in plain prompts. Every question shows its default in brackets; Enter accepts it. Ctrl-C quits at any prompt, and nothing on the
NAS is changed before step 7. It needs a terminal (stdin and stderr); without one it exits 2 and names the commands to use instead. `wizard --host H` skips the address question; `wizard --demo` /
`--facts FILE` rehearse with a sample or saved unit: no connection is made and the end is a dry run.

1. **Find the NAS**: scan now, type an address, or pick a remembered unit.
2. **Connect**: SSH user and port, password (or `$SYNOFURB_PASSWORD`) or a key file, sudo password.
3. **Inspect**: reads the unit, shows hardware, the compatibility status, platform support, disks, arrays and volumes. Unsupported hardware ends the wizard here (exit 5).
4. **Goal**: install Debian, boot manager, recover to DSM, or quit. The boot manager entry says when the unit cannot use it.
5. **Storage**: wipe or keep (and which arrays), system size with the allowed range, copies, RAID level, free percent. After each round the plan is shown with the *destroyed* and *preserved* lists; plans with errors loop back.
6. **System**: host name, account, password (generate or type), SSH key, time zone, kernel, management plane, boot method (unavailable ones are marked), serial console (detected from DSM), extra packages,
   firmware, installer verbosity, offline bundle, locale/keymap/mirror. Invalid values are caught here, not after the confirmation.
7. **Confirm**: unit, new system, plan, the destroyed list, the three-line "if something goes wrong" text, then the [safety gate](#the-safety-gate) with the **unit's address and serial** shown.
8. **Install**: the same runner as `install`: phases, steps, a progress bar on a terminal, Ctrl-C rules. You can ask to save the logins to a file.
9. **Done**: the logins.

A typical session on a dumb terminal (`TERM=dumb synofurb-cli wizard --no-color`) looks the same minus the progress bar.

## The safety gate

Before any disk is changed, **all three** must hold, every time:

1. a complete, verified backup exists (`--have-backup`, or answer `y`),
2. you understand that data can be destroyed, even data marked as preserved (`--accept-data-loss`, or answer `y`),
3. you typed the exact phrase `ERASE <MODEL>` (`--confirm "ERASE DS918+"`, or type it; the model is taken from the unit, case matters, surrounding whitespace is ignored).

On a terminal missing parts are asked (the phrase gets three tries, an empty line cancels). Without a terminal, missing parts are an error (exit 3) that lists them, and when flags are plainly absent it refuses
**before connecting** to anything. `--yes`, `--non-interactive`, `CI=true` and environment variables satisfy nothing. A plan with errors or unsupported hardware stops earlier (4, 5). `install` never accepts `--facts`
without `--dry-run`: a real install always talks to the live unit and re-checks its disks.

## Headless, CI and jump hosts

* **Servers and containers**: `install.sh --cli`; nothing else to install. Use environment variables or files for secrets, `--json`/`--log-format json` for machine output, check exit codes.
* **CI**: set `CI=true` (or run without a tty) so nothing prompts. The read-only commands and `install --dry-run --facts FILE` need no NAS; keep a saved `inspect --save` file to review plans offline.
  ```sh
  synofurb-cli inspect --host "$NAS" --key "$KEY" --save facts.json
  synofurb-cli plan --facts facts.json --mode keep --json > plan.json || [ $? -eq 4 ] && echo "plan has errors"
  synofurb-cli install --facts facts.json --mode keep --dry-run --out review/
  ```
* **Over SSH**: the CLI must run on a computer that can reach the NAS *and be reached by it* (the NAS's installer downloads the stage-2 script and payload from this computer over HTTP and sends its log to it on
  ephemeral TCP/UDP ports). A jump host on the NAS's LAN is ideal: `ssh -t jump synofurb-cli wizard` (`-t` gives the wizard its terminal). It does not tunnel; running it on a laptop elsewhere will not work.
  Wrap long installs in `tmux`/`screen` on the jump host, or `nohup synofurb-cli --log-format json install ... > run.ndjson 2> run.log &`, so a dropped connection cannot interrupt your view; the installer on the NAS
  keeps going after the point of no return regardless, and the new system's address is in the DHCP leases (or `synofurb-cli scan`, which also finds units this tool installed).
* **Firewalls** on the computer running the CLI must allow inbound connections from the NAS on the ports the log names (`-v`).

## Offline use

For networks without Internet access: build a **bundle** where there is Internet, move the directory (a USB stick is fine), install from it. Details of the format and
the dependency closure: [OFFLINE.md](OFFLINE.md).

```sh
synofurb-cli bundle create --arch amd64 --out /srv/synofurb-bundle                      # online machine; resumable (Ctrl-C, run it again); newest kernel, like the install default
synofurb-cli bundle info   /srv/synofurb-bundle [--json]                                # what is in it
synofurb-cli bundle verify /srv/synofurb-bundle                                         # re-hash everything, no network; exit 1 if damaged
synofurb-cli manage update --host 192.168.1.20 --from-bundle /srv/synofurb-bundle        # update the web UI on a unit from the bundle's mgmt/ folder
synofurb-cli install --host 192.168.1.20 --source offline --bundle /srv/synofurb-bundle ...   # offline machine
```

* **Sizes.** The bundle holds the installer files, the boot agent (a few MiB per architecture, for units with an internal boot flash) and only the Debian packages an install needs.
  amd64 is about **440 MiB** without installer firmware (405.7 MiB of it specific to amd64, 466 files); arm64 about 342 MiB and armhf about 234 MiB; arm64 + armhf in one bundle is about 608 MiB. The installer
  firmware (`firmware.cpio.gz`, **about 470 MiB**, shared by all architectures) is included unless you pass `--no-firmware`. Several architectures can go into one bundle (`--arch amd64,arm64`, or `all`).
* **`bundle create`** options: `--arch ARCH[,ARCH]|all` (default amd64), `--suite trixie`, `--kernel newest|stable` (default **newest**, the same as `install --kernel`: Linux 7.x from trixie-backports; `stable` is the 6.12 LTS kernel; the old `--kernel-newest` still works and means newest). A bundle holds **one** kernel and has no fallback to the other,
  `--no-firmware`, `--package PKG` (extra packages), `--no-agent`. It needs the Internet and refuses invalid architectures, suites and package names before starting (exit 2).
* **`install --bundle DIR`** (alias `--offline-bundle`; `--source offline` needs it, `--bundle` alone implies offline, `--source online --bundle` contradicts and exits 2) sets the job's `offline_bundle`:
  this computer serves the installer files, the boot agent and a Debian mirror made from the bundle, and neither side needs the Internet.
  Before the NAS is touched the CLI checks that DIR is a finished bundle (before even connecting) and that it **fits**: it contains the unit's architecture, the suite matches, the bundle's kernel channel
  equals `--kernel` (a default `bundle create` and a default `install` both mean newest; a bundle made with `--kernel stable` needs `install --kernel stable`). On a mismatch the message names both ways out: `install --kernel <the bundle's kernel>` or `bundle create --kernel <the install's kernel>`, and every `--package` was baked in with `--package` at creation.
  A mismatch is exit 2 with the reason; a bundle built without firmware only warns (use `--no-firmware` to silence it). The generated script and preseed then point at the bundle: `install --dry-run` shows the
  rewritten mirror host/directory (JSON field `offline_bundle`).
* **Needs an IPv4 address** of this computer on the NAS's network (the installer fetches packages from it); pick the LAN interface if the NAS is reached over IPv6.
* **`mgmt/` in the bundle.** `bundle create` adds the management app (`synofurb_mgmt.py`, `VERSION`, `README.md`, unit file, `MANIFEST.sha256`; [OFFLINE.md](OFFLINE.md#the-management-app-mgmt)).
  `bundle verify` checks it (extra output lines `MGMT <reason>` when damaged, `note: the bundle has no mgmt/ folder ...` for a bundle from an older build, which still verifies; exit 1 only for damage),
  `bundle info` prints `Mgmt app:  synofurb-mgmt <version>` and `bundle info --json` has `mgmt_version` (`null` when absent), and `install --bundle` exits 2 when the bundle's `mgmt/` is damaged.
  The installer itself does not read `mgmt/`; it installs the copy built into the program. `manage update --from-bundle` is the consumer.
* The wizard asks "Use an offline bundle?" in step 6 and runs the same checks, asking again when the bundle does not fit.
* `bundle create|verify|info` print their own progress and results (not the event stream); errors exit 1.

## Updating

`synofurb-cli self-update --check` asks GitHub for the newest release on the channel (`--channel stable|prerelease`), `self-update` downloads **`synofurb-cli-<version>-<os>-<arch>.tar.gz`** (the desktop program updates
from its own asset), verifies the minisign signature of `SHA256SUMS` and then the SHA-256, keeps the old binary as `synofurb-cli.old` until the new one has started once, and replaces itself. It refuses to install anything
while the project's signing key is still the placeholder. On Windows it stops after verification and tells you where the archive is (a running `.exe` cannot replace itself). A package-managed copy must be updated by its package manager.

## GUI parity

| App screen / button | Command |
|---|---|
| Find NAS (scan, manual address, inventory) | `scan`, `inventory list|add|forget`, `wizard` step 1 |
| Connect | `--host --user --port --key --ssh-password-*`, prompts |
| Inspect, compatibility status, platform support | `inspect`, `compat` |
| Goal: install / boot manager / recover | `install`, `boot-manager`, `recover`, `wizard` step 4 |
| Storage (mode, keep list, size, copies, RAID, free %) with the destroyed/preserved lists | `plan`, `install` plan knobs |
| System (host, user, password, SSH key, time zone, kernel, mirror, packages, serial console, DTB, management plane, boot method, firmware, verbose installer, kit folder, kexec) | `install` flags, `wizard` step 6 |
| Confirm (two boxes + typed phrase) | the [safety gate](#the-safety-gate) |
| Install (phases, 12 steps, log, cancel) | `install` progress, `--log-format json`, Ctrl-C |
| Done (logins, save to file) | the printed logins, `--save-logins` |
| Log viewer | `logs list|show|path` |
| Settings: compatibility list refresh, updates | `compat --refresh`, `self-update`, `config show` |
| Boot manager / Recovery screens | `boot-manager install|enter|status`, `recover` |
| Manage (health of the converted unit, web UI version and update) | `manage health|version|update|rollback|open` |
| (CLI only) | `--dry-run`, `--facts`, `bundle`, `backup-dom`, `disk-backup`, `completions`, JSON and NDJSON |

## Troubleshooting

* `no SSH password: set $SYNOFURB_PASSWORD ...`: there is no terminal to ask on; use the variable, `--ssh-password-file` or `--key`.
* `SSH authentication failed`: wrong user/password; the DSM user must be an administrator with SSH enabled (Control Panel -> Terminal). Encrypted private keys are not supported.
* `refusing to continue: still missing ...` (exit 3): add the flags it lists, with the phrase printed by `plan`.
* `the NAS did not contact this computer within 10 minutes`: a firewall on this computer, or the NAS cannot reach it; see the serial/VGA console and `-v`.
* `manage health` says "too old to report health" or "not installed": the unit's management plane predates `health` (or was installed with `--no-mgmt`); `manage update` installs the current one.
* `manage update` ends with "Update failed ... back on version X": the unit rolled back by itself; the failing step is in the message, the whole story in `logs show`.
* Exit 8/9 from `manage health` is the unit's verdict, not a failure of the command; use `--json` and read `summary.bullets`.
* Exit 5 on a model you believe works: `compat MODEL` shows why; the list can be refreshed with `compat --refresh`.
* `logs show` has the whole story of any run, including secrets-redacted remote commands.

## Testing

Automated (no NAS needed): `cargo test -p synofurb --no-default-features` (argument parsing for every command, exit codes, JSON shapes on the repository fixtures, the gate, dry runs, the wizard with scripted answers and a
fake backend, the runner) and `tests/cli/smoke.sh [path/to/synofurb-cli]`, which runs the binary under `env -i` with no display, drives the wizard through a real pty (`TERM=dumb`), and, when `sshd`, `ssh-keygen` and root are available,
exercises `inspect`, `plan`, `install --dry-run` and the NDJSON stream against a real local `sshd`, and `manage health|version|update|rollback` against a second `sshd` whose commands are redirected to the repository's
`synofurb_mgmt.py` in a sandbox root (details in [MANAGE.md](MANAGE.md#what-is-verified)). The engine's own tests (`cargo test -p synofurb-core manage`) also run that script, when `python3` is present. The installer scripts' `--cli` paths are covered by `tests/install/run-tests.sh` against a fake release server.

**Manual checks that need a real unit** (nothing has been run on a real Synology yet; record results with the compatibility report template):

1. `scan` finds the unit; `inventory list` shows it; `inventory forget` removes it.
2. `inspect --host` on DSM 7: model, serial, disks, arrays, firmware, serial console match what DSM shows; `inspect --save` then `plan --facts` gives the same plan as `plan --host`.
3. `plan` for wipe and keep, with `--keep`, `--raid`, `--copies`: the destroyed and preserved lists match the disks.
4. `install --dry-run --out` and read `stage2.sh`; `sh -n stage2.sh` passes.
5. `install` without any gate flag exits 3 without connecting; with a wrong `--confirm` exits 3 after the plan.
6. Real install on a spare unit with `--method kexec`/`dom`/`kit` as the hardware allows: Ctrl-C before the point of no return exits 7 with the disks untouched; a full run ends with the logins and `ssh` works.
7. Kill the SSH session of a running install after the point of no return (inside `tmux`): the unit still finishes; `scan` finds it.
8. `boot-manager install` / `status` / `enter` / `recover` on an x86 unit with a boot flash; `backup-dom` hash matches `sha256sum /dev/synoboot` on the NAS.
9. `disk-backup`: the head/tail files restore a scratch disk's partition table.
10. `--source offline --bundle` on a network with no route to the Internet.
11. `self-update --check` and `self-update` against a published release (once the signing key is set up).
12. `manage health` on a converted unit; degrade an array (`mdadm --fail`) and see exit 9 and the bullet; `manage update` from an older management plane with and without `--force`; stop the service by hand during an update
    and see the rollback reported; `manage rollback` afterwards.
