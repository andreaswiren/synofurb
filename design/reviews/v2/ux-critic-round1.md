# UX critic, round 1: desktop mockup v2

**Verdict: REJECT.** 10 blocking findings (B1 to B10), 10 non-blocking (N1 to N10).

Reviewed: `design/mockups-v2/desktop/index.html` (opened in Chromium with Playwright and clicked, typed and tabbed through), the 118 screenshots in `design/screenshots/v2-mockup-desktop-*`, against `design/BRIEF-v2.md`, `design/v2/UX-SPEC.md`, `docs/RECOVERY.md`, `docs/BOOT.md`, `README.md` and the owner's words. I did not read any other reviewer's file. I wrote nothing into `design/approvals/`. Approval is the owner's call.

How to reproduce: every "State" below is a URL for the prototype, `design/mockups-v2/desktop/index.html?fast=1&nofirst=1&bare=1&<params>#<screen>`. Viewport 1280x800 unless said otherwise.

## What improved (and what does not need arguing)

The owner's main complaint is fixed. I counted every visible word on every state in `screens.json`, including the spans the checker skips as data.
- Primary screens are sparse: Find 76, Goal 74, Confirm 113, Failed 80. Of those, only about 55 on Confirm are prose; the rest is disk names, sizes and IDs.
- Titles, subtitles, buttons and labels are within the limits.
- All 106 tooltips I collected are 24 words or fewer (one has two sentences). The three popovers I opened run 7 to 32 words, the 16 dialogs 6 to 75, the 14 Help topics 70 to 135.
- `node check_words.js` and `node check_flows.js` pass (52 states, 54 checks). They test none of what follows.

Also good, verified by hand:
- **Locked Erase** is focusable and explains itself on hover and on keyboard focus: "Missing: backup tested, data loss accepted, unit confirmed, the phrase."
- **Disabled-reason tooltips** exist on 12 of the 13 distinct disabled controls I found (the exception is a disabled System field). Wording is weak on "Fix the error first."
- **Phrase** is case-sensitive; space and trailing-space variants fail. The mismatch tag does not echo the expected value.
- **Host-key dialog** focuses Cancel first and Esc returns focus to the Connect button.
- **PONR marker** in the install list ("No return" divider between steps 2 and 3) and the Boot flash accordion (backup, agent, reboot, send installer) are good. The flash flow matches `docs/BOOT.md`.
- **Pre-failure state** ("Disks untouched", "Flash changed", "DSM returns in 0:42") is an honest failure screen.
- **Contextual popovers** open the right Help topic: Data RAID info opens raid-levels, Way back opens recovery.
- **Recovery copy** matches `docs/RECOVERY.md`: 60 s before step 3, 15 min rescue shell after, DSM loader and the Synology page, "we promise nothing about your data". The Help "Backups" topic is honest that the disk-metadata backup is "not your files".
- **Layout**: no horizontal scroll at 960x640 on any of the 52 states.
- **Manage Health** has a clear overall status ("Critical", "sdc failed", "md1 degraded"). Management-app update has dialog, progress, failure and rollback states.

None of that makes up for the findings below. They concern the exact things the owner listed: data-loss warning with backup emphasis, safe failure and recovery, RAID management.

---

## Blocking findings

### B1. The failure screens stop being designed at the countdown (High)

State: `unit=ds918&fail=post#failed` and `fail=pre` and `fail=postkeep`.

Evidence:
- **"Open rescue shell"** is the only primary action after the point of no return. Clicking it shows an auto-dismissing toast reading `ssh root@192.168.1.40 (mock)`. The toast is gone after about 5 s. It has no password, no Copy button, no instructions for what to do in the shell. "Wait for DSM" (pre) is also just a toast.
- **The countdown stops at 0:00 and nothing happens.** I set the clock to 0:02: the 15 minute countdown on `fail=post` and the 60 second one on `fail=pre` both pin at `0:00`, and no new state appears.
- **Every state after the clock is missing.** For `fail=pre`: DSM is back, or DSM is not back after 10 minutes so power-cycle. For `fail=post`: the shell closed and the NAS rebooted into the DSM loader. The tool is supposed to detect and say that. These are the screens a frightened person will actually be looking at. They are not in `screens.json` and not in the code. `UX-SPEC` D9 says "Waiting for DSM shows Spinner and elapsed time"; it does not exist.
- **The post-failure error is a dead end for the user.** `fail=post` says "boot entry failed read-back; the original flash was restored and verified" at step 11 of 12. That means Debian is fully installed on the disks and only the boot entry failed. Neither the screen nor the "What happens next" dialog says so, or what the user should do inside the shell. The only guidance is "Export diagnostics first, then open the rescue shell". The old design had "Continue without the failing disk" and "Retry step". This one has no next step beyond "open a shell and figure it out".
- **No connection-lost or laptop-slept state for the install.** The Install screen says "Keep this window open." with no consequence stated. Searching the source finds no "connection lost", "reconnect" or "sleep". UX-SPEC D8 lists an Alert "Connection lost" plus Retry; it was not built. Closing the window or losing Wi-Fi after the point of no return is the most likely real failure.

Why it hurts: "what do I do now" is not answerable on the post-PONR screen or after the clock runs out. The owner asked for safe failure and recovery.

What would convince me: the post-clock states for pre and post failure (DSM returned, not returned, loader reached) as real screens. A rescue-shell action that shows a persistent, copyable command and where the password is, plus two or three exact first commands or a "Copy rescue notes" button. A sentence in the post-failure view on what is and is not intact (here: "Debian is installed; only the boot entry failed"). A designed connection-lost state with what the NAS does meanwhile.

### B2. The Recovery screen is half dead and misleading (High)

State: `unit=ds918#recovery`.

Evidence:
- I clicked all five "Run" buttons. "Boot DSM next", "Open agent" and "Remove our entries" produce no dialog, no toast, nothing. Only "Restore flash" (typed phrase) and "Reinstall DSM" open dialogs.
- "Remove our entries" edits the boot flash and has no confirm at all.
- "Boot DSM next" is the primary violet button while the chip row says "Disks: modified 14:25". In that state DSM's own system may be gone, so "Boot DSM next" boots a loader, not DSM. The screen does not say so.
- Nothing is disabled-with-a-reason; every action looks available regardless of unit state.
- All five buttons say "Run". A person who sees five identical buttons cannot tell the safe one from the destructive one without reading each row.

Why it hurts: recovery is a safety feature. A screen of five identical, partly inert buttons, led by a misleading primary, is the wrong shape for a frightened user.

What would convince me: every row has a real outcome (dialog or progress), destructive rows confirm, rows that cannot work say why in a tooltip, the button names the action ("Boot DSM", "Open agent", "Remove entries"), and the primary reflects the state read from the unit.

### B3. The status bar and "Way back" are wrong in the places that matter (High)

States:
- `unit=ds918#recovery`: status bar says **Read-only** while the chip on the same screen says "Disks: modified 14:25".
- `unit=ds918&gate=ready#confirm`: the Way back popover says **"Nothing to undo."** On the Erase gate itself. The one thing to know there is that after step 3 there is no undo.
- `unit=ds918&step=6&dlg=cancelinst#install` (this is the screenshot `v2-mockup-desktop-install-cancel-*`): the dialog says "Cancel install? The NAS returns to DSM within 60 seconds." The same screen shows step 3 of 12, status bar "Disks changed", and the Cancel button behind it disabled. Cancel is unavailable here. The shipped screenshot is evidence for a false promise.
- `unit=ds918&health=unreachable#manage`: status bar shows a green "Debian running" while the main area says "Cannot reach unit".

The spec says the risk tag is "derived from facts, never from step number". It is derived from the screen.

Why it hurts: the status bar is the product's trust anchor. Wrong on Recovery and Confirm, the two screens where a person decides whether to proceed or how to recover.

What would convince me: tag and Way back text computed from real unit state, tested on those four states. On Confirm, Way back should say what the exit is after step 3 (the 15 minute shell, then the DSM loader, no data promise). The cancel-dialog screenshot regenerated from a state where Cancel is allowed (step 1 or 2).

### B4. The point of no return and the current step scroll out of view during the install (High)

State: live run, `unit=ds918&gate=ready#confirm`, click Erase (no `step` param), watch at normal speed.

Evidence: I sampled the `.stp-scroll` list every second. From installer step 1 through step 4, the running row and the "No return" divider are both outside the scroll area (`HIDDEN`). The list is scrolled past them (`scrollTop` 140 at step 1, 172 of 172 from step 2 on). That is the exact moment the NAS crosses the point of no return. The frozen state `step=6` shows the same: the list starts at "5. Create RAID arrays".

Also: the progress card counts past the end: "Step 13 of 12", "Step 14 of 12", "Step 15 of 12" (Waiting for reboot, Verifying).

Why it hurts: the design's headline safety idea (a named, visible point of no return) is invisible when it happens. The counter looks like a bug in a destructive tool.

What would convince me: the list scrolls so the current row is in view; the PONR divider is pinned or scrolled into view as step 3 starts; the counter never exceeds 12 (phases 6 and 7 use the phase counter). Capture a screenshot of step 3.

### B5. Wrong-unit protection fails for the twin it was added for (High)

State: `twin=1#find` then select the second `DS918+` (SN ...557710, 192.168.1.58), continue to Confirm (or in the page: `S.unit` = TWIN then `go('confirm')`).

Evidence:
- With the first DS918+ selected: header shows an amber "Twin" tag and an Alert "Same model nearby 192.168.1.58 · SN ...557710".
- With the second DS918+ selected (the one at .58): **no "Twin" tag, no Alert.** The check is `S.twin && u.id==='ds918'`, so it only fires for one of the two units.
- The typed phrase is `ERASE DS918+` for both. The gate has no serial, IP or disk serial in it. The checkbox "This is the unit" sits beside a phrase that does not distinguish units. The identity header is above, outside the gate card.
- Even where the Alert shows, it reports the other unit's address; it does not say which one you are about to erase.

Why it hurts: the gate's job is "is this the unit you mean". This is the case the whole wrong-unit idea exists for, and it fails in one direction.

What would convince me: both units get the warning; the phrase or the gate card carries something that differs between the two (last digits of the serial or the IP), shown inside the gate; the Alert says "You are about to erase ...".

### B6. Backup emphasis is missing from the data-loss gate (High; owner requirement)

State: `unit=ds918&gate=ready#confirm` and `#inspect`.

Evidence:
- The words "backup" appear once on Confirm, as a 3-word checkbox "Backup is tested" with the same weight as "I accept data loss" and "This is the unit". There is no sentence, no warning icon with text, no "back up first" in the page. The first-launch dialog says "Make a tested backup first" once, on Find. Help > Backups is two clicks away.
- The app already knows. Inspect shows "Backups: none found" (amber tag). That fact is not carried to Confirm. The old design carried the fact; this one drops it.
- The same weakness appears at Replace disk: the Operation plan (`health=faildisk`, Replace) starts a 9 hour RAID5 rebuild on a degraded array with no word about a backup.
- The "Disk warning" tag at the top right of Confirm does not say which disk; it needs a hover.

Why it hurts: the owner asked for a "data-loss warning with backup emphasis". A tool that erases 6.2 TiB must lead with that fact, specifically for this unit, not hide it in a checkbox.

What would convince me: one visible, terse line on Confirm that is true for this unit ("No backup found on this NAS. 6.2 TiB on volume1 will be erased.") with a popover for what to back up and how; the same on the Replace-disk dialog; the disk warning names sdc on the gate.

### B7. Enter on the focused Erase button erases (High)

State: `unit=ds918&gate=ready#confirm`; focus `#f-erase` (Tab ends there); press Enter.

Evidence: the screen changes to `install` on Enter. UX-SPEC D7 keyboard: "`Enter` never triggers Erase; `Space` on the focused button does." `check_flows.js` tests Enter in the phrase field only, not on the button.

Why it hurts: Enter is the key people hit to confirm a field they just typed. The spec says this exact accident must not erase a disk.

What would convince me: Enter on the focused button does nothing (or opens nothing destructive); Space or click does; tested in a script.

### B8. "Copy command" is not the equivalent command, and it is the only accessible path (High)

State: `unit=ds1621&raid=hyb2&copies=2&gate=ready#confirm` and `unit=ds918&keep=1&sys=9&gate=ready#confirm`; click the terminal icon; read the clipboard.

Evidence: both copy exactly the same shape: `synofurb install --have-backup --accept-data-loss --confirm --host <ip> --user admin`. Mode (Keep), RAID level (Hybrid 2-disk), mirror copies, system size, free space, kernel, source and boot method are all missing. The tooltip says "Copies the equivalent synofurb command." UX-SPEC F3 says the choices "are part of the Copy command output". The first-launch dialog and spec section 0.6 send screen-reader users to this CLI as the equivalent path.

Why it hurts: someone who followed the plan on screen (Keep, or Hybrid) and runs the copied command gets the CLI defaults: a Wipe install with default RAID. The flags `--have-backup --accept-data-loss --confirm` are already in it, so there is no second gate. This is the sole stated accessibility mitigation, and it erases something different from what was reviewed.

What would convince me: the command carries every chosen option; a test compares the copied command with the plan; until then the tooltip must not say "equivalent".

### B9. The first thing a new user hits, a failed Connect, has no states (High)

State: `unit=ds918#connect`.

Evidence: the only states are idle and the host-key dialog. `conn.err` is never set. Submitting with an empty password goes straight to the host-key dialog. There is no wrong-password screen, no "SSH is off" screen, no timeout, no changed-host-key (danger) dialog, though the spec's Trust-dialog row (a changed key shows the same dialog in danger style) and Help > Troubleshooting both describe them. The instruction for turning SSH on in DSM is nowhere on the Connect screen (only in Help). The product's one precondition is that SSH is on and reachable.

Why it hurts: this is the most common first failure on the primary flow. The user has "only SSH". The screen that first needs it is the one with no failure design. A first-conversion test would stop here.

What would convince me: screens or states for wrong password (with the DSM auto-block warning, as a tooltip or popover), SSH off (with the DSM menu path and a Retry), unreachable, and a changed host key in danger style with Cancel only.

### B10. Replace disk: the Operation plan ignores md0, and Manage cannot show Hybrid (Medium-High)

State: `unit=ds918&health=faildisk&tab=storage#manage`, click Replace disk.

Evidence:
- sdc sits in both arrays; the table shows md0 (RAID 1 ×3, the system) "Degraded" and md1 "Degraded". The Operation plan lists: "Mark sdc failed and remove it from md1", "Add the new disk", "Rebuild md1 (about 9 hours)". Nothing about md0. After the plan runs, the system array is still degraded. The plan also never says to insert a disk, or which slot, and offers no pre-check.
- The Manage > Storage tab shows only md0 and md1 and a disabled "Create array". No member disks, no scrub, no Hybrid layout, no slices. Hybrid RAID is visible only on the Plan screen at install time (`unit=ds1621&raid=hyb1`). The owner asked for "RAID + Hybrid management visible".

Why it hurts: replacing a disk is the main day-2 task in Manage. A plan that fixes one of two arrays is a correctness error, and the rebuild window is the most dangerous time for a RAID5.

What would convince me: the plan covers every array that contains the failed disk, names the physical step, and links to backup status; one Manage Storage state with a Hybrid unit (members, slices, scrub) so the owner can see it.

---

## Non-blocking findings

**N1. F1 Help is not contextual (Medium).** `F1` on Plan, Confirm and Failed all open the same topic list. UX-SPEC says F1 is "help for the current step". The popovers do link to the right topics, so the plumbing exists. On Failed, F1 should open Recovery.

**N2. Keep mode (hidden behind Advanced) is rough (Medium).** State `unit=ds918&keep=1#plan` and the Advanced > Use Keep path.
- After "Use Keep" the screen opens in an error: System size is set to 16 GiB and "Too large" shows at once. The app made the error, and the one-word alert does not say what is too large.
- "Set 9 GiB" leaves `/ 5 GiB · swap 4`. A 5 GiB root for Debian 13 with a 6.12 kernel is tight; the README sets 8 GiB as the minimum system size.
- Numbers on the Keep gate are wrong: `Destroyed 20 GiB` (Plan) and `DSM system · swap 2 × 10 GiB` (Confirm) on a four-disk unit whose DSM system and swap area is about 10 GiB per disk, so about 40 GiB.
- "Preserved: Data volume, best effort" does not name volume1 or its size.
- Recommendation: keep it hidden for v1, as the spec already suggests, and fix the numbers before it is shown.

**N3. Keyboard path is long, and several targets are small (Medium).** `fast=1&nofirst=1&bare=1&unit=ds918&gate=part#confirm`: 9 Tab stops (Help, Settings, Menu, six stepper items) before the first control on the page; 18 to reach Erase. The brief's own minimum is 32 px; I counted many 20 to 22 px targets at 960x640: "About 25 min", "How recovery works", "Way back", "Advanced", "Slices", "Show full", the info buttons. Confirm and Plan also scroll by 3 to 9 px at 1280x800, which looks like a layout slip. The phrase field has no focus ring of its own but its wrapper changes border to purple, so focus is visible.

**N4. Goal puts "Recommended" on Boot manager (Medium; test it).** `unit=ds918#goal`. The person came to reinstall; the recommended card is a different goal ("Try Linux. Disks untouched."). The idea is sound for an Untested model (try before you commit). It needs a usability test for how many people stall or pick wrongly. The Goal screen is at exactly 60 words by the checker.

**N5. Done screen details (Medium).** `unit=ds918#done`. "Password copied" is a plain self-attested checkbox; Finish unlocks without a copy action. "Next steps" is collapsed and holds the "replace sdc" and "RAID sync about 9 hours" items, so the failing-disk warning does not follow the user to the end. The certificate fingerprint is truncated; "Show full" is a 20 px link.

**N6. Unreachable and update-failed states give no help (Medium).** `health=unreachable#manage`: "Cannot reach unit" plus Retry and permanent skeleton bars. It offers no next step (check power, cable, scan again), shows no last-known data, and the status bar stays green. `mgmt=failed`: "Update failed. Rolled back to 0.9.1" with Retry, no reason.

**N7. The word checker undercounts honestly-labelled data (Low).** On Install the checker reports 46; real visible text is 131 words. The gap is the twelve step names (about 45 words) and other strings classed as data by the `.d` span. That is defensible as engine text, but it should be a stated rule. The 12 step names are also what hides the PONR (B4). The checker also ignores tooltips and popovers by design; the ones I counted are fine.

**N8. 960x640 hides the safety content on Plan and Confirm (Low).** `unit=ds918#plan` at 960x640 scrolls 116 px: Destroyed/Changed/Preserved summary and the Disk warning are below the fold. On Confirm at 960x640 the "Preserved" group is below the fold, so the only reassurance on the screen requires a scroll. The Stepper loses its labels (numbers only) at that size; the buttons have accessible names, which is fine.

**N9. Recovery and Manage labels (Low).** Five "Run" buttons (B2).

**N10. Things I could not verify (see end).**

---

## Against the owner's words

| Owner criterion | Result |
|---|---|
| Too much text, 60 words per screen, explanations behind tooltips and dialogs | **Pass.** Real counts 74 to 113 words on the main screens, of which about half is data. Tips, popovers, dialogs and Help are within bounds. |
| GPUI components used properly | **Cannot verify from HTML.** The spec maps every element to a gpui-component widget and lists exceptions; the prototype mimics them well. The custom pieces (disk diagram, countdown, steps list) are the listed exceptions. Confirm against the real app. |
| Help and About | **Pass in structure.** Help sheet: 14 topics, search, shortcuts, contextual popover links. About: version, copy diagnostics, credits. F1 is not contextual (N1). |
| Deep UX redesign | **Partly.** The flow is clean and quiet. The failure, recovery and connect branches are not (B1, B2, B9). |
| RAID + Hybrid management visible | **Partly.** Hybrid shows well in Plan, with slices and a weaker-slice dialog. Manage shows only a two-row array table (B10). |
| Manage Health view + management-app update | **Pass in states** (ok, rebuild, failed disk, service down, unreachable, update, updating, failed, updated, rollback dialog). Weak copy on unreachable and failure (N6). |
| Data-loss warning with backup emphasis | **Fail** (B6). |
| Safe failure and recovery | **Fail** (B1, B2, B3, B4). |

## What would convince me to approve

In order of weight:
1. B1, B2: every failure and recovery action has an outcome and every clock has an end state; the rescue-shell action is persistent and copyable.
2. B3, B4: status and Way back computed from facts; the PONR and current step visible during the live run.
3. B5, B6, B7, B8: the gate identifies the unit in both twin cases, leads with the backup fact for this unit, ignores Enter on Erase, and the copied command equals the plan.
4. B9, B10: Connect failures and a complete replace-disk plan with a Hybrid Manage state.

Then re-run: `node check_words.js`, `node check_flows.js`, plus new tests for Enter-on-Erase, the twin warning in both directions, the copied command against the plan, the clock reaching zero, and the PONR visible at step 3. I will review the real application screenshots with the same checks.

## What I could not verify

- Real GPUI behaviour: widget use, focus rings, text scaling, screen readers, scrollbar theming. Everything above is from the HTML prototype. The spec states that screen readers are not guaranteed; I did not test one.
- Contrast numbers: I did not recompute them. `check_contrast.js` exists; a UI critic should own it.
- Dark theme: looked at Confirm only; no problems seen.
- Whether the real engine can detect the post-failure states B1 asks for (DSM loader reached, rescue shell closed). The design must say what it can know.
- The behaviour of Synology's reinstall page and RESET steps on real hardware. The prototype's dialogs follow `docs/RECOVERY.md` and do not invent a RESET procedure, which is right.
- Web UI and the management plane: out of scope for this round.
