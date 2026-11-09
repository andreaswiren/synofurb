# Handover to a local Claude Code session (2026-11-09)

Written by the cloud session that built synofurb 0.2.x, so a session **on the owner's PC** (which can reach the NAS) can continue. The cloud session could not
reach the owner's LAN, could not run `kexec_load`, and could not edit published GitHub releases. A local session can do all three.

Read this file first, then `CHANGELOG.md` (0.2.3 to 0.2.6), `docs/kexec.md`, `docs/BOOT.md`, `docs/RECOVERY.md`, `docs/HARDWARE-TEST-PLAN.md`, `docs/RESET.md`.

## 0. Ground rules from the owner (standing)

* **Reports are honest.** Say what was run and what was not. No "should work" without a run behind it. Unverified goes in the docs as unverified.
* **Never start the real install on the NAS without the owner's explicit go-ahead in that message.** The test unit is the owner's **RS812+ at 192.168.67.133** (admin
  user over SSH, key or password; never ask for or paste the password into a file or a commit). It holds **168 GiB of data**. A failed install after step 3 leaves it without DSM.
* Do not start sub-agents unless the owner asks (they have interrupted runs twice).
* UI rules: at most 60 words per state, Lucide icons, tokens, one primary action per screen, the disabled-reason pattern, tooltips only on focusable controls.
* `/home/user/forgevault` (cloud path; the owner's other project) is read-only reference. Never modify it.
* Releases are **unsigned** and the **design gate is waived** by the owner (the workflow prints the gate report and goes on). Do not write critic approval files.
* Publishing a release: ask first unless told "publish". Prefer `publish: false` (artifacts only), let the owner test, then publish the same build with `reuse_run_id`.

## 1. Where things stand

| Item | State |
|---|---|
| Repo | `andreaswiren/synofurb` (public). Work branch `claude/synofurb-debian-reinstall-34mpul`, releases are cut from `main` (merge the branch in, keep trees identical). |
| Published | v0.2.0 to v0.2.5 (Windows only, unsigned). **v0.2.0 to v0.2.2 install a Debian without an init system (it cannot boot): their notes still lack that warning** (see section 6). |
| v0.2.5 | published. Its bundled `kexec` is **broken** (built with zig): `kexec -l` fails on the RS812+ with `Unhandled rela relocation: R_X86_64_29`. |
| 0.2.6 | **fix is committed on the branch/main but not released.** Native-gcc `kexec` build, `scripts/test-kexec.sh`, workflows switched. Needs a build and a test on the unit. |
| RS812+ | **no install has completed.** Three attempts stopped safely before anything was changed (see section 2). Disks untouched. |

## 2. The RS812+ attempts (evidence)

Unit facts (from `synofurb-cli inspect --host 192.168.67.133 --json`): RS812+ (`synology_cedarview_rs812+`), DSM 6.2.4-25556, kernel 3.10.105, Intel Atom D2700, 3.7 GiB RAM,
4 x WD30EFRX (2.73 TiB each), md0 (DSM system) / md1 (swap) / md2 RAID5 + LVM `vg1000/lv` ext4 (`/volume1`, 168 GiB used). Console `ttyS0` 115200. `/sys/firmware/efi` exists (**firmware reads as UEFI**),
`kexec_loaded` exists. Internal boot flash `synoboot` is a 122.5 MiB **USB** device (GPT, partitions of 32067 and 192780 sectors = 15.7 and 94 MiB) with **no DS918+-style markers**
(`DOM|250880|32067|192780|0|0`), so synofurb does not treat it as editable and plans a UEFI layout (GRUB on a mirrored ESP on the disks).

| Run | App | Failure | Cause | Fixed in |
|---|---|---|---|---|
| 1 | 0.2.3 | `No static kexec binary for amd64 found` | the Windows release shipped no kexec | 0.2.4 (bundled `assets/kexec/amd64/kexec`) |
| 2 | 0.2.4 | `kexec -l failed: Failed to copy decompressed buf` | kexec-tools tries `kexec_file_load` first, which needs `memfd_create` (Linux 3.17); DSM 6 has 3.10; no fallback | 0.2.5 (`--kexec-syscall`) |
| 3 | 0.2.5 | `kexec -l failed: Unhandled rela relocation: R_X86_64_29` | the zig-built kexec's *purgatory* stub has a GOTPC64 relocation kexec cannot process | **0.2.6 (not yet released)**: native gcc build |

What run 3 proved on the unit: the bundled `kexec` uploaded, its SHA-256 matched on the NAS, it ran (`kexec-tools 2.0.32`) on DSM's 3.10 kernel, and `kexec_load` was reached. Nothing after
`kexec -l` has ever run on the unit: the kernel has not been loaded or executed, the Debian installer has not started, and nobody knows yet whether Debian will boot from the disks afterwards.

## 3. The 0.2.6 fix and how it was tested

* `scripts/build-kexec.sh x86_64` now builds **natively with gcc** (static glibc, "for GNU/Linux 3.2.0") when run on x86-64 Linux with gcc, make, curl, xz. zig is only used for ARM, which does **not** build (zig rejects `-mcmodel=large` for aarch64; armv7 purgatory link fails).
* `scripts/test-kexec.sh KEXEC [KERNEL]` runs the user-space half of `kexec -l` (kernel parse, initrd placement, purgatory relocation) against Debian trixie's installer kernel without a NAS. It **fails on the zig build with the exact error from the NAS** and **passes on the gcc build** (which reaches the `kexec_load` syscall; an unprivileged user or container is refused there: `kexec_load failed: ...`). Both controls were run on 2026-11-09.
* `.github/workflows/release-windows.yml` job `kexec` builds with the runner's gcc, runs `scripts/test-kexec.sh` and refuses to package a failing binary; `ci.yml` job `kexec-static` and the manual `kexec-static.yml` do the same. The MSI smoke test checks `assets\kexec\amd64\kexec` is installed next to `synofurb.exe`; both zips are inspected.
* Same commit also: `--kexec-syscall` (0.2.5), a plan warning for a boot flash synofurb cannot edit (0.2.5), reset-help for 78 models (0.2.4), Confirm alignment fixes (0.2.4), release notes generated with "What changed" from `CHANGELOG.md` and the kexec SHA-256.

Not verified by anyone: `kexec_load` on the real unit with this binary; that the loaded installer runs; Debian booting afterwards; static glibc 2.39 start-up on the Atom D2700 / DSM 3.10 (expected fine: minimum kernel 3.2).

## 4. First thing to do locally: a NON-destructive load test on the NAS

Loading a kernel with kexec changes nothing until `kexec -e` is run. This reproduces exactly what the app does at the failing step, without the install. **Never run `kexec -e` by hand.**

1. Get the 0.2.6 `kexec`: either build it (WSL Ubuntu or any x86-64 Linux: `sh scripts/build-kexec.sh x86_64`, then `sh scripts/test-kexec.sh assets/kexec/amd64/kexec`, expect PASS) or download the `kexec-amd64` artifact of a `release-windows` run.
2. The app cached the Debian installer here: `%APPDATA%\synofurb\synofurb\data\cache\debian-trixie-amd64\` (`kernel`, `initrd.gz`; `firmware.cpio.gz` is 496 MB, skip it for this test).
3. From PowerShell:
   ```
   scp .\kexec admin@192.168.67.133:/tmp/kexec-test
   scp "$env:APPDATA\synofurb\synofurb\data\cache\debian-trixie-amd64\kernel" admin@192.168.67.133:/tmp/kx-kernel
   scp "$env:APPDATA\synofurb\synofurb\data\cache\debian-trixie-amd64\initrd.gz" admin@192.168.67.133:/tmp/kx-initrd
   ssh admin@192.168.67.133
   ```
4. On the NAS:
   ```
   chmod +x /tmp/kexec-test; /tmp/kexec-test --version                 # kexec-tools 2.0.32
   sudo /tmp/kexec-test --kexec-syscall -l /tmp/kx-kernel --initrd=/tmp/kx-initrd --command-line='console=ttyS0,115200n8'
   cat /sys/kernel/kexec_loaded                                         # 1 = loaded; nothing has run
   sudo /tmp/kexec-test -u; cat /sys/kernel/kexec_loaded                # unload; 0
   rm -f /tmp/kexec-test /tmp/kx-kernel /tmp/kx-initrd
   ```
5. If the load fails, the message is the next bug: read it before touching the code. Useful: `dmesg | tail -30`, `cat /proc/iomem`.

Only after that works, and the owner says go, run the app's install (GUI or `synofurb-cli install --host 192.168.67.133 ...`). Have the serial console (ttyS0, 115200) or a monitor attached if the owner can. After the point of no return (step 3 of the stage-2 script) there is no undo; recovery is Mode 2 (`synofurb-cli reset-help RS812+`, RESET button on the **back panel**) then a DSM reinstall from find.synology.com.

## 5. Open design questions (owner decides)

1. **Will Debian boot from the disks on the RS812+?** The plan installs GRUB on the disks' ESP (removable path, NVRAM updates off). The firmware may keep starting Synology's loader from the USB flash instead. The plan now warns about this; nobody has verified the firmware's behaviour.
2. **Two fallback designs** (the owner asked for the second as "a secondary option"):
   * **Edit the flash** like the DS918+ route: needs this flash's layout. Read-only commands for the owner/local session: `sudo fdisk -l /dev/synoboot; sudo blkid /dev/synoboot1 /dev/synoboot2; sudo mkdir -p /tmp/b1 /tmp/b2; sudo mount -o ro /dev/synoboot1 /tmp/b1; sudo mount -o ro /dev/synoboot2 /tmp/b2; ls -lR /tmp/b1 /tmp/b2 | head -80; cat /tmp/b1/grub/grub.cfg /tmp/b2/grub/grub.cfg; sudo umount /tmp/b1 /tmp/b2`. The synofurb-cli `backup-dom` command can also save an image.
   * **DSM as first-stage loader**: keep DSM's system and swap partitions, install Debian in the rest, and let a DSM boot-time script `kexec` into Debian (kernel and initrd on a small FAT partition DSM can read), with an attempt counter so a broken Debian falls back to DSM. Same mechanism as the installer's kexec, so it depends on section 4 working. Needs a new storage mode (keep partitions 1 and 2, reclaim 5), a hook that survives DSM (`/usr/local/etc/rc.d/` on DSM 6.2.4 is unverified), a success flag written by Debian. Not started.
3. The compatibility note for the RS812+ in `compat/models.json` still says "almost certainly legacy BIOS"; the unit reports UEFI. Update it from the evidence above if the owner agrees.
4. ARM kexec binaries (aarch64, armv7) do not build; they need a proper per-target gcc cross toolchain, not zig.

## 6. Housekeeping the cloud session could not do

* Add a warning ("this release installs a Debian that cannot boot; use 0.2.3 or later") to the notes of **v0.2.0, v0.2.1, v0.2.2**, and the missing "What changed" to **v0.2.3**. The cloud session was blocked from editing releases; the owner asked, a local `gh release edit` can do it. Ask the owner first. For v0.2.3 take the text from the `## 0.2.3` section of `CHANGELOG.md` (it lists the init-system fix first).
* Dependabot PR #1 (action version bumps incl. `softprops/action-gh-release`) is unreviewed.
* Signing is not set up (owner would run `minisign -G` and add the secret; Windows code signing not set up).
* The desktop app does not show the factory-reset help yet (CLI and `docs/RESET.md` only). The RS812+ button location is recorded (back panel); the other 77 models point to their Synology download page.
* "Read-only" status chip stays on during the Install screen's first phases; the word is misleading. A copy decision, not changed.
* Web (management plane) review items m5 and a part of m4 are open; desktop "Connect anyway" for an unreachable added address is open.

## 7. Tests: what exists and the expected result

Run from the repo root. On a machine with little disk use `CARGO_INCREMENTAL=0` and delete `target/debug/deps` test binaries between runs (the cloud sandbox kept running out of space).

| Check | Command | Expected (0.2.6) |
|---|---|---|
| Format | `cargo fmt --all --check` | clean |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| Rust tests | `cargo test --workspace` | core 367 (3 ignored), binary 189, cli 106 and 42, boot_chain 10, all pass |
| CLI smoke | `cargo build -p synofurb --no-default-features --bin synofurb-cli` then `bash tests/cli/smoke.sh target/debug/synofurb-cli` | passed 105, failed 0 |
| kexec | `sh scripts/build-kexec.sh x86_64 /tmp/kx && sh scripts/test-kexec.sh /tmp/kx/amd64/kexec` | `PASS: user-space preparation succeeded ...` |
| Release tooling | `python3 -m unittest discover -s tests/gate`, `python3 -m unittest discover -s tests/sbom`, `bash tests/install/test-package-scripts.sh` | OK / OK / passed 10, skipped 1 |
| Shell | `shellcheck -S warning scripts/*.sh` (and `shellcheck -s sh scripts/build-kexec.sh scripts/test-kexec.sh`) | clean |
| Management plane (Python, stdlib) | `python3 -m unittest discover -s apps/mgmt-plane/tests` (about 3.5 minutes; needs node + playwright for the browser tests) | 609 tests OK. Not touched since 0.2.3 except the `dig` test fix. |
| Windows GUI | not runnable headless on Windows. On Linux: `crates/synofurb/assets/tools/gui-walk.sh` (Xvfb + lavapipe) | screens render, no clipping |

Version is in `Cargo.toml` (`[workspace.package]`, currently 0.2.6) and `Cargo.lock`; the management plane has its own `apps/mgmt-plane/VERSION` (0.6.0). Every change updates `CHANGELOG.md` (and docs).

## 8. Releasing (what the workflows do)

* `release-windows.yml` (inputs `tag`, `publish`, `prerelease`, `reuse_run_id`, `target_sha`): job `kexec` (gcc build + test) then `build` (MSI + NSIS + portable zip + CLI zip, MSI silent-install smoke test, kexec-installed check) then `publish` (SHA256SUMS, notes with "What changed", GitHub release). The tag must equal `v<Cargo.toml version>`.
  * Build only: `gh workflow run release-windows.yml --ref main -f tag=v0.2.6 -f publish=false`; the installers are the `release-windows` artifact of the run (30 days).
  * Publish that same build: `-f publish=true -f reuse_run_id=<run id> -f target_sha=<head_sha of that run>`.
* `release-sbom.yml -f tag=v0.2.6` attaches the SBOM (CycloneDX + SPDX) and THIRD-PARTY-LICENSES.txt to an existing release. Run it after publishing.
* The merge to `main` must leave a tree identical to the tested branch tree (check with `git diff --stat main origin/<branch>`).

## 9. Where things are

* Core engine: `crates/synofurb-core/src` (`kexec.rs` stages and loads, `session.rs` drives an install, `plan.rs` storage plan and warnings, `inspect.rs` facts, `domboot.rs` boot-flash editing for DS918+-style units, `reset.rs` reset help). Stage-2 installer script is generated (`script.rs`, `payload/`); the app saves it to `%APPDATA%\synofurb\synofurb\data\logs\stage2-<id>.sh` for review.
* Desktop UI: `crates/synofurb/src/ui` (GPUI + gpui-component 0.8.1); CLI: `crates/synofurb/src/cli`.
* App logs and the diagnostic bundle (plan JSON, stage-2 script, log): `%APPDATA%\synofurb\synofurb\data\logs`; disk-metadata backups: `...\data\backups`.
* Management plane (on the NAS after install): `apps/mgmt-plane` (Python stdlib), add-on catalog `apps/mgmt-plane/addons/`, certificates + HAProxy `synofurb_edge.py`.

## 10. A starting prompt for the local session

> Read `docs/HANDOVER-LOCAL-SESSION.md` and `CHANGELOG.md`. Check out `main`. Do section 4 (the non-destructive kexec load test on the RS812+ at 192.168.67.133) and report exactly what each command printed. Do not run the real install and do not publish anything until I say so.
