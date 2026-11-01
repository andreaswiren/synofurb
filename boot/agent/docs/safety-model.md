# Safety model (fail-safe design)

Goal: a NAS that was switched into the agent must never be stuck there when nobody is driving it, and it must never be left
in a state where a power cycle does not get DSM back. The user may have **no** serial console and **no** USB stick.

## Layers

1. **The DOM boot default is DSM before anything else is attempted.** `boot.sh` starts the first action of the agent in the background at once,
   before drivers or network: `synofurb-dom --yes --skip-auto-backup default dsm`, retried every 3 s for two minutes while the storage drivers come up
   (the DOM block device appears late). From then on a reset, a power cut, a crash, a hang during `kexec` or a hung installer all end in DSM, because the
   agent was only a one-shot boot. This is the layer that protects the cases the agent itself can no longer act in.
2. **The watchdog (`svc-watchdog`)**: `AGENT_TIMEOUT_MIN` (default 20) after boot, if the unit is not claimed, it calls `synofurb-dom default dsm` *again*
   (belt and braces), syncs, runs `reboot`, and 20 s later, if the platform has not reset, forces it with `echo b > /proc/sysrq-trigger`.
3. **Claiming**: the unit is claimed — watchdog off — by (a) the first successful SSH login (`AGENT_WATCHDOG_MODE=login`, default), (b) `sf-boot claim`,
   (c) a started autorun script, (d) a successfully loaded `sf-boot kexec`. A claim is the app saying "I am driving"; it is not undone by time. `sf-boot arm [MIN]`
   takes it back. A failing autorun script re-arms automatically.
4. **Credentials fail closed**: no key and no password hash means nobody can log in (the console says so); a one-time password is revoked after its first use;
   an autorun script without a pinned SHA-256 is not run; downloads for `kexec` need hashes. The watchdog still protects the unit in all of these.

## What each failure does

| situation | outcome |
|---|---|
| nobody connects (wrong network, no DHCP, no key) | watchdog reboots after the timeout; `dom_default` was set to DSM at the start, so DSM boots |
| overlay config has a syntax error | defaults are used (no keys); same as above |
| `synofurb-dom default dsm` fails (no DOM found, damaged) | `dom_default=failed`, a WARNING on every console and in the log; the watchdog still reboots, but **whether that boots DSM then depends on the DOM component** (e.g. a one-shot boot entry). A stub build says `dom_default=missing` |
| the app logs in, then disappears | stays in the agent forever (claimed). Intended: the app owns the unit now. Run `sf-boot arm` from the app for a dead-man behaviour |
| `kexec` into the installer, installer hangs / crashes | agent no longer runs; a power cycle boots DSM (layer 1) |
| installer finishes and installs Debian | the installer/DOM component must set the default to `debian` (`sf-boot reboot debian` does it from the agent) |
| `reboot` hangs on odd hardware | sysrq `b` after 20 s |
| agent kernel panics | `panic=-1`/`panic=N` on the command line is advisable (the DOM component's call); with it the machine reboots into DSM |

## What the agent does *not* do

* It does not write anything to disks or the DOM itself except through `synofurb-dom`. `--skip-auto-backup` is used because the one-time full-DOM backup belongs to the installer
  that runs under DSM; in this RAM-only system it would only fill memory and vanish on reboot.
* It does not trust the network: no unauthenticated service except the SSH port and the (send-only) beacon. busybox `wget` does not verify certificates, so every download that leads to
  code execution (autorun script, kexec kernel/initrd from a URL) is pinned by SHA-256 supplied through the authenticated channel (the overlay, or the SSH command line).
* It has no secrets of its own: the host key is regenerated unless pinned; the overlay is the secret carrier and sits in the DOM — treat the DOM contents as confidential.
* The serial/VGA console gives a root shell without login. Physical access equals root anyway (the kernel console, the DOM, the disks), so this is by design; `AGENT_CONSOLE_SHELL=0` removes it.
