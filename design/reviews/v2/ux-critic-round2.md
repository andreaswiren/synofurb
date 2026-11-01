# UX critic, round 2: desktop mockup v2 (revised)

**Verdict: REJECT.** Two blocking findings remain (NB1, NB2). All ten of my round-1 blockers (B1 to B10) are fixed. NB1 is a false sentence on the Keep-mode data-loss gate. NB2 is a missing failure state in the phase where first installs on real hardware are most likely to fail. Both are cheap to fix. This is the closest the mockup has been to approval.

Reviewed: `design/mockups-v2/desktop/index.html` (92 states in `screens.json`, `src/js1-7`), the revised `README.md`, the screenshots, `design/v2/UX-SPEC.md`, `docs/RECOVERY.md`, `docs/BOOT.md`, `README.md`, and the engine script `crates/synofurb-core/src/script.rs`. I opened the prototype in Chromium with Playwright and clicked, typed and tabbed through it again. I did not read other reviewers' files. I wrote nothing to `design/approvals/`. I did not edit the mockup.

Reproduce with `design/mockups-v2/desktop/index.html?t=<anything unique>&nofirst=1&bare=1&<params>#<screen>`. Add `t=` so the browser reloads instead of reusing state. Params are in `screens.json` and the README.

My checks and theirs: `check_flows.js` passes (101 checks). `check_words.js` passes (90 states, max 60, prose inside "data" spans now counted). `check_deadends.js` crashes on this machine (`page.waitForTimeout: Target page ... has been closed`), so I ran my own dead-end sweep. I clicked every enabled button on Failed (pre and post), Recovery (2 states), Boot manager, Manage health and storage, and Done. Everything produced a dialog, a toast, a navigation or a state change. The only exceptions were segmented toggles, the current tab, and one info icon ("About Reinstall DSM" on `facts=debian#recovery`) that showed nothing.

---

## Status of my round-1 findings

| ID | Status | Evidence |
|---|---|---|
| **B1** Failure screens dead-end | **Fixed** | See "B1 detail" below. |
| **B2** Recovery half dead | **Fixed** | All 5 rows have an outcome. Labels are distinct (Boot DSM, Open agent, Restore flash, Remove entries, Open guide). Dialogs: "Boot DSM?" (24 words), "Open the agent?" (25), "Restore boot flash?" (typed `RESTORE FLASH`), "Remove our entries?" (typed `REMOVE ENTRIES`, "Debian may no longer start"), "Reinstall DSM". With `facts=debian` Open agent, Restore flash and Remove entries are disabled with a reason ("No agent on the flash. Set it up under Boot manager."). The primary button follows the state: "Open guide" when disks are modified. The statuses chip row reads from the unit (`facts=`). |
| **B3** Status bar / Way back wrong | **Fixed** | I read the status bar on 88 of the 90 states (Settings and About have none). It now follows facts: Recovery says "Disks changed" with the "Disks: modified" chip, `facts=debian` says "Debian running", install-pnr (step 2) says "Flash changed", install step 6 says "Disks changed", unreachable says "Unreachable", health states say "Needs attention" or "Critical". Way back on Confirm now reads "Nothing is written yet. From installer step 3 there is no undo: a failure opens a 15-minute rescue shell, then the DSM loader. No data promise." The cancel-dialog screenshot (`install-cancel`) is now generated at the staging phase: "Before step 3 nothing on the disks has changed. The NAS returns to DSM within 60 seconds. Our entries stay on the boot flash until you remove them under Recovery." with status "Flash changed". |
| **B4** PONR hidden, counter overruns | **Fixed** | The progress card has a segmented bar with a "No return" flag under step 3, which becomes "No return passed". It is in the card, so it is never scrolled out. Live run at 1280x800 and 960x640: the marker stays visible in every sampled frame. Counter never exceeds the limit: "Phase 6 of 7", "Phase 7 of 7" after step 12. The Steps list is a normal accordion on the page's single scroller. |
| **B5** Twin protection | **Fixed**, with notes | Both twins now get the "Twin" tag, an Alert naming the other unit ("Same model nearby. 192.168.1.40 · SN ...3456") and a phrase with a suffix. Unit A: `ERASE DS918+ 3456`; unit B (.58): `ERASE DS918+ 7710`. I typed the plain phrase and the other unit's suffix on unit B: the plain phrase fails (no match), the other suffix fails, `ERASE DS918+ 7710` matches. The gate card carries `192.168.1.58 · SN 1930PDN557710` and the checkbox reads "This is 192.168.1.58 · …7710". Notes: N-R2-4 and N-R2-9. |
| **B6** No backup emphasis | **Fixed** for Wipe | Red Alert at the top of Confirm: "No backup found. 6.2 TiB on volume1 is erased." with an info button whose popover says "No Hyper Backup, USB Copy or rsync task was found on this NAS. Copy everything you need to another device first. After step 3 DSM and its data may be gone." The checkbox reads "I have a tested backup". The "Warning sdc" tag now names the disk. **The Keep variant of this line is wrong: NB1.** |
| **B7** Enter erases | **Fixed** | `gate=ready`, focus `#f-erase`: Enter does nothing (screen stays `confirm`, no dialog). Space and click start the install. Enter in the phrase field also does nothing. |
| **B8** Copy command not equivalent | **Fixed** | The terminal button now opens a "Command" dialog with the full command and a "Matches plan / Same choices as the screens." line. I checked six states. The command carries mode, RAID, copies, system size, free %, hostname, login, ssh-key, kernel, source, web-ui, timezone and boot-via. It changes with the plan: Hybrid 2-disk gives `--raid hyb2 --copies 2`; Keep gives `--mode keep --system-size 9G`; toggling Newest and Offline in the UI gives `--kernel newest --source offline`. Before the gate is complete the text says "The confirmation flags are added once the gate is complete" and the flags are absent; once the gate is complete it adds `--have-backup --accept-data-loss --confirm "ERASE DS918+"`. With a twin, the phrase carries the suffix (`"ERASE DS918+ 3456"`). Caveat N-R2-3. |
| **B9** Connect failure states | **Fixed** | `conn=empty`, `login`, `off`, `unreach`, `hostchanged`, and live `wrong` password all work. Each is a one-line Alert with an info button (details behind it). "SSH is off" opens "Turn SSH on: In DSM: Control Panel, Terminal and SNMP, tick Enable SSH service, Apply. Then retry." plus a Retry button. A changed host key opens a danger dialog that offers only Cancel: "synofurb will not continue." |
| **B10** Replace-disk plan | **Resolved by owner decision** | Desktop Storage is read-only with "Open web UI" ("Array degraded. Replace the disk in the web UI."). It shows members per array and arrays per disk (md0 sda/sdb/sdc, md1 sda/sdb/sdc/sdd; sdc "Failed" in md0 and md1), and a Hybrid layer ("Hybrid 1-disk · LVM vg0 · /srv/data · 15.88 TiB", with data1/data2/data3 arrays). The replace-disk plan itself now lives in the web UI, which is out of scope here. Carry-over: the web plan must cover md0 as well as md1 and put the backup line before the rebuild. |

### B1 detail

- **Open rescue shell** copies `ssh root@192.168.1.40` to the clipboard and shows "Rescue command copied". The command and a masked password are already on the page, each with a Copy button and (for the password) a show/hide eye. They do not vanish. The password copied is `rescue-Tn4k-Qe7`.
- **Rescue notes** dialog (41 words): "Debian is installed under /target; only the boot entry failed. Export diagnostics first, then look at the log." with copyable commands. `/target` and `/var/log/synofurb-install.log` match `script.rs`.
- The card says "Debian is installed." (derived from step 11 or later; otherwise "Disks are partly changed").
- **Clock end states**, run live with `run=1&clock=2`:
  - Pre-PONR: "DSM returns in" then "Waiting for DSM", then either "DSM is back. The NAS answered on its DSM address." with a Done button, or (`out=notback`) "DSM did not return. Power-cycle the NAS." with a Check again button.
  - Post-PONR: "Rescue shell closes in", then "Waiting for DSM", then "The NAS is in the DSM loader. DSM's system may be gone." with a Reinstall DSM button, which opens the find.synology.com dialog.
- **Lost contact**: `failed-post-lost` and `install-lost` ("Connection lost. The NAS keeps installing." with Reconnect). The claim holds: the engine streams the log from a background `tail -F | nc` (`script.rs:270-271`), so a dead log sink does not stop `stage2.sh`.

---

## Blocking findings (new)

### NB1. The Keep-mode gate says volume1 "is erased" (High)

State: `unit=ds918&keep=1&sys=8#confirm` (also reachable by Setup, Advanced, Use Keep, then Next: Confirm).

Evidence: the top Alert reads "No backup found. 6.2 TiB on volume1 is erased." On the same screen the Preserved group lists "volume1 · best effort". The Wipe sentence is reused unchanged. The Keep dialog, plan and Help all say the data volume is left alone on a best-effort basis.

Why it hurts: the gate's whole job is to say exactly what happens. In the mode where the user is trusting the tool to keep their data, the loudest line on the page says the opposite. A user who reads it and ticks "I have a tested backup" is being told two different things. "Preserved" is also the only mode where a wrong word is a legal-grade promise (README: "Even in keep mode there is no guarantee").

What would convince me: in Keep mode the line reads along the lines of "No backup found. 6.2 TiB on volume1 is kept on a best-effort basis; it can still be lost." with the same popover. A screenshot of Keep Confirm (`confirm-keep` is not in `screens.json`; please add it, because the Keep gate has no committed screenshot).

### NB2. No failure state for the staging phase, which is where first installs on real hardware are most likely to fail (High)

State: only three failure variants exist: `fail=stopped`, `fail=pre` (always "Step 2 of 12 · Preflight: match disks", cause "Disk changed after confirm", 60 s clock) and `fail=post` (always step 11). The dev menu lists nothing else.

Evidence:
- `docs/RECOVERY.md` has a separate row: "Agent installed, armed ... if nobody claims it in N minutes it reboots, which boots DSM." The mockup's own Boot manager says "30 min watchdog". The install screen's first phases are "Boot flash backup, Boot agent added, Reboot into the agent, Send installer".
- A failure in those phases (flash backup hash mismatch, agent never answers after the reboot, installer send fails) has no screen. There is no failure screen with a 30-minute clock. The generic `fail=pre` screen says "DSM returns in 0:42" (60 s), "Disks untouched", "Disk changed after confirm", which is the wrong cause and the wrong clock for "the agent did not answer".
- `docs/BOOT.md` states that none of this has run on a real Synology. This is the first thing that will break for the first user, on an Untested unit.

Why it hurts: the user sits in front of "Reboot into the agent" with the NAS offline. The app must tell them how long to wait, that DSM will come back by itself, and that the boot flash holds our entries (and a backup on this computer). Today the nearest screen gives a 60 s promise that the product does not make in this phase.

What would convince me: one more failure variant for the staging phase: cause line ("The agent did not answer"), the watchdog clock (30 min), "Disks untouched", "Flash changed", "DSM: Default", an "After the clock" outcome (DSM back, or power-cycle), and a flash-backup-mismatch variant where nothing was changed. Parameters so I can open it.

---

## Non-blocking findings

**N-R2-1. Confirm never states what will be built (Medium).** `unit=ds918#confirm`. Destroyed, Changed and Preserved are good. Nothing says "RAID 5, 32 GiB system, hostname ds918". The only places the resulting plan is shown are the Command dialog (a long command line) and Quick setup's one-time dialog. From the Storage tab "Next: Confirm" skips the System tab entirely, so hostname, user, password, kernel and source can reach the gate unseen. A one-line "Will install: ..." with a popover would close it.

**N-R2-2. Defaults leak from the wrong unit (Medium).** `unit=ds1621#system` shows hostname `ds918`, and Quick setup on the DS1621+ says "ds918 · andreas". The Command dialog then carries `--hostname ds918` for a DS1621+. Fixture issue, but it is on the gate's command and sits next to the twin protection.

**N-R2-3. "Matches plan" is a hard-coded label (Low).** The command is built by the same function as the screens, so the claim is true today, but there is no test that fails when a screen adds an option the command lacks. Add one (a fixture that sets every control and compares).

**N-R2-4. The twin suffix appears only when a twin was seen (Low; declared deviation).** In the default demo (`unit=ds918`) the phrase is plain `ERASE DS918+`; the suffix appears only with `twin=1`. If the twin is off, on another VLAN or not yet scanned, the gate is weaker without telling anyone. Always appending the last four digits (or the IP) would make the muscle memory the same everywhere. I accept the declared deviation from UX-SPEC D7 as an improvement.

**N-R2-5. Quick setup silently prefers a worse layout (Low-Medium).** `unit=ds1621#goal` then Quick setup: "Wipe all 6 disks · RAID 6 · 7.15 TiB usable". The Plan tab for the same unit says "Hybrid: +12.7 TiB" (Hybrid 1-disk gives 19.85 TiB and survives 1 disk). Quick setup is a recommendation and should mention the trade ("Hybrid would give 19.85 TiB, survives 1 disk") or pick it.

**N-R2-6. "Recommended" still sits on Boot manager (Low; test it).** `unit=ds918#goal`. Unchanged from N4. With Quick setup now on the Install card, the person who came to reinstall has an even clearer path. The Recommended ribbon on the "try first" goal still needs a usability test.

**N-R2-7. Small targets (Low).** Info and "Why locked" buttons are 24x24 px, "About backups" 24x24, "How recovery works" 24x24, "Way back" 24 px high (brief minimum: 32 px). Tab count to Erase fell from 18 to 15 (stepper items are no longer tab stops). `size=largest` at 960x640 shows no horizontal scroll and no clipped footer button on any of the 90 states, which is good.

**N-R2-8. Odd or wrong states and copy (Low).**
- `unit=ds918&facts=debian#recovery` is "Flash: clean · Default: Debian · Disks: untouched · Nothing to recover. The unit is as it was found." Debian cannot be the default with a clean flash and untouched disks.
- Boot-manager progress (`boot=progress`) lists "Send installer", which does not happen in that flow.
- The twin units' disks show identical serial suffixes (`H8D4`, `L2N6`, ...); real units would differ.
- Done > Next steps still says "Check the BIOS boot order"; on a boot-flash unit `docs/BOOT.md` says the firmware boots the flash, not the disks.
- `fail=*` cause text is fixed ("Boot entry check failed") whatever the step, which is fine for the mock but must follow the engine in the real app.

**N-R2-9. Declared deviations, accepted.**
- Actions that leave the app are toasts. The only one that mattered, "Open rescue shell", is backed by persistent on-page commands, so I accept it.
- "Open web UI" is a toast. It is the only way to replace a failed disk after the owner's decision, so the web UI becomes a dependency of recovery: it must be reachable and signed in. That is for the web review.
- The mgmt update is allowed with a warning ("An array is degraded or rebuilding. Updating is allowed."). The dialog explains the 10 second restart and the automatic rollback. Acceptable.
- Setup tabs: the stepper shows six steps (Find, Connect, Goal, Setup, Confirm, Install); numbering and the "Done" placement are consistent.

---

## What is good now

- **Failure design is complete up to the staging phase**: persistent rescue command and password, honest clock end states, a state for lost contact, and Recovery with real, confirmed, disabled-with-reason actions.
- **The gate is honest** (Wipe): no-backup line, disk named, identity in the card, twin suffix, Enter ignored, locked button explained by a "Why locked" button and a click popover (a change from hover tooltips; keyboard reachable, 15 Tab stops).
- **The status bar tells the truth** across 90 states.
- **Copy command is real** and tied to the plan.
- **Install progress** has the point of no return in the card and a connection-lost state.
- **Text size** (Default, Large, Largest) scales the whole window; 960x640 at Largest holds.
- **Help is contextual**: F1 opens RAID levels on Plan, Recovery on Confirm and Failed, Troubleshooting on Connect, First install on Install.

## What I could not verify

- Real GPUI behaviour and screen readers; widget fidelity beyond the spec's mapping.
- Contrast and the type-scale checks (`check_contrast.js`, `check_scale.js`): I did not recompute them. A UI critic should.
- The dark theme beyond a glance at Confirm in round 1; the web UI; the engine's real behaviour after a lost connection beyond `script.rs:270-271`.
- The Keep gate has no screenshot, so I verified NB1 by reading the DOM.

## What would turn this into an APPROVE

1. Keep-mode backup line fixed (NB1) and a Keep Confirm screenshot.
2. A staging-phase failure state with the 30-minute clock and its end states (NB2).
3. Re-run my checks: Enter on Erase, twin both ways, copied command vs plan (add a fixture for every control), clock end states, status bar across states, PONR visible in the live run.

N-R2-1 and N-R2-2 I would like fixed in the same pass, but I will not hold approval for them.
