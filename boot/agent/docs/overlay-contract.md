# Contract between the agent and the desktop app

This document is the interface: what the app must put into the overlay, what the agent expects on the kernel command line, and
what the app can rely on when it drives the agent over SSH.

## 1. Kernel command line (set by the DOM component, not by the agent)

The agent needs **nothing** on the command line. It reads `/proc/cmdline` only for:

* `console=` — to log its address summary and to start root shells. Recommended: `console=tty0 console=ttyS0,115200n8` (or whatever DSM used; copy DSM's own `console=`).
  Any console the kernel registers works, including `console=uart8250,mmio32,0xfedc9000,115200n8`: important lines go through `/dev/kmsg` at `KERN_CRIT` (shown on all consoles at the default
  `loglevel`; `quiet` is fine, `loglevel=0` would hide them) and a shell is started on every `tty*`/`ttyS*`/`ttyAMA*`/`ttyUSB*`... listed in `/sys/class/tty/console/active`.
* `sf-boot kexec --reuse-console` copies the `console=` words to the next kernel's command line.

Do **not** pass `init=`, `root=` or `rdinit=`; `/init` is the right entry point and there is no root device.

## 2. The overlay

A gzip-compressed **newc** cpio archive (`cpio -H newc`, owner 0:0), appended to the base initrd:

```bash
cat agent-amd64.initrd overlay.cpio.gz > initrd        # byte concatenation is the whole operation
```

`test/make-overlay.sh` is the reference implementation (any language can write a newc archive; the format is 110-byte ASCII headers). Files:

| path in the agent | purpose |
|---|---|
| `/etc/synofurb-agent.conf` | settings, **shell syntax** (sourced by busybox ash). Mode 0600 recommended. LF line endings. |
| `/etc/synofurb-authorized_keys` | one OpenSSH public key per line (`#` comments and blank lines ignored). Alternative or addition to `AGENT_SSH_AUTHORIZED_KEYS` |
| `/etc/dropbear/dropbear_ed25519_host_key` | optional, pins the SSH host key (dropbear format, `dropbearkey -t ed25519`); otherwise a key is generated at every boot and its fingerprint printed on the console and in `agent.log` |
| anything else | any file can be shipped, e.g. `/usr/bin/synofurb-dom` to override the embedded one, `/lib/firmware/...` for NIC firmware |

A syntax error in the conf file is logged (`sf-boot log`) and **all defaults are used**: no keys, so the unit is reachable only on the serial console, and the watchdog reboots it.
Validate with `sh -n` before shipping.

### `/etc/synofurb-agent.conf` settings

Quote values with single quotes (`'` as `'\''`). Unset = default.

| variable | default | meaning |
|---|---|---|
| `AGENT_HOSTNAME` | `synofurb-<mac6>` | `[A-Za-z0-9-]`, otherwise ignored; sent as DHCP hostname and in the beacon |
| `AGENT_TIMEOUT_MIN` | `20` | minutes until the watchdog reboots the unit to DSM; `0` disables the watchdog (dangerous: logged loudly) |
| `AGENT_WATCHDOG_MODE` | `login` | `login`: the first successful SSH login (key or password) disarms; `claim`: only `sf-boot claim` (or `autorun`/`kexec`) does |
| `AGENT_SSH_AUTHORIZED_KEYS` | empty | multi-line allowed; combined with `/etc/synofurb-authorized_keys`; root login only |
| `AGENT_ONE_TIME_PASSWORD_HASH` | empty | `crypt(3)` hash (`openssl passwd -6` was tested; other schemes libcrypt1 knows should work) for root password login. Without it dropbear runs with `-s` (no password auth). **Revoked after its first successful use** unless `AGENT_PASSWORD_REUSABLE=1` |
| `AGENT_AUTORUN_URL` | empty | script to download (`http://`, `https://`) once the network is up (waits `AGENT_AUTORUN_WAIT_S`, default 180) and run with `sh` as root |
| `AGENT_AUTORUN_SHA256` | empty | **required** for autorun unless `AGENT_AUTORUN_ALLOW_UNVERIFIED=1`: busybox wget does not verify TLS certificates. A mismatch refuses to run the script |
| `AGENT_NET_MODE` | `dhcp` | `dhcp` or `static` |
| `AGENT_NET_IFACE` | `auto` | static mode only: `auto` (first port with carrier, else first port), `ethN`, or a MAC address. Other ports keep using DHCP |
| `AGENT_STATIC_ADDRESS` / `_GATEWAY` / `_DNS` | empty | `192.168.1.50/24`, `192.168.1.1`, `'192.168.1.1 1.1.1.1'` |
| `AGENT_STATIC_ADDRESS6` / `_GATEWAY6` | empty | `2001:db8::50/64`, `2001:db8::1` |
| `AGENT_SSH_PORT` | `22` | |
| `AGENT_BEACON_PORT` | `9988` | UDP port of the discovery beacon |
| `AGENT_CONSOLE_SHELL` | `1` | `0`: no shells on the consoles |
| `AGENT_MODULES` | empty | extra module names to `modprobe` at boot (must be in the image) |

### Autorun contract

* Downloaded to `/run/synofurb/autorun.sh`, SHA-256 checked, then **the unit is claimed** (watchdog off, reason `autorun`) and the script runs with `sh`, output in `/var/log/autorun.log`.
* Exit status 0: the unit stays claimed (the script is expected to have chained into something, e.g. `sf-boot kexec ... && sf-boot exec`).
  Non-zero: the watchdog is **re-armed** with the full timeout, so a broken chain falls back to DSM.
* Any download failure or hash mismatch: the script is not run and the watchdog stays armed.
* The environment is minimal; source `/etc/synofurb-agent.conf` yourself if you need the settings. `/usr/lib/synofurb/lib.sh` offers `note` (log to all consoles), `claim_unit`.

## 3. Driving the agent over SSH

Login as `root`. Every command below exits 0 on success, 1 on error (message on stderr), 2 on usage errors.

* **Is it the agent, and is it healthy?** `sf-boot status` prints `key=value` lines (`--json` for a flat JSON object with string values):

  | key | values |
  |---|---|
  | `agent`, `agent_version`, `agent_arch`, `agent_kernel`, `agent_suite`, `agent_built`, `agent_dom_stub` | build facts (`agent_dom_stub=1` means the DOM fail-safe is a stub) |
  | `hostname`, `uptime_s`, `kernel` | |
  | `state` | `starting`, `armed`, `claimed`, `disabled` |
  | `claim_reason` | `ssh-login`, `claim`, `autorun`, `kexec`, or what was given to `sf-boot claim` (only when claimed) |
  | `watchdog_timeout_min`, `watchdog_remaining_s` | remaining is present only while armed |
  | `dom_default` | `pending` (still trying), `ok` (DOM default is DSM), `failed` (gave up after 40 tries), `missing` (stub) |
  | `kexec_loaded` | `0`, `1`, or `unsupported` |
  | `net.<if>.mac` / `.carrier` / `.ipv4` / `.ipv6` | comma-separated CIDR lists |

* **Take ownership**: `sf-boot claim agent-app` (idempotent). With `AGENT_WATCHDOG_MODE=login` the SSH login itself already did it (within 3 s).
* **Upload files**: dropbear has no sftp/scp server; stream over stdin: `ssh root@ip 'cat > /tmp/vmlinuz' < vmlinuz`, then `ssh root@ip sha256sum /tmp/vmlinuz`. `/tmp` is RAM.
* **Chain into an installer**:

  ```
  sf-boot kexec /tmp/vmlinuz /tmp/initrd.gz --cmdline 'auto=true priority=critical ...' --reuse-console   # local files: no hash needed
  sf-boot kexec http://host/vmlinuz http://host/initrd.gz --sha256 kernel=<hex>,initrd=<hex>             # URLs: hashes mandatory
  sf-boot exec --detach --delay 3     # returns immediately; the SSH connection drops when the new kernel starts
  ```

  `kexec` claims the unit, frees the downloaded copies, and tries `kexec -l` then `kexec -s -l` (kexec_file_load). On x86 the next kernel starts with the command line you gave;
  it does not inherit the agent's. On ARM pass `--dtb`.
* **Back to DSM / on to Debian**: `sf-boot reboot dsm|debian|agent` runs `synofurb-dom --yes default <x>` and reboots only if that succeeded.
* **Discovery without SSH**: UDP broadcast `synofurb-agent <hostname> <mac> <ip>` every 5 s on port 9988 (IPv4 only); the MAC is that port's MAC, one datagram per port that has an IPv4 address.

## 4. Timing the app can expect

QEMU/TCG (software emulation, slower than any real NAS): kernel starts `/init` after ~8 s (kernel + 8 MiB initramfs unpack), DHCP lease and SSH at ~12-20 s. On real hardware add the BIOS/UEFI + bootloader time (outside this repository, unmeasured) and keep the connection retry loop at least a few minutes long. Default watchdog: 20 minutes.
