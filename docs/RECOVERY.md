# If something goes wrong: fallbacks and recovery

Short answer: **yes, there is a fallback, and it is layered.** What you can get back depends on *when* the failure
happens, because once disks are repartitioned nothing can un-repartition them. This page says exactly what the
software does and what it cannot do. Nothing here replaces a backup of your data.

## The design rule

> **DSM stays the default boot entry. Every step that boots something else is one-shot, and every failure path points
> the default back at DSM.** One case is not covered: if the boot agent itself never starts, nothing has run to reset
> the default, so the unit may keep booting the agent. Then use the manual recovery below.

This applies to x86 units (Intel and AMD) that boot through the internal boot flash ("DOM"), which is what `synofurb`
uses on those units (see [boot/dom](../boot/dom/README.md)). The mechanism is the same idea the
[synodeb](https://github.com/rylos/synodeb) project uses for the DS923+.

## When the boot agent does not answer (read this first)

If the app asked the unit to reboot into the boot agent and the agent did not answer within **10 minutes**, the app stops and does **not** reset anything
(CLI exit code **10**, `SessionError::AgentTimeout` in the engine). It cannot know which of two things happened:

1. **The agent started** but is not reachable (network, DHCP, address). Its watchdog counts from the moment the *agent* started, not from the reboot
   request: after **30 minutes** (install) or **60 minutes** (boot manager mode) without a login or `sf-boot claim` it sets the default boot entry back to
   DSM and reboots. So the unit can still come back by itself, possibly 20 minutes or more after the app gave up.
2. **The agent never ran** (it failed to boot). Nothing has reset the default and there is no watchdog, so the unit may boot the agent **again after every
   power cycle**. A power cycle is not a recovery in this case.

What to do, in this order: wait (the watchdog may still fire); look at the unit's serial or VGA console; run `synofurb-cli recover wait --host <ip>`
to watch for DSM; if nothing changes, recover by hand, which needs physical access: choose **`SYNOLOGY_2`** in the boot loader menu on a serial or VGA
console for one boot and then run `synofurb-dom default dsm --yes` on DSM, **or** take the boot flash to another Linux computer and restore the
saved image (`synofurb-dom restore --yes FILE` with `SYNOFURB_DOM=/dev/sdX`; see "Manual recovery" in [boot/dom](../boot/dom/README.md)).
Once you can reach the agent or DSM over SSH, `synofurb-cli recover status --host <ip>` shows what the boot flash says and the actions below fix it.

## What protects you, by phase

| Phase | What has changed on the NAS | If it fails or you cancel | You get back |
|---|---|---|---|
| Scan, connect, inspect, plan | Nothing (read-only script) | Nothing to undo | DSM, untouched |
| **Boot flash backup** | Nothing; the whole DOM is streamed to *your computer*, hashed, and compared with a hash computed on the NAS | Abort, nothing changed | DSM, untouched. The backup file is kept (`dom-backups/`) |
| **Agent installed, armed** | Files + a menu entry added on the boot flash (DSM entry untouched, still the fallback) | The agent resets the default to DSM *as its first action*; if nobody claims it within its timeout (30 minutes in an install, 60 in boot manager mode, counted from when the agent boots; the app itself gives up after waiting 10 minutes) it reboots, which boots DSM. The app also resets the default if any step before the reboot fails | DSM, after a reboot. **Exception: if the agent never runs at all** (it fails to boot), nothing resets the default and there is no watchdog, so a power cycle boots the agent again. The app cannot tell this apart from an agent that runs but is unreachable. Recover from the boot menu on a console, or restore the boot flash from another computer (see "Manual recovery" in [boot/dom](../boot/dom/README.md)) |
| **Installer running, before the point of no return** | Still no disk changed | The generated installer script's failure handler sets the default to DSM and reboots after 60 s (boot-flash units only). A power cycle does the same once the agent has run, because the default is then already DSM | DSM, untouched |
| **After the point of no return** (disks being repartitioned) | The reclaimed partitions are gone (in *Keep volumes* mode DSM's own system partition; in *Wipe* mode everything) | Default is set to DSM; the unit reboots after 15 minutes (so you can read the log on the console) into the DSM loader. There is **no rescue shell and no SSH server** in the installer; on units without a boot flash (arm64/armhf, BIOS/UEFI kexec or kit installs) the installer halts and does **not** reboot by itself: read the log on the console, then power-cycle | The DSM **loader** runs, but DSM's operating system is gone: see "Reinstalling DSM" below. Preserved data volumes are untouched unless the failure was a hardware problem |
| **Debian installed** | Debian entry on the boot flash; previous kernel kept as `DEBIAN_OLD` | A kernel that fails to boot: pick `DEBIAN_OLD` (or DSM) in the boot menu / `synofurb recover` | Debian with the previous kernel, or DSM loader |

### Why "one-shot" matters when you cannot see a screen

You usually have only SSH. So no step is allowed to leave the unit in a state where the only way out is a keyboard.
The default boot entry is switched to Debian **only** by the very last successful step of an install
(`synofurb-dom default debian`), and back to DSM by any failure. If the unit ends up unreachable, powering it off and on
again is a valid recovery from the pre-point-of-no-return phases **once the agent has run** (it resets the default to DSM as its
first action). If the agent never started, a power cycle boots it again; use the manual recovery in
[boot/dom](../boot/dom/README.md) (the boot menu on a serial or VGA console, or restoring the boot flash from another computer).

## Commands

```
synofurb-cli recover status  --host <ip>           # READ the unit's boot state (default entry, our entries, agent files, flash backup). "not known" is reported as such
synofurb-cli recover boot-dsm --host <ip>          # set the default to DSM and read it back; does NOT reboot
synofurb-cli recover boot-dsm --host <ip> --reboot --yes   # ... and ask the unit to reboot ("reboot requested", nothing more)
synofurb-cli recover wait    --host <ip> [--timeout 15]    # probe until DSM answers (SSH banner + DSM web port); needs no login
synofurb-cli recover remove  --host <ip> --yes     # take synofurb's entries out of the boot menu, default DSM, verified by reading it back
synofurb-cli recover restore --host <ip> --backup FILE.img --yes   # write a saved flash image back (checked, read back)
synofurb-cli recover         --host <ip>           # older form: set the default to DSM (works from DSM, Debian or the agent), no reboot
synofurb-cli boot-manager enter --host <ip> --yes  # boot the SSH-controlled agent to try anything else
```
`recover status` is the only honest way to know where a unit stands. It runs `synofurb-dom status --json` on the unit (and `sf-boot status` on the agent). If the tool is
not installed or refuses, the answer is **NOT READ / NOT KNOWN**, which is a different thing from "DSM is the default": nothing is claimed about a menu that was not read.
`boot-dsm` verifies the default by reading it back and sends **no reboot if that did not verify**; `wait` reports `DSM is back` only when an OpenSSH banner and a DSM web
port (5000/5001) both answer. The states: *DSM is back*, *only DSM's web port answers* (DSM still starting, or only the DSM loader runs: keep waiting a few minutes, then see
"Reinstalling DSM"), *the boot agent answers* (dropbear: the unit is in the agent), *SSH answers but not DSM's web ports* (another system), *nothing answers*.
These checks are heuristics tested on this computer, not on a Synology.

On the unit itself: `synofurb-dom status`, `synofurb-dom default dsm`, `synofurb-dom restore <backup.img>`, `synofurb-dom remove`.
After `remove`, the boot flash contains only Synology's own files again (the tool refuses to finish if `zImage`/`rd.gz` are missing).

## Reinstalling DSM after an install went past the point of no return

If DSM's system partition was reclaimed and the install did not complete, the unit boots Synology's loader, which
brings up Synology's own recovery/installer web page. The usual way to use it is to open
<https://find.synology.com> (or Synology Assistant), find the unit and install DSM again with the `.pat` file for your
model from Synology's download centre. Synology documents this "reinstall DSM" procedure and states that data volumes
are kept when you choose to keep them, **but you must check Synology's current documentation for your DSM version, and
this project cannot guarantee it** (you lose DSM settings, packages and users either way; only files on intact volumes
can survive). That is why the confirmation screen says *no guarantee* and why a backup is not optional.

## What is NOT covered (be realistic)

* **A damaged boot flash.** If the DOM itself is corrupted (hardware wear or a bad write), the NAS may not boot at all.
  `synofurb` mitigates this by (1) writing only to *copies* of the partitions, verifying every write by reading it back and
  rolling back automatically, and (2) giving you a full image of the DOM before the first write. **Keep that image off the
  NAS.** Restoring it with `synofurb-dom restore` needs a working Linux on the unit (for example the Debian that is
  installed, or the agent), otherwise it requires Synology's service or a USB flash programmer. Never delete the backup.
* **Hardware failure, power loss in the middle of repartitioning.** Disk metadata can be left half-written. In *Keep volumes*
  mode the preserved data partitions are never part of a write command and their RAID superblocks are re-checked after the
  install; this reduces the risk but cannot remove it.
* **Units without an internal boot flash** (ARMv7/arm64 boards use U-Boot from SPI flash, which `synofurb` never writes): the
  stock U-Boot still boots DSM from flash unless *you* saved a changed environment over the serial console. To go back:
  `bootspi` / restore the saved `bootcmd` (the install log prints the original). Debian on these boards needs a serial adapter,
  so SSH-only recovery is not possible there.
* **A completely untouched DSM.** `synofurb` reclaims DSM's system partitions for Debian. If you want DSM to remain bootable
  with *no* reinstall ever needed, install Debian onto disks or SSDs DSM does not use (not offered yet in the planner: it is on the roadmap, and is how
  synodeb works).
* **Secure erasure / encrypted volumes.** Not handled.

## Reporting

Please use the compatibility report issue template (`.github/ISSUE_TEMPLATE/compat-report.yml`) when something fails: attach the
debug log (`~/.local/share/synofurb/logs/` on Linux). That is how a model moves from *Untested* towards *Verified*.
