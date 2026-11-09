#!/bin/sh
# Test a statically linked kexec WITHOUT a NAS (docs/kexec.md).
#
#   scripts/test-kexec.sh path/to/kexec [path/to/kernel]
#
# kexec prepares everything in user space (it parses the kernel, places the initrd and relocates its "purgatory" stub) before it makes the
# kexec_load syscall. That user-space part is where the zig-built binary of the 0.2.4 and 0.2.5 releases failed on a real RS812+ with
# "Unhandled rela relocation: R_X86_64_29". It can be run on any x86-64 Linux computer, as any user:
#   * a build that is fine gets as far as the syscall, which an unprivileged user (or a container) is refused: "kexec_load failed: ..."
#     (as root on a machine with kexec it loads the kernel; the script then unloads it again);
#   * a broken build stops earlier, with "Unhandled ..." or "Cannot ...".
# The kernel is Debian trixie's installer kernel (the one the app uses) unless you pass one. Exit 0: passed, 1: failed, 2: could not run the test.
set -eu
KEXEC=${1:?usage: test-kexec.sh path/to/kexec [kernel]}
KERNEL=${2:-}
[ -x "$KEXEC" ] || { echo "$KEXEC is not an executable file" >&2; exit 2; }
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
if [ -z "$KERNEL" ]; then
  KERNEL="$WORK/kernel"
  curl -fsSL --retry 5 --retry-delay 3 -o "$KERNEL" \
    https://deb.debian.org/debian/dists/trixie/main/installer-amd64/current/images/netboot/debian-installer/amd64/linux \
    || { echo "could not download the Debian installer kernel" >&2; exit 2; }
fi
head -c 1048576 /dev/zero > "$WORK/initrd"
rc=0
"$KEXEC" --kexec-syscall -l "$KERNEL" --initrd="$WORK/initrd" --command-line='console=ttyS0,115200n8' > "$WORK/out" 2>&1 || rc=$?
cat "$WORK/out"
if grep -Eq 'Unhandled|Cannot|[Uu]nsupported|Segmentation|not found' "$WORK/out"; then
  echo "FAIL: kexec stopped while preparing the kernel in user space (before the kexec_load syscall)"
  exit 1
fi
if [ "$rc" = 0 ]; then
  "$KEXEC" -u >/dev/null 2>&1 || true
  echo "PASS: the kernel was loaded (and unloaded again)"
  exit 0
fi
if grep -q 'kexec_load failed' "$WORK/out"; then
  echo "PASS: user-space preparation succeeded; the kexec_load syscall itself was refused here (needs root and a kernel with kexec)"
  exit 0
fi
echo "FAIL: kexec exited with $rc and no recognised message"
exit 1
