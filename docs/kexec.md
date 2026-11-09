# kexec: starting the Debian installer from DSM

synofurb starts the installer without any USB stick by loading it into RAM from the running DSM with `kexec`
(`kexec -l ...; sync; remount read-only; kexec -e`). This needs two things.

## 1. A kernel with kexec

`synofurb plan` / the GUI check `/sys/kernel/kexec_loaded`; if that file is missing the DSM kernel has no `CONFIG_KEXEC` and
the automatic method is refused (use the installer kit). Many Synology kernels, especially ARM ones, do not have it.

## 2. A static `kexec` binary for the NAS CPU

DSM does not ship `kexec-tools`, and a dynamically linked build will not run against DSM's libc. The repository carries no
binaries; build them yourself:

```
# needs zig 0.13.0 and curl; builds x86_64 from kexec-tools 2.0.32 (source tarball SHA-256 pinned)
python3 -m venv zigenv && zigenv/bin/pip install ziglang==0.13.0     # zig from PyPI; put its folder on PATH
scripts/build-kexec.sh x86_64
file assets/kexec/amd64/kexec        # must say: statically linked
assets/kexec/amd64/kexec --version   # kexec-tools 2.0.32
```

**Status (2026-11-09).** The `x86_64` build was run: with zig 0.13.0 it produces a statically linked 64-bit ELF of about 214 KB that starts and prints
`kexec-tools 2.0.32`. Two things had to be fixed in the script to get there (the purgatory stub is built with `-march=x86-64`, which zig misreads, and with a
`-Wl,-Map=` option zig's linker rejects; the compiler wrapper now rewrites the first and drops the second). It has **not** been run on a NAS yet.
**`aarch64` and `armv7` do not build**: zig rejects `-mcmodel=large` for the aarch64 purgatory, and the armv7 purgatory link fails. Those units need their own work.

The manual GitHub workflow `kexec-static.yml` (also a job in `ci.yml`) builds the x86_64 binary on a runner and keeps it as an artifact `kexec-static` with its SHA-256 in the log.
The `setup-zig` action is not used: it could not download any zig version (404 on every mirror), so zig comes from PyPI, pinned.
The Windows release does not contain this binary yet, so for now put it next to the app or point the app at it (below).

synofurb looks for `assets/kexec/<amd64|arm64|armhf>/kexec` next to the executable or in the working directory, in
`$SYNOFURB_KEXEC`, or at the path given with `--kexec` / the GUI browse button.

## What happens on the NAS

1. Files go to a private directory under `/tmp` (or `/var/tmp`), mode 0700, with size-checked free space and **SHA-256 verified on
   the NAS** after each upload.
2. `kexec -l kernel --initrd=... --command-line='auto=true priority=critical ... console=tty0 console=ttyS0,115200n8 log_host=...'`
   (ARM adds `--dtb=`), then `kexec_loaded` must read `1`. Kernel and initrd are deleted from `/tmp` once loaded.
3. After your confirmation: `sync`, `echo s`/`echo u` to `/proc/sysrq-trigger` (emergency sync and remount read-only, so preserved
   volumes are left clean), then `kexec -e`. The SSH connection drops; that is expected.

If the NAS does not contact the desktop within 10 minutes the job stops with advice (check the serial/VGA console and the desktop firewall).
