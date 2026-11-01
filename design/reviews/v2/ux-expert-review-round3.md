# UX expert review, round 3: desktop prototype v2

Reviewed: `design/mockups-v2/desktop/index.html` (431 KB, `screens.json` with 104 states) opened in Playwright/Chrome; every claim below was reproduced by clicking, typing and pressing keys (state = `index.html?nofirst=1&fast=1&<state>#<screen>`; `fast=1` only for demos of timers). I did not edit the mockup. Probe scripts are in `/tmp/rv2` (not committed). `check_words` ran to completion (104 states, max 60); `check_tips` and `check_flows` aborted in my environment (browser closed / launch error), `check_deadends` aborted in round 2, so I replaced them with my own probes (focusable-tooltip check on 10 states, Tab-order checks, the click paths listed below).

## Verdict

**Round 3 closes everything I called major in round 2.** The Stop-after-step-3 race is gone, the post-clock Rescue panel is gone, the gate is above the fold at 960x640, Quick setup respects mixed disks and the Hybrid hint states the trade-off, the failed-post screen is under budget when counted honestly, and the Boot manager has a real life cycle. The new variants (flash backup did not verify, agent did not answer, Keep confirm, Twin confirm) are well chosen. The default demo now exercises the twin case.

I found **0 blocking and 0 major** issues in what the owner will look at first; **2 moderate (minor+)** and **9 minor** items remain, none of which changes the safety story. I would now let the critics review it.

| Criterion | R1 | R2 | R3 |
|---|:-:|:-:|:-:|
| Text budget (as the owner feels it) | 4 | 4 | 5 |
| Safety of the gate | 4 | 4 | 5 |
| Failure and recovery | 2 | 4 | 5 |
| First-time use / wizard length | 3 | 4 | 5 |
| Keyboard | 2 | 4 | 5 |
| RAID / Hybrid discoverability | 3 | 4 | 4 |
| Health to action | 3 | 4 | 4 |
| GPUI fidelity | 3 | 4 | 4 |

## 1. Verification of the round-2 findings

| Id | Status | What I did and saw |
|---|---|---|
| **R2-1** Stop-after-step-3 race | **Fixed** | Real timing (no `fast`): *Erase and install*, *Cancel* at step 1, left the dialog open. After about 7 s the open dialog rewrote itself to "Too late to cancel. The first disk is being changed. The install can no longer be stopped from here." with *Stop install* disabled; the status bar read "Disks changed", consistent with the screen. The `cancel-too-late` state shows the same. |
| **R2-2** Rescue panel after the clock | **Fixed** | `fail=post&phase=waiting` (also Keep): "Waiting for DSM 0:00 The shell closed. The NAS is rebooting." No Rescue panel, no *Open rescue shell*, no *Export diagnostics*. `loader`: only *Reinstall DSM*; `lost`: *Reconnect* with "The clock keeps running on the NAS." |
| **R2-3** Gate below the fold | **Fixed at 960x640, still scrolls at Largest** | 960x640 plain, twin and Keep: the three checkboxes (y=300 to 396) and the phrase (y=441 to 471) are above the footer (y=536); the summary cards are visible too. At Largest (130 %) the third checkbox straddles the footer (503 to 524 vs 510) and the phrase is at y=581 (below the fold); Erase stays pinned. Acceptable for the largest text size; see R3-3. |
| **R2-4** Quick setup and mixed disks | **Fixed** | `?unit=ds1621#goal` *Quick setup*: "Wipe all 6 disks · Hybrid 1-disk · 19.85 TiB usable · survives 1 disk", "32 GiB x 3 copies · ds1621 · andreas", the line "Your disks differ in size. RAID 6 would give 7.15 TiB and survive 2 disks.", buttons *Cancel*, *Customize*, *Continue to Confirm*. Confirm then shows an After line "Debian 13 · Hybrid 1-disk · 19.85 TiB usable · system 32 GiB x 3 · ds1621". Hostname follows the unit (`system` shows `ds1621`). |
| **R2-5** Hybrid hint | **Fixed** | `?unit=ds1621#setup`: the Tag "Hybrid: +12.7 TiB" is at y=461 to 486, above the footer at both 1280x800 and 960x640; the Popover says "RAID 6: 7.15 TiB, survives 2 disks. Hybrid 1-disk: 19.85 TiB, survives 1 disk. More space, less fault tolerance." with *Use Hybrid 1-disk* and *Open Help*. |
| **R2-6** Word count residue | **Fixed** | The failed screens' subtitle is now "Step 11 of 12" (3 words, counted, not data). My audit: `failed-post` counted 58 and no prose in data (only the unit-header facts); `failed-post-keep` 57; `failed-pre` 46; `confirm-twin-*` 57 (no longer at the limit). `check_words`: 104 states, max 60. |
| **R2-7** Boot manager | **Fixed** (one stale panel) | No Stepper on the screen; the tag reads "Will change flash"; progress steps are "Boot flash backup, Agent added, Reboot into the agent, Agent answers"; *Cancel* is enabled at 0 % and disabled (with a sibling info button) at 25 %; the result panel offers *Return to DSM* (AlertDialog) and *Done*; after returning: "Back in DSM. Our entries remain on the flash." with *Recovery options*; `find-boot-agent` shows the unit with a "Boot agent installed" Tag. |
| R2-8 Open web UI label | Not re-checked in detail (round-3 hand-off still reads "Open web UI"); see R3-6. |
| R2-9 Stepper for non-install goals | **Fixed** for Boot manager (no stepper); Recovery has none. |
| R2-10 Tab order | Unchanged: Help, Settings, Menu first, then Untested, Twin, sdc, About backups, the controls. Acceptable. |
| R2-11 Enter on Erase | Unchanged by design (suppressed by a key handler); spec now states it as a deliberate override. |
| R2-12 rem scaling | Largest verified at 960x640: no horizontal scroll, header dropped on Confirm, Erase pinned. px-sized widgets still to be verified in the real app. |
| R2-13 default twin | **Fixed**: default Find has four units, with `Twin` Tags on both DS918+ rows, and the default Quick path ends on `ERASE DS918+ 3456`. |
| R2-14 update warning reason | Not re-checked in this round (still states "Updating is allowed" with the web-restart sentence). |

New behaviour verified: disabled controls are real disabled buttons that are not Tab stops (Erase has `aria-disabled`, `tabindex=-1`; the tab order on Confirm reaches "Why locked" instead), every `[data-tip]` element on ten states sits on a focusable control, twin gate (`ERASE DS918+` alone shows "Add serial last 4." and stays locked; the other twin's suffix `7710` shows "No match"; `3456` unlocks), Keep confirm ("volume1 · 6.2 TiB may still be lost", Preserved "volume1 · best effort", Destroyed "40 GiB DSM system", After "volume1 kept · system 8 GiB x 3").

## 2. Remaining issues, ranked

**Moderate**

- **R3-1. Quick setup chooses capacity over protection without asking.** `?unit=ds1621#goal` *Quick setup* defaults to **Hybrid 1-disk (survives 1 disk)**, where plain RAID 6 on the same six disks survives two. The dialog is honest about it, and *Customize* is one click away, but a default that lowers fault tolerance should be an explicit choice. Make the dialog show two radio rows ("Hybrid 1-disk, 19.85 TiB, survives 1 disk" and "RAID 6, 7.15 TiB, survives 2 disks") with the capacity one preselected only when the difference is large, and add "survives N" to the Confirm After line (today: "Hybrid 1-disk · 19.85 TiB usable", without the fault tolerance).
- **R3-2. Boot-manager result panel is stale after *Return to DSM*.** After "Back in DSM. Our entries remain on the flash." the panel still shows "Agent ready", "30 min watchdog" and `ssh boot@192.168.1.40`, which no longer apply. Collapse those three to a muted "Agent stopped" line when the unit returned.

**Minor**

- **R3-3. Largest text at 960x640.** The phrase field is below the fold (y=581 vs footer 510) and the unit header disappears on Confirm. Erase is pinned, so it works, but consider moving the phrase above the three checkboxes at Largest so the field that matters is visible, and keeping the one-line identity visible.
- **R3-4. Gate hint after a match.** "Add serial last 4." stays visible next to "Matches" (twin right suffix: both texts shown). Hide the hint once the phrase matches, and say what to add: "Add the last 4 of the serial: 3456" is already in the checkbox, so "Add serial last 4." could become "Add 3456".
- **R3-5. Tag text is cryptic.** The disk-warning Tag now reads only "sdc" (a warning icon plus a drive name) and the twin Tag reads "Twin". Both have popovers, but the visible label should carry the meaning: "sdc warning" and "Same model" (2 words each, within the limit).
- **R3-6. "Open web UI" still appears up to three times on a Critical unit** (header card, the failed disk row, the degraded array row) and only raises a toast. Name the hand-offs by intent ("Replace sdc", "Fix md1") with the external-link icon, and mention the certificate warning in the toast or popover (the fingerprint is on Done).
- **R3-7. Footer button is called "Command".** One word with a terminal icon is fine, but "Command" alone does not say what it does until hover; "Copy cmd" is worse. Keep the tooltip and the icon; consider "CLI" as a one-word alternative.
- **R3-8. Duplicated text on Confirm.** The Changed and Preserved cards repeat what the list rows below say ("Boot flash · backed up first", "Nothing"). Keep either the cards or the list at 960x640.
- **R3-9. Update dialog warning has no reason** ("An array is degraded or rebuilding. Updating is allowed."). Add "The rebuild keeps running." as agreed (spec section 11 and 12).
- **R3-10. Quick and Customize both go on to Confirm without showing the generated password policy.** The password is first visible on Done; say so in the Quick summary ("Password is generated; you copy it at the end") so it is not a surprise.
- **R3-11. Real-app risks, not prototype defects:** text scaling with px-sized widgets (R2-12), the Enter-on-Erase key interceptor (R2-11), and the dual Cancel/Why-unavailable pairs depend on the `Button` disabled state being reachable by the sibling: all feasible, but each needs a test in the first GPUI build.

## 3. UX-SPEC consistency (spec text fixed, mockup untouched)

I compared section 12 and the screen entries with the final mockup and corrected the spec in these places:

1. **One-line recovery text on Confirm**: the mockup says "Free to cancel until step 3; then no undo." (with an info icon opening the dialog); the spec said "Cancel is free until step 3; DSM may be gone after." Spec now matches the mockup and keeps "DSM may be gone" in the dialog.
2. **Disabled-reason pattern**: D7 (Erase), D8 (Cancel), D5, D13, D15 and rule 5b still described "styled disabled but focusable; activating opens a Popover". Replaced everywhere by: a real disabled `Button` (not a Tab stop) plus a sibling focusable info `Button` ("Why locked" / "Why unavailable") that opens the Popover.
3. **Twin**: the Alert and Tag are called `Twin`; the phrase suffix, the "Add serial last 4." hint and the "wrong suffix shows No match" behaviour are specified; the demo inventory always contains a twin pair.
4. **Command button**: labelled "Command" (terminal icon + one word) on Setup and Confirm; word counts updated.
5. **Stepper**: six steps (Find, Connect, Goal, Setup, Confirm, Install); Inspect is the second half of Connect, Done the end of Install; D5 and D6 are the Storage and System tabs of Setup; Boot manager and Recovery have no Stepper; shell words 12 (11 on Find). The earlier "open decision O1" is closed.
6. **D9 variants**: added flash-backup-did-not-verify (Back to Confirm), agent-did-not-answer (30:00 watchdog), waiting ("The shell closed. The NAS is rebooting."), loader and lost, with the rule that an open Cancel dialog re-evaluates when step 3 starts.
7. **Boot manager**: tag "Will change flash", the four progress steps, Cancel free during the read-only backup, "Back in DSM. Our entries remain on the flash.", and the "Boot agent installed" Tag in Find.
8. **Quick setup**: dialog buttons Cancel, Customize, Continue to Confirm; mixed-disk default and RAID 6 comparison; hostname follows the unit.

Items still open for the spec: R3-1 (Quick default choice and "survives N" on Confirm), R3-2, R3-4, R3-5.
