# Architecture

## Pieces

```
agent-<arch>.vmlinuz   Debian's kernel image, byte-identical to the one in linux-image-* / linux-binary-*
agent-<arch>.initrd    [archive 1: uncompressed newc cpio, 3 entries: dev/, dev/console, dev/null]
                       [archive 2: xz (crc32, 16 MiB dictionary; x86 BCJ filter on amd64) of one newc cpio = the root file system]
overlay (optional)     [archive 3: gzip newc cpio appended by the desktop app: /etc/synofurb-agent.conf, keys, ...]
```

The Linux initramfs loader accepts any number of concatenated cpio archives, each plain or compressed with a different
algorithm, and extracts them in order into the same tmpfs (later files replace earlier ones). That is the whole overlay mechanism;
no repacking of the 8 MiB xz stream is ever needed. Archive 1 exists because a cpio containing device nodes cannot be
produced without root, so `build.sh` writes those three 110-byte headers by hand (`newc_dev_archive`) and the rest is made unprivileged.

## Boot flow

```
bootloader (DOM grub entry)  kernel + initrd(+overlay)  cmdline: console=... (from the DOM component)
  │
kernel unpacks the initramfs, runs /init  (PID 1, busybox ash script)
  │ /init:  mount proc sys devtmpfs devpts tmp run; stdio -> /dev/console; mkinittab; exec busybox init
  ▼
busybox init (PID 1 for good: reaps zombies, handles reboot/poweroff) reads the generated /etc/inittab:
  ::sysinit:  boot.sh              (init waits for it)
  ::respawn:  svc-net  svc-dropbear  svc-beacon  svc-watchdog
  ::once:     autorun
  tty1/ttyS0:: respawn console-shell     (one per console the kernel reports in /sys/class/tty/console/active)
```

`boot.sh`, in this order:

1. read `/etc/synofurb-agent.conf` (defaults first, then the overlay's file; a syntax error is logged and defaults are used);
2. **start the fail-safe**: a background loop runs `synofurb-dom --yes --skip-auto-backup default dsm` every 3 s until it succeeds
   (up to 2 minutes), because the DOM block device only exists after the storage drivers have loaded. Exit status 64 (the build-time stub)
   stops the loop at once. Result in `/run/synofurb/dom_default`: `pending`, `ok`, `failed`, `missing`;
3. load drivers: `modprobe` for `af_packet ipv6`, then two passes of `find /sys/devices -name modalias | sort -u | xargs modprobe -qba`
   (busybox modprobe resolves modaliases through `modules.alias`; the second pass catches PHY/MDIO devices that only appear after their NIC driver bound);
4. hostname (`AGENT_HOSTNAME` or `synofurb-<last 6 hex digits of the first MAC>`), `/etc/hosts`;
5. SSH credentials: `/root/.ssh/authorized_keys` from `AGENT_SSH_AUTHORIZED_KEYS` + `/etc/synofurb-authorized_keys`; a root `shadow` entry only when a password hash was supplied;
   ed25519 + ecdsa host keys unless the overlay brought some; the host-key fingerprint is printed on every console;
6. `accept_ra=2`; returns to init.

Then, in parallel, the respawned services:

| service | job |
|---|---|
| `svc-net` | for every `eth*`/`en*` port (ARPHRD_ETHER, not wireless): link up, `accept_ra=2` (kernel SLAAC), `udhcpc` (v4) and `udhcpc6` (v6) in the background, or static addressing for one port. Rescans for late hardware (USB NICs) every 5 s for the first minute. The event script `udhcpc.script` sets address, default route (metric 100+ifindex), a 224.0.0.0/4 route, and rebuilds `/etc/resolv.conf` from all ports |
| `svc-dropbear` | dropbear on port 22 (`-s` = no password unless a hash was supplied), log in `/var/log/dropbear.log` |
| `svc-beacon` | the UDP beacon, every 5 s |
| `svc-watchdog` | reboot after `AGENT_TIMEOUT_MIN` unless claimed; detects SSH logins by `auth succeeded` in the dropbear log; revokes the one-time password |
| `autorun` | optional script download + verification + run |
| `console-shell` | root shell on VGA/serial consoles (no getty, no login) |
| `announce` | called by `udhcpc.script`/static setup whenever the address set changes: prints `READY <host>: eth0[mac]=addr -- ssh root@ip` |

Important lines are written with `note()`: to `/var/log/agent.log` **and** to `/dev/kmsg` at `KERN_CRIT`, which the kernel prints on *every* console it registered
(VGA, `ttyS*`, `uart8250,mmio32,...` ports, netconsole). That is how a unit with only a serial console, or only a VGA console, shows its addresses without the agent
knowing which consoles exist. Shell access on the consoles is derived from `/sys/class/tty/console/active`; a console given as `uart8250,mmio32,0xfedc9000,115200n8` is
listed there as an ordinary `ttyS<N>`.

## Why these choices

* **Debian kernel image unchanged, modules selected from `linux-modules-*`**: the DOM can hold only ~24 MiB; the image alone is 13.7 MiB. Only NIC + DOM-storage modules ship
  (67 files, 33 MiB raw → 4.8 MiB xz). `build.sh` decompresses the `.ko.xz` files (busybox `modprobe`/`insmod` cannot read them), regenerates `modules.dep`/`modules.alias`
  with `depmod -b` over just that subset, and drops everything else. The closure of a requested module name (`-`/`_` equivalent, globs allowed) is computed from the
  distribution's `modules.dep`; names that are not modules in that kernel (built in, e.g. `ipv6`, `af_packet`) are listed in the manifest and skipped.
* **busybox init as PID 1** rather than a shell script: it reaps the orphans udhcpc/dropbear leave behind, and `reboot`/`poweroff` (busybox applets that signal init) just work.
  `/init` is still an ash script, as the design asked; it only gets the machine to the point where busybox init can take over.
* **Dynamic dropbear/kexec/e2fsck are copied with exactly the libraries they need**: `readelf -d` NEEDED entries are followed recursively, so of glibc only `libc.so.6` and the loader
  ship (1.0 MiB xz) instead of the whole `libc6` package, and of `libxenmisc4.20` only the one `libxenctrl.so` that kexec-tools links against.
  kexec-tools in Debian is built with Xen support, which is why 7 small Xen libraries are in the image; there is no static kexec in the Debian archive.
* **`netcat-traditional` just for the beacon**: busybox `nc` has no UDP; every other Debian tool that can send a UDP broadcast with a chosen payload is larger.
* **No firmware, no ca-certificates**: see README.
* **`/tmp` and `/run` are tmpfs, the root is the kernel's rootfs (tmpfs)**, so nothing the agent does touches a disk except what `synofurb-dom` writes on purpose.

## Build pipeline (`build.sh`)

1. For `trixie`, `trixie-updates`, `trixie-security` (and `trixie-backports` for the default kernel suite): fetch `InRelease`, check `Valid-Until`, verify PGP with `gpgv`
   (any `GOODSIG` from a key in `debian-archive-keyring` or `--keyring` passes, `BADSIG` is fatal, no verifiable signature is a warning unless `--strict-pgp`),
   take the SHA-256 of `main/binary-<arch>/Packages.xz`, fetch it (by-hash URL first), verify, convert to a TSV index.
2. Kernel: `linux-image-<flavor>` (flavor: `amd64`, `arm64`, `armmp`) → its `linux-image-<version>` dependency → that package plus its `linux-binary-*`/`linux-modules-*` dependencies.
   The highest version across the suite's indexes wins (`dpkg --compare-versions`).
3. Userland: busybox-static, dropbear-bin, kexec-tools, e2fsprogs, dosfstools, netcat-traditional plus their `Depends`/`Pre-Depends` closure (first alternative of each; a skip list
   removes debconf, dpkg, util-linux... which would only add weight). Every `.deb` is checked against the SHA-256 in `Packages` and cached by that hash.
4. Assemble the root: our `rootfs/` tree, busybox + applet symlinks (the applet list is read from busybox's own table, so no foreign binary is executed; the build fails if an
   essential applet is missing), the extra binaries, their libraries and the loader symlink, `synofurb-dom` (or the stub), the release file, the modules.
5. cpio (`-R 0:0`, mtimes normalised to the suite's `Date:`; not bit-for-bit reproducible, cpio records inode numbers), xz, concatenate with the device-node archive, size report, budget check, outputs.
