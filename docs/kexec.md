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
# x86_64, on an x86-64 Linux computer with gcc and make (what the release workflow does): a native static build
scripts/build-kexec.sh x86_64
file assets/kexec/amd64/kexec          # must say: statically linked
assets/kexec/amd64/kexec --version     # kexec-tools 2.0.32
scripts/test-kexec.sh assets/kexec/amd64/kexec    # must say PASS (see below)
```

The binary is kexec-tools 2.0.32 (source tarball SHA-256 pinned), linked statically against glibc for Linux 3.2.0 and up, so it runs on DSM's 3.10 kernel and needs nothing on the NAS.

**Do not build the x86_64 binary with zig.** The 0.2.4 and 0.2.5 releases did, and on a real RS812+ `kexec -l` failed with `Unhandled rela relocation: R_X86_64_29`: the purgatory stub
that zig produces contains a GOTPC64 relocation that kexec cannot process. `scripts/test-kexec.sh` reproduces that without a NAS (kexec prepares the kernel in user space before the
`kexec_load` syscall) and is part of the release workflow: it must reach the syscall. It fails on the zig build and passes on the gcc build (checked 2026-11-09).

`aarch64` and `armv7` still use the zig route in `scripts/build-kexec.sh` and **do not build**: zig rejects `-mcmodel=large` for the aarch64 purgatory, and the armv7 purgatory link fails.
Those units need a proper cross toolchain (gcc for the target), not zig. They are not built in CI.

The manual GitHub workflow `kexec-static.yml` (also a job in `ci.yml`) builds the x86_64 binary on a runner and keeps it as an artifact `kexec-static` with its SHA-256 in the log.
Windows releases from 0.2.4 contain the amd64 binary at `assets/kexec/amd64/kexec` next to the app (installers, portable zip and CLI zip); the release workflow builds it.
Other platforms' packages do not carry it yet; put it next to the app or point the app at it (below).

The load uses `kexec --kexec-syscall -l ...` (the classic `kexec_load` syscall). kexec-tools' default, `kexec_file_load`, needs `memfd_create` (Linux 3.17) and fails on DSM 6's 3.10 kernel with `Failed to copy decompressed buf`; kexec does not fall back from that failure by itself.

## What happens on the NAS

1. Files go to a private directory under `/tmp` (or `/var/tmp`), mode 0700, with size-checked free space and **SHA-256 verified on
   the NAS** after each upload.
2. `kexec -l kernel --initrd=... --command-line='auto=true priority=critical ... console=tty0 console=ttyS0,115200n8 log_host=...'`
   (ARM adds `--dtb=`), then `kexec_loaded` must read `1`. Kernel and initrd are deleted from `/tmp` once loaded.
3. After your confirmation: `sync`, `echo s`/`echo u` to `/proc/sysrq-trigger` (emergency sync and remount read-only, so preserved
   volumes are left clean), then `kexec -e`. The SSH connection drops; that is expected.

If the NAS does not contact the desktop within 10 minutes the job stops with advice (check the serial/VGA console and the desktop firewall).
