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
# needs zig (https://ziglang.org) and curl; builds x86_64, aarch64 and armv7 from kexec-tools 2.0.32 (SHA-256 pinned)
scripts/build-kexec.sh x86_64
scripts/build-kexec.sh aarch64
scripts/build-kexec.sh armv7
file assets/kexec/amd64/kexec        # must say: statically linked
```

The GitHub workflow `ci.yml` has a manually triggered `kexec-static` job that does the same on a runner. **The script has not been
run by the author** (the environment it was written in does not allow compiling downloaded sources): inspect the result and run
`kexec --version` before trusting it.

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
