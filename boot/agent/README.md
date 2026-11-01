# synofurb boot agent

A very small, remote-controllable Linux boot environment (kernel + initramfs) that lives in the internal boot
flash module ("DOM") of a Synology NAS. It boots in seconds, brings up the network on every Ethernet port,
offers SSH, and can `kexec` into any other kernel: the full Debian installer, or any OS installer you want to try.

The only access the user is guaranteed to have is SSH to the running DSM, so the agent must be reachable over the
network by itself and must **fail safe**: its first action is to point the DOM boot default back at DSM, and if
nobody logs in (or claims it) within N minutes it reboots, which boots DSM again.

```
 desktop app ──ssh──> DSM ──synofurb-dom install-agent──> DOM (grub entry: agent kernel + initrd + overlay)
                       │ one-shot reboot into the agent
                       ▼
                    agent (this directory):  default := DSM  →  DHCP/SLAAC  →  SSH + beacon  →  watchdog
                       │ sf-boot kexec <installer kernel> <initrd> ; sf-boot exec
                       ▼
                    Debian installer (or anything else)
```

| | |
|---|---|
| Kernel | Debian's stock kernel image, unchanged (`linux-image-amd64` / `-arm64` / `-armmp`) |
| Userland | busybox-static (PID 1 = busybox init), dropbear SSH, kexec-tools, e2fsck + fsck.fat (for `synofurb-dom`), netcat-traditional (UDP beacon) |
| Drivers | only the NIC and DOM-storage modules in the initramfs, found via `modalias` scan |
| Personalisation | a ~500-byte gzip cpio **overlay** appended to the initrd (`test/make-overlay.sh`) |
| Budget | kernel + initrd **≤ 24 MiB** (amd64); the build fails when it is exceeded |

## Build

```bash
# host: Linux x86-64 with network access; Debian/Ubuntu packages:
sudo apt-get install curl dpkg-dev xz-utils cpio kmod binutils gawk gnupg gpgv debian-archive-keyring zstd

./build.sh amd64                       # default suite: trixie-backports (Linux 7.2.x)
./build.sh amd64 --suite trixie        # Linux 6.12 LTS
./build.sh arm64
./build.sh armhf                       # → dist/agent-<arch>.vmlinuz, agent-<arch>.initrd, SHA256SUMS, MANIFEST-<arch>.json, initrd-<arch>.lst
```

`build.sh` downloads over plain HTTPS from `deb.debian.org` (and `security.debian.org` for fixes) and verifies the chain
`InRelease` (PGP, via `gpgv` + `debian-archive-keyring` when present; `--strict-pgp` makes that mandatory) →
`Packages.xz` (SHA-256 from InRelease) → each `.deb` (SHA-256 from Packages). Packages are unpacked with
`dpkg-deb -x`; **nothing of the target architecture is executed**, so all three architectures build on x86-64.
Downloads are cached in `~/.cache/synofurb-agent` (`--cache DIR`). The first amd64 build downloads ~190 MB
(the 150 MB `linux-modules` package is where the NIC drivers come from); a rebuild takes about a minute.

Other options: `--out DIR`, `--budget-mib N`, `--dom-script FILE`, `--modules "a b"`, `--compress xz|zstd`, `--keyring FILE`,
`--keep-work`, `--version V` (see `./build.sh --help`).

`synofurb-dom` is taken from `../dom/synofurb-dom` and must run under busybox ash. If it is missing, a **stub** is embedded
(exit status 64) and the build prints a loud warning; `MANIFEST-<arch>.json` then says `"dom_stub": true`. Never ship a stub build.

## Personalise a unit

```bash
test/make-overlay.sh --out overlay.cpio.gz --hostname nas-livingroom --pubkey ~/.ssh/id_ed25519.pub --timeout-min 20
cat dist/agent-amd64.initrd overlay.cpio.gz > initrd-for-this-nas        # the kernel unpacks concatenated cpio archives
```

Full format: [docs/overlay-contract.md](docs/overlay-contract.md).

## Sizes (what the build actually produced)

Measured on 2026-11-01 with Debian trixie / trixie-backports as of that day.

| Arch | Kernel (suite) | `agent-<arch>.vmlinuz` | `agent-<arch>.initrd` | Total | Budget |
|---|---|---:|---:|---:|---|
| amd64 | 7.2.11+deb13 (trixie-backports) | 14,329,792 B (13.67 MiB) | 8,505,868 B (8.11 MiB) | **22,835,660 B (21.78 MiB)** | 24 MiB, 2.22 MiB free |
| amd64 | 6.12.113+deb13 (`--suite trixie`) | 12,154,816 B (11.59 MiB) | 7,628,832 B (7.28 MiB) | **19,783,648 B (18.87 MiB)** | 24 MiB, 5.13 MiB free |
| armhf | 7.2.11+deb13-armmp (trixie-backports) | 6,849,024 B (6.53 MiB) | 5,666,696 B (5.40 MiB) | **12,515,720 B (11.94 MiB)** | 24 MiB, 12.06 MiB free |
| arm64 | 7.2.11+deb13-arm64 (trixie-backports) | 48,832,448 B (46.57 MiB) | 6,556,572 B (6.25 MiB) | **55,389,020 B (52.82 MiB)** | **does not fit 24 MiB**; default budget 64 MiB |

Why arm64 is so big: Debian ships the arm64 kernel as an *uncompressed* `Image` (gzip of it is 16.2 MiB). arm64 Synology units
have no DOM (they boot through U-Boot), and the task is "kernel image unchanged", so the build keeps it and uses a 64 MiB budget
for arm64. Compressing it for U-Boot is the consumer's decision.

amd64 7.2.11 initrd breakdown (each part compressed alone with `xz -6`, so the parts add up to a bit more than the real 8.11 MiB):
kernel modules 4.83 MiB (67 modules, 33.4 MiB raw; `mlx5_core` at 6.5 MiB raw, then `bnx2x`, `ixgbe`, `i40e`, `mlx4_core` are the big ones),
busybox 0.86 MiB, shared libraries 1.85 MiB (glibc 1.0, the Xen libraries kexec-tools links against, libext2fs, libgmp…),
dropbear/kexec/e2fsck/fsck.fat/nc 0.32 MiB, scripts 10 KiB.
Boot time in QEMU/TCG (no KVM, so slow): kernel up at 8 s, SSH reachable at ~20 s.

## `sf-boot`

```
sf-boot status [--json]      state, watchdog, DOM default, addresses (key=value lines)
sf-boot claim [REASON]       cancel the safety watchdog
sf-boot arm [MINUTES]        re-arm it
sf-boot kexec KERNEL INITRD [--cmdline STR] [--reuse-console] [--dtb FILE] [--sha256 kernel=HEX,initrd=HEX] [--insecure]
sf-boot kexec --unload
sf-boot exec [--delay SEC] [--detach]    sync, then kexec -e
sf-boot reboot dsm|debian|agent          synofurb-dom default <x>, then reboot
sf-boot dom ARGS...                      pass-through to synofurb-dom
sf-boot net | sf-boot log [-f] [agent|dropbear|autorun|dmesg] | sf-boot version
```

`KERNEL`/`INITRD` are local files (e.g. uploaded over SSH) or `http(s)://` URLs. **busybox `wget` encrypts HTTPS but never
verifies the server certificate** (verified here: a self-signed certificate is accepted), so an HTTP(S) download without
`--sha256` is refused unless `--insecure` is given. That is also why no `ca-certificates` bundle is shipped: nothing could use it.

## Discovery beacon

Every 5 s, per IPv4-configured port, a UDP broadcast to `255.255.255.255:9988` (`AGENT_BEACON_PORT`):

```
synofurb-agent <hostname> <mac> <ip>\n
```

(busybox `nc` cannot do UDP, so the 139 KiB `netcat-traditional` provides `nc.traditional -b -u`.) IPv6-only networks get no beacon.

## Tests

```bash
test/check-structure.sh <amd64|arm64|armhf>   # no execution of target code: file types, ELF machine, loader, NEEDED libs, modules.dep, initrd layout
test/smoke.sh                                  # QEMU/TCG boots of the amd64 agent (needs qemu-system-x86, openssh-client, python3, openssl)
```

See [docs/hand-testing.md](docs/hand-testing.md) for doing it by hand, and [docs/safety-model.md](docs/safety-model.md) for the fail-safe design.

## What is verified (honest list)

Verified in this repository's development sandbox (x86-64 host, **no KVM**, QEMU TCG, no Synology hardware):

* amd64 build end to end for both suites (`trixie-backports` 7.2.11, `trixie` 6.12.113), with PGP verification of all `InRelease` files.
* `test/smoke.sh` on the 7.2.11 build, five guest configurations (A–E), all checks passing: boots in QEMU with `e1000` and `virtio-net`; DHCPv4 lease, default route,
  DNS; SLAAC address from a router advertisement; SSH key login through the appended overlay; one-time password login (works once, then revoked);
  `sf-boot status/--json/claim/arm/reboot/kexec/exec`; `kexec --version`; the UDP beacon (seen on the guest, see limits below); the watchdog
  rebooting an unclaimed guest after 1 minute (QEMU `-no-reboot` turns the reboot into an exit); `claim` mode, autorun with pinned SHA-256
  (good, wrong hash, failing script); the fail-safe call to `synofurb-dom` happening before the network is up (with a recording stand-in);
  a real `kexec` of a second kernel downloaded from a local HTTP server with SHA-256 enforcement; HTTPS download with a self-signed certificate;
  static addressing; console handling for `console=ttyS0` and for `console=tty0 console=uart8250,io,0x3f8,115200n8` (VGA plus an 8250 port named by I/O address).
* The 6.12 build boots and passes guests A and B (39 checks).
* arm64 and armhf: **built only, never booted.** `test/check-structure.sh` passes for both (ELF machine type of every binary and module, program
  interpreter at its exact path, all `DT_NEEDED` libraries present, `modules.dep` consistent, initrd layout, SHA256SUMS).

Not verified (do not assume):

* Anything on real Synology hardware: the real `synofurb-dom` against a real DOM (QEMU has none; in QEMU it fails gracefully with "no Synology DOM found"),
  the DOM boot chain that loads kernel+initrd, real NICs (only `e1000` and `virtio_net` were exercised; all other NIC modules are merely present and
  dependency-complete), USB/SATA DOM driver behaviour, `kexec` on real NIC/storage hardware, reboot behaviour of the real chipset.
* `console=uart8250,mmio32,0xfedc9000,...` consoles (QEMU's PC has no device there; the `uart8250,io,0x3f8` form was verified and takes the same path: the kernel names the port
  `ttyS<N>` and lists it in `/sys/class/tty/console/active`), and what a *real* VGA console looks like (only QEMU's was present; the shell on tty1 was started).
* The UDP beacon on a real LAN: QEMU user networking does not forward guest broadcasts, so reception was verified on the guest itself (a broadcast is looped back to
  local sockets). The sender sets `SO_BROADCAST` and binds the interface address; that is all that was checked.
* DHCPv6 (`udhcpc6` ran but QEMU's slirp offers no DHCPv6 lease, so only the failure path was seen); SLAAC worked. RDNSS (DNS via router advertisements) is not implemented.
* arm64/armhf at runtime and building on a non-Debian-family host. (`--compress zstd` was built and booted through guest A: 22.46 MiB total vs 21.78 MiB for xz; xz stays the default.)
* NIC firmware: none is shipped (see below).

## Known limitations and risks

* **No NIC firmware.** Debian's firmware lives in `firmware-*` packages (non-free-firmware) that are large, and the DOM budget has no room for them (2.2 MiB free). Drivers such as `bnx2x` and
  `atlantic` need host-loaded firmware to work at all, and some `r8169` chip revisions run in a degraded mode without it; the modules load, then log `Direct firmware load failed`.
  From the driver sources (not tested here), `e1000e`, `igb`, `ixgbe`, `i40e`, `tg3`, `sky2` and `virtio_net` normally need none. If you need one, pass `--modules` to add drivers, and put firmware into the overlay via `make-overlay.sh --file /lib/firmware/NAME=PATH`
  (the overlay is not size-limited by the build, the DOM is).
* **The 24 MiB budget has 2.2 MiB headroom** with Linux 7.2 (6 MiB less with the 6.12 LTS kernel). A new kernel version can push it over; the build then fails and says so.
  The biggest knob is `mlx5_core` (6.5 MiB raw): drop it from `NIC_MODULES` in `build.sh` if no Mellanox NIC is relevant.
* The agent keeps everything in RAM: ~42 MiB for the root plus the kernel; `sf-boot kexec` downloads the next kernel+initrd into RAM (tmpfs) first and `kexec` makes a second copy (the tmpfs copy is deleted once loaded), so the peak is about
  twice the payload on top of the agent's own ~60 MiB. 512 MiB is plenty for the Debian installer netboot images.
* `sf-boot exec` ends the watchdog's protection: after `kexec` the next kernel is on its own. That is why the DOM default is set to DSM *first*; a power cycle returns to DSM.
* If `synofurb-dom default dsm` cannot succeed (stub build, no DOM found, DOM damaged) the agent says so loudly on every console and in `sf-boot status` (`dom_default=failed|missing`),
  but it cannot do better.
* busybox `wget` has no certificate verification (see above); dropbear host keys are regenerated on every boot unless the overlay pins one (`--host-key`).
* The serial console gets a root shell without login (physical access = root, like the kernel console itself). `AGENT_CONSOLE_SHELL=0` turns it off.

## Layout

```
build.sh                  the build (bash)
rootfs/                   files copied into the initramfs: /init, /usr/lib/synofurb/*, /usr/bin/sf-boot, /etc/*
test/make-overlay.sh      builds the per-unit overlay
test/smoke.sh             QEMU smoke test (amd64)
test/check-structure.sh   structural checks for any architecture
docs/                     architecture, overlay contract, safety model, hand testing
ci-snippet.yml            GitHub Actions job (build 3 archs, QEMU smoke test, upload artifacts)
dist/ work/ cache/        build products (git-ignored)
```
