# UX expert review, round 2: desktop prototype v2 (revised)

Reviewed: `design/mockups-v2/desktop/index.html` (424 KB, single file, `src/js1..7`, `screens.json` with 92 states) opened in Playwright/Chrome, clicked, keyboard-driven, and compared with `design/v2/UX-SPEC.md`, `design/BRIEF-v2.md` and `docs/RECOVERY.md`, `docs/BOOT.md`, `docs/RAID.md`. Screenshots in `design/screenshots/` were used as a map only; every finding below was reproduced by clicking (URL = `index.html?nofirst=1&fast=1&<state>#<screen>`). I did not edit the mockup. My probe scripts are in `/tmp/rv2` (not committed).

## Verdict

**The author's claims hold up.** All five round-1 blocking findings are fixed or superseded, 11 of 12 majors are fixed (one partially), and the new decisions (6-step wizard with Setup merge, Quick setup, Connect continuing into Inspect, read-only Storage with a web-UI hand-off, Rescue panel, Boot manager confirm/progress/result, update-with-warning) are good designs that I would keep. `check_words` (90 states, max 60) and my independent audit agree; focus now lands on the title; the stepper is out of the Tab order; the risk Tag is derived from unit facts in every state I tried.

It is **not yet at "zero blocking" for the owner's safety bar**, because of **0 blocking, 6 major and 8 minor remaining findings**, three of which are state-machine contradictions in the crisis screens (R2-1, R2-2) or the gate (R2-3). I rate none "blocking" because each needs a specific timing or state to appear, but R2-1 and R2-2 would be blocking in the real app if implemented as drawn.

| Criterion | Round 1 | Round 2 |
|---|:-:|:-:|
| Text budget (as the owner feels it) | 4 | 4 (residual: R2-6) |
| Safety of the gate | 4 | 4 (gate below the fold at 960x640: R2-3) |
| Failure and recovery | 2 | 4 (two contradictions left: R2-1, R2-2) |
| First-time use / wizard length | 3 | 4 (Quick setup good; R2-4) |
| Keyboard | 2 | 4 |
| RAID / Hybrid discoverability | 3 | 4 (R2-5) |
| Health to action | 3 | 4 |
| GPUI fidelity | 3 | 4 |

## 1. Status of every round-1 finding

| Id | Round-1 finding | Status | Evidence (state, what I did) |
|---|---|---|---|
| **B1** | Boot manager install was a dead end, no confirm | **Fixed** | `?unit=ds918#boot`: *Install agent* opens "Install boot agent?" (backup first, DSM stays default, power-cycle if the agent does not answer); *Install* shows a Boot flash progress (75 % step list); result panel "Agent ready", `ssh boot@192.168.1.40`, "30 min watchdog", *Copy command*, *Return to DSM* (AlertDialog), *Done*. Status bar flips to "Flash changed". Residuals in R2-7. |
| **B2** | Risk Tag/Way back wrong in six states | **Fixed**, one regression (R2-1) | Recovery shows "Disks changed" (flash,disks) and "Debian running" (flash,disks,debian), "Flash changed" (flash only); stopped install shows "Flash changed"; Manage shows Critical / Needs attention / Unreachable / Debian running; Settings and About show no Tag and no Way back; Manage > Recovery tab stays inside Manage. |
| **B3** | Rescue shell was a toast; "export first" hidden | **Fixed for the running clock**, regression after the clock (R2-2) | `?fail=post#failed`: persistent Rescue panel with mono `ssh root@192.168.1.40` + Copy, masked password with Show and Copy, *Rescue notes*, "Export first.", 15:00 clock, *Open rescue shell* copies the command and confirms with a toast. Clock expiry states exist (`phase=waiting|loader|lost`). |
| **B4** | Replace-disk did not choose the new disk | **Superseded** (orchestrator decision), verified | Storage tab is read-only: array table (members, state, size), disk table, "Array degraded. Replace the disk in the web UI.", *Open web UI*, a "Why the web UI?" info button. Overview shows *Open web UI* on a failed disk and on a degraded array. No replace dialog remains on the desktop. See R2-8 for the hand-off label. |
| **B5** | Prose hiding in "data" spans | **Mostly fixed** | `check_words.js` now counts prose in data spans; the engine cause became the label "Boot entry check failed" with the full sentence in a Popover; Install shows the current step only. Residual: the subtitle "Step 11 of 12 · Configure packages, users, boot" (8 words) is still a `.d` span: failed-post honest total is 65 (> 60). See R2-6. |
| **M1** | Confirm one-liner reassuring | **Fixed** | `#confirm`: "Cancel is free until step 3; DSM may be gone after." |
| **M2** | Identity not next to the checkbox; no twin | **Fixed** (twin only in dedicated states) | Checkbox reads "This is 192.168.1.40 · …3456"; twin states add a "Same model nearby" Alert and a **phrase suffix** `ERASE DS918+ 3456` (a new, good idea; fold into the spec). The default inventory still has no twin (minor R2-m1). |
| **M3** | Wizard length, no express path | **Fixed** | Six steps (Find, Connect, Goal, Setup, Confirm, Install); Connect continues into Inspect after the host-key Trust; Setup has *Storage / System* tabs; Goal's Install card has *Quick setup* (summary Dialog "Continue to Confirm") and *Customize*. See R2-4 for gaps. |
| **M4** | RAID discoverability | **Partly fixed** | Hybrid suggestion Tag + Popover with a *Use Hybrid 1-disk* action; "Colour key" button beside the diagram; Storage tab is read-only by decision. Gaps: R2-5. |
| **M5** | Keyboard | **Fixed** | Tab order on Confirm: Help, Settings, Menu, Untested, Warning sdc, About backups, 3 checkboxes, phrase, How recovery works, Back, Copy command, Why locked, Erase, Way back (stepper no longer a tab stop; Tags are buttons). Focus lands on the `h1` after Connect. `F1` on *Data RAID* opens "RAID levels"; on *Warning sdc* opens "Recovery". Trailing and leading spaces are accepted, lowercase is rejected. |
| **M6** | Disabled + reason | **Fixed** | A disabled button has an adjacent focusable info button ("Why locked", "Why unavailable") that opens a Popover ("Missing: tested backup, data loss accepted, unit confirmed, the phrase."). This is simpler and more GPUI-native than my own proposal; adopt it. |
| **M7** | Text size not wired | **Fixed** | Settings > *Largest* = root 20.8 px (130 %); `size=largest` at 960x640 on Confirm: no horizontal scroll, Erase still pinned (but see R2-3). Implemented in `rem`. |
| **M8** | Health hierarchy and actions | **Fixed** | `health=faildisk`: *Open web UI* is primary, *Update* is outlined; `svcdown`: *Restart* per failed service; `unreachable`: Update and Open web UI disabled with "Why unavailable", *Retry* and *Scan again*; page title is "Manage"; the header shows "Debian 13 · 6.12.57" instead of DSM. |
| **M9** | Recovery rows | **Fixed** | Each row: label, one line, one button, an info icon; *Boot DSM* and *Open agent* are AlertDialogs; *Restore flash* needs `RESTORE FLASH`; *Remove entries* needs `REMOVE ENTRIES` and is disabled with a reason when Debian is the default (`facts=flash,disks,debian`); *Reinstall DSM* is *Open guide*; Recovery also opens as a Manage tab. |
| **M10** | No outcome after Stop install | **Fixed**, race found (R2-1) | Stop leads to "Install stopped": *Disks untouched*, *Flash changed*, "Stopped by you.", "DSM returns in 0:57", *Wait for DSM*. |
| **M11** | Boot manager alert copy | **Fixed** | "After another OS starts, only a power cycle returns." |
| **M12** | 960x640 fold | **Partly fixed** (R2-3) | The three summary cards are now visible at 960x640, but the gate (third checkbox, phrase) is below the fold. |
| m1 | `find` screenshot preselected | Fixed (`find-none`, default has no selection) | |
| m2 | Untested twice on Inspect | Fixed (no Tag in the Model row) | |
| m3 | Row selection colour-only | Not re-verified | |
| m4 | Password dot count | Fixed (12) | |
| m5 | About showed a unit Tag | Fixed | |
| m6 | Stepper labels hidden | Fixed (six labelled steps at 960) | |
| m7 | Mockup mark in screenshots | Unchanged (acceptable) | |
| m8 | Icon-only log button | Fixed (log moved into the Log accordion header) | |
| m9 | Dev menu documented | Fixed (README) | |

## 2. Remaining issues, ranked

### Major

**R2-1. Cancel race: Stop install is honoured after the point of no return, and the result contradicts itself.**
State: `?unit=ds918&gate=ready#confirm`, *Erase and install*, at step 1 or 2 press *Cancel*; leave the dialog open for a few seconds (with `fast=1` about 1 s; in real life steps 1 and 2 last about 30 s, so reading the dialog is enough); press *Stop install* after the installer has entered step 3. Observed: the page "Install stopped, Step 3 of 12 · Release RAID/LVM, wipe reclaimed areas" shows Tags **Disks untouched** and **Flash changed**, while the status bar shows **Disks changed**. The dialog text was "Before step 3 nothing on the disks has changed", which stopped being true while it was open.
Fix: when step 3 starts, close or rewrite the open Cancel dialog ("Too late: the first disk is being changed") and disable *Stop install*; derive both the Tags and the status bar from the same fact (disks written yes/no).

**R2-2. After the clock runs out, the Rescue panel and *Open rescue shell* are still shown.**
State: `?unit=ds918&fail=post&phase=waiting#failed`: the card says "Waiting for DSM 0:00" with a spinner, yet the Rescue panel (`ssh root@192.168.1.40`, password, *Rescue notes*), "Export first." and the primary *Open rescue shell* and *Export diagnostics* remain. After the 15 minute deadline the shell is closed and the NAS is not reachable; offering them is the same kind of false statement as round 1's B3 in reverse. (`phase=loader` is correct: only *Reinstall DSM*; `phase=lost` is correct: *Reconnect*.)
Fix: in `waiting`, replace the Rescue panel with one line ("The shell closed. The NAS is rebooting into the DSM loader.") and make *Wait for DSM* the footer primary; remove *Export diagnostics* unless the export was already made.

**R2-3. The gate falls below the fold at 960x640 and at Largest.**
State: `?unit=ds918#confirm` at 960x640: the scroll area is 370 px high; the three confirmation checkboxes start at the fold (the third and the phrase input are at y=623 against a footer at y=536); twin state: input at y=681. At Largest (130 %) the content region shows about 200 px (chrome is about 310 of 640 px). The summary cards are above the gate, so a user can read the lists but must scroll to find what to do. Two Alerts (No backup found, Same model nearby) sit above the cards.
Fix: make the gate card sticky at the top of the right column; show the per-disk list collapsed ("sda sdb sdc sdd · 4 x WDC WD40EFRX" as one row, "Details" Accordion for serials); merge the two Alerts into one line; at heights below 700 px collapse the unit header to one line and the stepper to "Step 5 of 6: Confirm".

**R2-4. Quick setup users never see the layout again, and the "recommended" defaults ignore mixed disks.**
State: `?unit=ds1621&dlg=quick#goal`: "Wipe all 6 disks · RAID 6 · 7.15 TiB usable" for disks of 1.82, 3.64 x3 and 7.28 x2 TiB, while Hybrid 1-disk gives 19.85 TiB; the dialog shows "ds918" as the hostname of a DS1621+ (data bug). *Continue to Confirm* lands on a Confirm that shows destroyed, changed and preserved, but **not** the chosen layout (RAID level, usable capacity, system size), so a Quick user never sees what they will get after the dialog closes.
Fix: (a) when disks differ in size by more than 10 %, the Quick setup dialog names the choice and offers *Customize*; (b) add one line "After: RAID 6 · 7.15 TiB · system 32 GiB x3" to Confirm for every path; (c) fix the hostname.

**R2-5. Hybrid suggestion hides the trade-off and sits below the fold.**
State: `?unit=ds1621&nofirst=1#setup` (Storage tab, 1280x800): *Hybrid: +12.7 TiB* is under the Data RAID control, below the visible area. Its Popover says "RAID 6: 7.15 TiB usable. Hybrid 1-disk: 19.85 TiB on these disks; it survives 1 disk." It does not say that RAID 6 survives **two** disks, i.e. the user is trading fault tolerance for capacity.
Fix: move the Tag directly beside *Data RAID*; Popover text "RAID 6: 7.15 TiB, survives 2 disks. Hybrid 1-disk: 19.85 TiB, survives 1 disk." The Quick setup default should keep RAID 6 only if the user has not asked for capacity (state the trade-off there too).

**R2-6. Text-budget residue.**
Honest counts (my audit, counted + prose-like data): `failed-post` 57 + 8 (subtitle "Step 11 of 12 · Configure packages, users, boot") = **65**; `failed-post-keep` the same; `confirm-twin-a/b` exactly 60 with nothing in reserve; the Changed and Preserved cards repeat the same information as the list rows below them ("Boot flash · backed up first" twice, "Nothing" and "Nothing on the disks"). Install before the point of no return shows all 12 step names (121 words incl. data; acceptable only as a collapsed list).
Fix: subtitles are never data (spec rule): "Step 11 of 12" alone (3 words) as the subtitle, the engine step name in the Log; remove the duplicated card/row text; extend the checker to fail on any `.t-sub` that contains a `.d`.

### Minor

- **R2-7. Boot manager rough edges.** (a) `boot=progress`: the step list contains "Send installer", which belongs to the install path, not the agent. (b) *Cancel* is disabled for the whole progress even during the read-only flash backup, which is the one moment cancel is free. (c) After *Return to DSM* the panel stays on "Agent ready" with a "rebooting" line forever: add a "Back in DSM; our entries remain on the flash" result with *Remove entries*. (d) After *Done* the unit appears nowhere with its new state: `Find` lists it as an ordinary "Untested" DS918+ and the only way back to Recovery is through Goal; add a "Boot agent installed" Tag in Find. (e) The page Tag "Flash changes" under a status bar "Read-only" is still ambiguous before the click: rename to "Will change flash".
- **R2-8. "Open web UI" appears up to three times with the same label** (`health=faildisk`: header card, on the failed disk, on the degraded array) and only shows a toast ("Opens https://192.168.1.40:8443/raid in your browser"). Label the repair hand-offs by intent ("Replace in web UI" has 4 words; use "Replace sdc" with the external-link icon), and mention the self-signed certificate and fingerprint (Done shows it) in the hand-off popover.
- **R2-9. Stepper for non-install goals.** Boot manager and Recovery start from Goal but keep the install stepper (steps 4 to 6 greyed). Hide the Stepper or replace it with a two-step "Boot manager: Configure, Result".
- **R2-10. Tab order starts in the title bar** (Help, Settings, Menu) on every screen, before the content. Acceptable; add `F6` and a "skip to content" first stop.
- **R2-11. `Enter` on Erase is suppressed by a key handler.** `gpui-component` `Button` activates on Enter and Space by default (its own tests assert both); the prototype's behaviour needs an `on_key_down` interceptor in the real app. Feasible, but note it in the spec as a deliberate override.
- **R2-12. Text scale is rem-based in the prototype; in GPUI many widget sizes are `px`** (button heights, icon sizes). Verify the Largest setting on the real app; budget one iteration.
- **R2-13. The default demo inventory has no twin** (`find-twin` is a separate state), so the main screenshots never show the case the phrase suffix exists for.
- **R2-14. The update dialog's warning is thin** ("An array is degraded or rebuilding. Updating is allowed.") with no reason. See section 3.
- *Process:* `check_deadends.js` failed to run to completion in my environment (a navigation timeout on `keep=1&sys=16#plan`); `check_flows` reported 54 passed, `check_contrast` min 5.28:1 (round 1), and `check_scale` passed (type scale, weights and radii, 91 states). I did not rely on them.

## 3. Judgement on the new decisions

- **Setup merge (6 steps), Connect continues into Inspect:** correct. The merged step reads well, the Stepper at 960x640 keeps all labels, and Back/Next behave. Keep. Only the Boot-manager/Recovery stepper (R2-9) is wrong.
- **Quick setup:** a good express path, with the right safeguard (the dialog's safe default button is Cancel; the Confirm gate is unchanged). It must show the layout on Confirm and respect mixed disks (R2-4).
- **Read-only Storage with an Open-web-UI hand-off:** I endorse it. It removes the dead "Create array" and the unchosen-disk problem, and the tab is honest ("Changes run in the web UI"). Make the hand-off labels intent-specific (R2-8) and let the web UI deep-link to the failed array.
- **Management-app update, warn but allow on degraded arrays:** acceptable, because the update restarts only the web service (about 10 s) and the md rebuild runs in the kernel. The dialog must say that ("The rebuild keeps running.") instead of only "Updating is allowed", must stay disabled when the unit is unreachable (it is), and the rolled-back state ("Health check timed out. Rolled back to 0.9.1.") is clear. Do not allow it when the service is already `synofurb-mgmt`-down without saying that the update will start it.
- **Rescue panel:** the right pattern (persistent, copyable, credential via Clipboard, "Export first." as the only prose). Fix the post-clock state (R2-2). Consider making *Export diagnostics* the primary until it has been done once, because it is the only time-critical action.
- **Boot manager confirm / progress / result:** correct structure and copy; fix R2-7.

## 4. What I verified and found good (new since round 1)

- Quick setup dialog (defaults for Storage, System, Boot; "Confirm shows what is erased before anything changes."), two-button Install card.
- Confirm: one Alert on top ("No backup found. 6.2 TiB on volume1 is erased." with a Popover), three summary cards, gate card with identity line, checkbox identity, Type chip, "Matches" Tag, one-line recovery summary, *How recovery works*, *Copy command*, *Why locked* + disabled *Erase*.
- Twin states: "Same model nearby", `Twin` Tag in the unit header, phrase `ERASE DS918+ 3456` and "Matches" only for the suffixed text.
- Install: single progress with the current step, Cancel disabled after step 3 with "Why unavailable", "Connection lost. The NAS keeps installing." with *Reconnect*.
- Failure matrix: stopped by you, failed before step 3 (waiting, back, not back), after step 3 (running, waiting, loader, lost, Keep variant, notes), each with the correct Disks and Boot flash chips.
- Manage: the Health card (worst component + reason chips), management-app card with version on unit vs in this app, four-phase update, Rolled back / Updated states, Storage tab (members listed), Logins, History, Recovery in the TabBar.
- Help and About unchanged in quality; F1 is contextual now.

## 5. Fold into `design/v2/UX-SPEC.md` (also appended there as section 12)

1. **Twin phrase suffix**: when a same-model unit is on the network the typed phrase is `ERASE <MODEL> <last 4 of serial>` (here `ERASE DS918+ 3456`), the gate header and the checkbox show IP and the same four characters, and a "Same model nearby" Alert names the other unit. Without a twin the phrase is `ERASE <MODEL>`.
2. **Recovery rows**: each row is *label, one line, one action*, with an info icon; actions: Boot DSM (AlertDialog), Open agent (AlertDialog), Restore flash (typed `RESTORE FLASH`), Remove entries (typed `REMOVE ENTRIES`, disabled with a reason when Debian is the default boot entry), Reinstall DSM becomes *Open guide*. Recovery is also reachable as a tab of Manage (the TabBar stays) and from Goal; the status Tag reflects the state read from the unit.
3. **Stopped and post-clock screens**: D9 has these variants: *stopped by you* (Disks untouched, Flash changed, "DSM returns in 1:00", primary *Wait for DSM*), *failed before step 3* (waiting, back, not back), *failed after step 3* running (Rescue panel, clock), *waiting* (clock 0: Rescue panel and Export removed, *Wait for DSM*), *loader* (only *Reinstall DSM*), *lost* (*Reconnect*, "The clock keeps running on the NAS"), plus the Keep variant. Rule: a control whose resource is gone must disappear or be disabled with a reason; the open Cancel dialog must re-evaluate when step 3 starts.
4. **Disabled-reason pattern (replaces my styled-disabled-but-focusable rule)**: a real disabled `Button` plus a sibling focusable info `Button` ("Why locked" / "Why unavailable") that opens a Popover with the reason. Use it on Erase, Finish, Cancel after step 3, Update/Open web UI when unreachable, disabled Recovery rows. It is GPUI-native (disabled Buttons are not focusable, the sibling is).
5. **Wizard**: six steps (Find, Connect, Goal, Setup, Confirm, Install); Inspect is the second half of Connect (opens automatically after the host-key Trust); Setup = TabBar *Storage | System*; Goal's Install card = *Quick setup* (summary Dialog, safe default Cancel) and *Customize*; Confirm always shows an "After" line (RAID level, usable, system size).
6. **Manage**: Overview = Health card + management-app card (version on unit vs in this app, Update primary only when Healthy, four phases, Rolled back/Updated) + six cards; Storage = read-only arrays and disks with *Open web UI* and "Why the web UI?"; updating while an array is degraded or rebuilding is allowed with the sentence "The rebuild keeps running."; hand-off buttons are named by intent.
7. **Boot manager**: confirm AlertDialog, progress (backup, add agent, reboot into agent, agent answers; no installer step), result panel (SSH command with Copy, 30 min watchdog, *Return to DSM* AlertDialog, *Done*), Cancel enabled during the read-only backup; the unit carries a "Boot agent installed" Tag in Find.
8. **Diagram**: a *Colour key* button names every array chip; the Hybrid suggestion beside Data RAID states both capacity and fault tolerance of each option.
9. **Enter on Erase** is suppressed by a key handler (GPUI Buttons activate on Enter and Space by default).
10. **Text size** is `rem`-based; verify px-sized widgets in the real app.
