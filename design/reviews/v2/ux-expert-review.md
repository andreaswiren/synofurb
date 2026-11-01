# UX expert review: desktop prototype v2

Reviewed: `design/mockups-v2/desktop/index.html` (single file, opened in Playwright/Chrome and clicked through at 1280x800 and 960x640; the 118 screenshots were used only as a map), against `design/v2/UX-SPEC.md`, `design/BRIEF-v2.md` and the owner's complaints. Engineering truth: `docs/RECOVERY.md`, `docs/BOOT.md`, `docs/RAID.md`, `docs/COMPAT.md`.

Method notes: every finding gives the URL state (`index.html?nofirst=1&...#screen`) so it can be reproduced. Words were re-counted with my own Playwright script (same visible-text rule as `check_words.js`, but reporting what the `.d` "data" exclusion hides; not committed, the method is described in B5). Keyboard behaviour was driven with real `Tab`/`Enter`/`Space`/`Esc`/`F1` presses. I did not edit the prototype.

## Verdict

**The prototype is a big step forward and I would build from it**: the screens are scannable (the median screen has about 45 words), explanations really moved behind popovers, tooltips and dialogs, the structure (persistent unit header, Now | After plan, Health overview with a management-app update, Operation-plan dialog with typed phrase, Help sheet, About with logo) is right, the gate is terse and complete, and contrast and flow checks pass (54/54 flow checks; text contrast min 5.28:1).

It is **not ready to be called the review artifact**: 5 blocking findings (all of them either a false safety statement, a dead end in a destructive or crisis flow, or text-budget accounting that does not survive scrutiny), 12 major, and a list of minor ones. Counts: blocking 5, major 12, minor 9.

## Scores (1 to 5)

| Criterion | Score | Why |
|---|:-:|---|
| Text budget (as the owner will feel it) | 4 | Typical screens are light. Failure and Install screens hide prose under the `.d` data class (B5). |
| Safety of the data-loss gate | 4 | Terse, complete, identity in the persistent header, lowercase and trailing spaces rejected. One-line recovery statement is reassuring (M1); identity not next to the checkbox (M2). |
| Failure and recovery | 2 | Risk Tag wrong on Recovery/Cancel/Boot manager (B2); rescue shell is a toast (B3); replace-disk does not choose the disk (B4). |
| First-time use / wizard length | 3 | Nine steps; four "Next" clicks with no decision; no express path (M3). |
| Keyboard | 2 | 9 tab stops before the gate; focus drops to `body` on every navigation; tips on tags unreachable (M5). |
| RAID / Hybrid discoverability | 3 | Picker and Now|After are good; Auto never suggests Hybrid on mixed disks; the app can only replace a disk (M4). |
| Health to action | 3 | Replace is reachable; services-down and degraded md1 have no action; hierarchy puts "Update" above "Replace" (M8). |
| GPUI fidelity | 3 | Mostly buildable; several things are not (section 5). |

## 1. Blocking findings

**B1. Boot manager "Install agent" is a dead end and writes to the boot flash without a confirmation.**
State: `?unit=ds918&nofirst=1#boot`, press *Install agent*. Observed: no dialog, no progress, no result; the page does not change (button still "Install agent"), only the status-bar Tag flips from "Read-only" to "Flash changed". The user never learns that the agent is installed, how to connect (`ssh boot@...`), that it reboots to DSM after 30 minutes, or how to leave. The screen's own Tag says "Flash changes" while the status bar says "Read-only" before the click. A write to the boot flash is the only unrecoverable-by-power-cycle risk in `docs/RECOVERY.md` ("a damaged boot flash") and it has no gate.
Fix: confirm AlertDialog ("Install agent?" what is written, flash backup first, Install), then the D8 pattern (Boot flash accordion: backup, add agent, reboot, agent answers), then a result panel with the SSH command (Copy), the 30 minute timer, and *Return to DSM*. The spec now says this (D13).

**B2. The risk Tag and the "Way back" popover are wrong in six states (same defect class as D6 in the first review).**
- `#recovery` (after a failure): Tag "Read-only", Way back "Nothing to undo." The disks are modified and our entries are on the flash.
- `#install`, press *Cancel*, *Stop install* (state `?unit=ds918&gate=ready#confirm`, Erase, wait 2 s, Cancel): returns to `#inspect` with Tag "Read-only", although the Tag read "Flash changed" a moment earlier and our entries and the agent are on the flash; nothing says the NAS reboots to DSM in 60 s or that the entries remain.
- `#boot`: Tag "Read-only" under a screen whose own Tag says "Flash changes".
- `#manage` with `health=faildisk`, `unreachable`, `svcdown`: green "Debian running" while the overview says Critical, Unreachable or Needs attention.
- `#settings`, `#about`, Help: the status bar shows "Read-only" and "Way back" on pages that are not about a unit.
- Manage > Recovery tab leaves Manage for the separate Recovery screen (Tag "Read-only" again).
This is the product's central safety claim; it must be computed from facts (flash entries present, disks written, install running, unit reachable, worst health), never from the screen name.
Fix: in the spec (section 1.1) the Tag is now a pure function of the unit state; Recovery, Boot manager, Cancel and Manage use it; Settings/About/Help hide it.

**B3. After the point of no return the rescue shell is a transient toast, and the time-critical instruction is hidden.**
State: `?unit=ds918&fail=post#failed`, press *Open rescue shell*: a toast "ssh root@192.168.1.40 (mock)". There is no persistent, copyable command, no credential, and the 15 minute clock keeps running. The instruction that matters most ("export diagnostics first, the NAS is unreachable after the clock") exists only in the *What happens next* dialog. The countdown card also has an unexplained terminal icon next to the time (failed-pre shows a spinner there).
Fix: a persistent Rescue panel on the failed screen (mono `ssh root@ip`, Copy, credential via Clipboard, "Export first." as the only prose), toast only as confirmation of the copy. Specified in D9.

**B4. "Replace" (health overview and Storage tab) never lets the user choose or even see the new disk.**
State: `?unit=ds918&health=faildisk&nofirst=1#manage` > *Replace* on `sdc`: Operation plan dialog says "Add the new disk", asks to type `REPLACE sdc`, and offers *Run*. Which disk will be erased? Is it large enough? The typed phrase names the removed disk, not the erased one. This is a destructive action on an unchosen device.
Fix: the dialog needs a *New disk* Select (free disks only, size check, name shown) and the phrase should name both (`REPLACE sdc WITH sde`) or the dialog should show the chosen disk beside the phrase. Specified in D18/W16.

**B5. The text-budget exclusion is not honest in the failure and install screens.**
The checker excludes `.d` ("data") spans; for names, sizes, addresses and hashes that is fair. It is not fair for sentences. What I measured (counted words / words hidden in `.d` / visible total):

| State | Counted | In `.d` | Prose hiding in `.d` | Honest count |
|---|---|---|---|---|
| `fail=post#failed` | 47 | 33 | cause sentence "boot entry failed read-back; the original flash was restored and verified" (12) + "Step 11 of 12 · Configure packages, users, boot" (8) | **67 (> 60)** |
| `fail=pre#failed` | 44 | 32 | "disk sdc changed after you confirmed; the installer refused to continue" (10) + "Step 2 of 12 · Preflight: match disks" (7) | **61 (> 60)** |
| `step=12#install` | 46 | 86 | 12 step names (about 45 words, all visible, 8 rows on screen), "Install the Debian base system", "Step 9 of 12", "about 7 min left" | about 90 |
| `#confirm` | 55 | 57 | "Boot flash · backed up first", "Nothing on the disks" | about 59 (ok) |
| `health=ok#manage` | 45 | 55 | "DSM loader entry remains" | about 48 (ok) |

Honest totals including real data are 100 to 164 words on Plan, Install, Inspect and Manage (e.g. `plan-hybrid` 149, `install-log` 164); that is fine when it is a table of names and sizes, but the *rule* has to say so. The spec asks that the Install screen shows "current step open"; the prototype shows all 12 step names.
Fix: counting rule in the spec (section 0, rule 1c): data = entity names, sizes, addresses, hashes, log lines, times, engine step names inside a collapsed list; **any sentence or clause is prose**. Engine error causes are prose: put them in a tooltip/dialog and show a short `short` label ("Boot entry check failed"). Show the current step only; the other steps are an Accordion. Make `check_words.js` count `.d` spans that have more than 3 words or a verb.

## 2. Major findings

**M1. The one-line recovery summary on Confirm is reassuring and incomplete.** `#confirm`: "Fallback: DSM stays default." After step 3 DSM's own system may be gone (RECOVERY.md). The brief asks for a terse line, not a comforting one. Replace with "Cancel is free until step 3; DSM may be gone after." (10 words) and keep *How recovery works*. (Spec D7 updated; budget stays at 60.)

**M2. The identity is not next to the confirmation it protects.** `#confirm`: the checkbox says "This is the unit" and the IP/serial are in the persistent header 200 px above; at 960x640 header and checkbox are both visible but visually unrelated. The twin warning ("Same model nearby") only appears with `?twin=1`; the default inventory (3 units) has no twin, so the main screenshots never exercise the case that justifies the phrase rule. Put `{IP · …serial}` after the checkbox label, and keep a twin in the default demo data.

**M3. Wizard length / no express path.** `Find > Connect > Inspect > Goal > Plan > System > Confirm` is 7 screens before the gate. From Goal the defaults are valid, yet the user presses *Next* on Plan and on System with no decision to make. Proposed (now an open decision in the spec): merge Plan and System into one tabbed step "Setup" (Storage | System) and let Connect continue straight into Inspect with a Spinner; the stepper becomes 6 steps (Find, Connect, Goal, Setup, Confirm, Install). Keep an "Install with defaults" path: Goal > Confirm with the defaults summarised on Confirm (hostname, user, kernel, source) behind a collapsed *Options* accordion.

**M4. RAID: the app only replaces a disk; Hybrid is not discoverable from Auto.**
- `?unit=ds918&health=faildisk#manage` > Storage: only *Replace disk*, *Open web UI* and a disabled primary-style *Create array* (tooltip "Use the web UI to change arrays."). The owner asked to manage RAID in the app *or* the web UI; a disabled primary button is a dead end. Either implement the spec's *Actions* menu with the Operation plan dialog (at least Scrub, Add spare, Replace) or remove the button and make *Open web UI* the single call to action.
- `?unit=ds1621&nofirst=1#plan` (disks 1.82, 3.64 x3, 7.28 x2 TiB): *Data RAID* = Auto gives **RAID 5, 7.15 TiB usable**; Hybrid 1-disk on the same disks gives **19.85 TiB** (see `raid=hyb1`). Nothing on the screen tells the user. Add a one-line, dismissible suggestion when disk sizes differ by more than 10 %: Tag "Hybrid: +12.7 TiB" next to *Data RAID*, opening the Slices popover. (Spec D5.)
- Colour-only identity in the Now | After diagram (`raid=hyb1`: slices data1/data2/data3 in purple, blue, green with the legend only in the "After" info popover). Add a text key (chip names exist but are not tied to the colours).

**M5. Keyboard path.**
- `#confirm`: 9 Tab presses (Help, Settings, Menu, six stepper "Back to ..." buttons) before the first gate control; the stepper is six tab stops on every wizard screen. Make the stepper a landmark (not in the Tab order; `Alt+Left/Right` moves between completed steps).
- After every navigation (`Connect`, `Next`) focus falls to `body` (`document.activeElement` = BODY); the spec says focus goes to the title (or the error summary).
- Tags that carry a tooltip are not focusable: "Disk warning", "Untested", "Twin" on Confirm. The tip (the sdc detail) is therefore hover-only. Every tag with information must be a focusable trigger of a Popover, or its content must be repeated in a focusable control.
- `F1` with focus on *Data RAID* opens the topic list, not `raid-levels` (spec: "topic of the focused control").
- `Enter` on the focused *Erase and install* starts the install (spec said `Space` only). See also section 5: GPUI buttons take both keys; amend the spec rather than the prototype. `Enter` in the phrase field does nothing, which is correct.
- Phrase matching rejects a trailing space (`"ERASE DS918+ "`) with only "No match": trim leading and trailing whitespace (still case sensitive).

**M6. Disabled-with-reason is not keyboard-reachable in GPUI.** `gpui-component` 0.8.1 `tooltip.rs` has no focus path (hover only) and a disabled `Button` is not focusable. The prototype shows the Erase tooltip on hover ("Missing: backup tested, ...") and the spec relied on focus. Spec change: the Erase button stays focusable and styled disabled; *activating* it opens a Popover "Missing: ..." (no action); the same for Finish, Next on blocking errors.

**M7. Text size is not wired.** `#settings` > Text size > *Largest*: nothing changes (screenshot `/tmp/rv/34-settings-largest.png`). With a 130 % root font size at 960x640 (`#confirm`) the gate still fits but the stepper numbers crowd; "scalable text" is a brief requirement and cannot be judged from this prototype.

**M8. Health overview: hierarchy and actions.**
- `health=faildisk`: the primary (violet, solid) button is *Update* (management app); *Replace* is a small outlined button inside the Disks card. On a Critical unit the primary action must be the repair; the update must be secondary while health is not OK.
- `health=svcdown` (`synofurb-mgmt` and `mdadm` red): no *Restart* action; the only buttons are Update and Open web UI, and *Update* is offered while the management service itself is down (the dialog says the web UI restarts).
- Arrays: degraded `md1` / `md0` have no action; only the disk has *Replace*. `md1` degraded should offer *Replace disk* too.
- Duplicate identity: the page title "DS918+" repeats the unit header directly above it; "Running Debian 13." repeats the Tag; the header still says "DSM 7.2.2" for a unit that runs Debian.
- `health=unreachable`: the status bar still says "Debian running" in green (B2).
- The management-app update flow itself is good: version on unit vs in app, a four-phase progress (Upload, Install, Restart, Health check), "restored automatically" in the dialog, failed and updated states. Missing: block *Update* while an array is rebuilding or degraded (or warn).

**M9. Recovery screen.** `#recovery`: five rows all with the same *Run*; "Remove our entries" and "Restore flash" are not visually different from "Boot DSM next". *Remove our entries* ran without confirmation in my test (`Boot DSM next`, `Open agent`, `Remove our entries` showed no dialog); when Debian is the default boot entry this removes the only path to boot Debian. The row must (a) be disabled with a reason when our entries are the active boot path, (b) require confirmation otherwise. *Reinstall DSM* is a "Run" that only opens an explanation dialog; label it *Open guide*. Manage > Recovery tab leaves Manage (no tab bar, no unit title), and *Back* returns to a different place than the tab you left.

**M10. Install: cancel before step 3 gives no outcome.** See B2; additionally the Cancel dialog says "The NAS returns to DSM within 60 seconds." but the screen after *Stop install* is Inspect with no countdown, no "waiting for DSM", no cleanup offer for the boot-flash entries.

**M11. Boot manager copy.** "Other OS: power cycle returns." is unreadable (what OS? what returns?). Use "After another OS starts, only a power cycle returns." and show the 30 minute watchdog as a Tag on the result panel.

**M12. At 960x640 the safety summary falls below the fold.** `?unit=ds918#plan` at 960x640: *Destroyed / Changed / Preserved* and the *Disk warning* Alert are below the footer; `?gate=ready#confirm` at 960x640: only *Destroyed* is visible, *Changed* and *Preserved* need scrolling while the gate card is fully visible, so the gate can be completed without having seen them. Collapse the Destroyed list to one row per group ("sda sdb sdc sdd · 4 x WDC WD40EFRX · 14.55 TiB", "volume1 · btrfs · 6.2 TiB used", "DSM · settings · users") so all three groups fit at 960x640.

## 3. Minor findings

- m1. The `find` screenshots preselect DS918+ (`unit=ds918`); the real first-run state (nothing selected, *Connect* disabled) is not in the screenshot set. Add a `find-none` capture.
- m2. `#inspect`: "Untested" appears twice (header and Model row) and "Inspection" repeats the header's unit; drop the Model row Tag.
- m3. `#find` Rescan/Add by IP sit left of the filter in the same row; the primary *Connect* is far away in the footer (acceptable) but the row selection is only a tint: add a visible radio/check mark for colour-independence.
- m4. `#done`: the title row has "Verified over SSH" as a status Tag; Next steps (collapsed) correctly carries the `sdc` reminder, good. The password dots are 14, the real password is longer; irrelevant but do not leak length: always show 12 dots.
- m5. `#about`: *Close* leads where? There is no unit context; also shows "Read-only / Way back". (B2.)
- m6. Stepper at 960 shows numbers only except the current one; labels hidden means discoverability suffers on a first run: keep labels for the next step too.
- m7. The `mockup` mark overlaps the title bar centre; fine for a prototype, but any review screenshot should be taken with `bare=1` consistently (some files include it).
- m8. `#install` footer left holds an icon-only button (open log file) with no label or tooltip until hover; keep an `aria`-style visible text on first appearance or move it into the Log accordion header.
- m9. The hidden dev menu (`Ctrl+Shift+D`) and URL parameters are good for review; document them in the README (they are) and keep out of screenshots.

## 4. What I verified and found good

- **First launch**: Dialog "Before you start" (34 words) with read-only, backup, screen-reader and CLI statements; dismiss with `Esc`.
- **Connect**: host-key dialog on first contact, `Trust`/`Cancel`; Connect has a loading state; nothing preselected (the default state; see m1).
- **Gate** (`#confirm`): three checkboxes, phrase field with live "Matches" Tag, lowercase and wrong phrase rejected, `Enter` in the field does nothing, Erase disabled with a tooltip naming what is missing (hover), Copy command included. The phrase chip is shown above the field and paste is allowed.
- **Plan**: Now | After with a text key for the disks; Hybrid slices dialog; *Usable* / *Survives* live; Keep mode behind *Advanced* with a blocking error and a *Set to 9 GiB* fix.
- **Install**: collapsed Boot flash and Log accordions, a single progress bar with the current step, the "No return" Tag, Cancel disabled after step 3 with an explanation.
- **Failed pre / post**: honest Disks and Boot flash chips, countdowns with the right durations (60 s, 15 min), distinct variants, primary action pinned in the footer.
- **Manage**: persistent unit header, Overview health with reason chips, Disks/Arrays/Volumes/Services/Updates/Boot default cards, the management-app update with auto-rollback copy, Operation plan dialog with the commands collapsed and the typed phrase.
- **Help and About**: Sheet with search, topics and shortcuts; About with the recycle-style logo, version, *Copy diagnostics*, links and credits.
- Contrast and flow checks both pass; scroll bars themed; no horizontal scroll at 960x640 in the states I opened.

## 5. What the prototype does that GPUI cannot (or does not do by default)

1. **Tooltip on keyboard focus.** `gpui-component` 0.8.1 `Tooltip` is hover driven (no focus handling in `tooltip.rs`). Anything needed to act must be on a focusable info button that opens a `Popover`; tooltips are an enhancement only (spec rule changed).
2. **Disabled buttons that show a reason and can be focused.** A disabled `Button` is neither focusable nor reliably tooltip-able; use a styled-disabled but enabled `Button` whose click opens a Popover (spec changed).
3. **WOFF fonts.** The prototype embeds `.woff`; GPUI's text system loads TTF/OTF. Bundle Inter and JetBrains Mono as TTF/OTF (OFL).
4. **CSS scrollbar styling** (`::-webkit-scrollbar` with a 16 px hit area, thumb that grows on hover): GPUI uses its own overlay scrollbar (`.overflow_y_scrollbar()` with the theme tokens `scrollbar`, `scrollbar_thumb`, `scrollbar_thumb_hover`); the width/hover behaviour must be taken from the widget, not copied from the CSS.
5. **Native `<select>`**: becomes `Select`/`Combobox`, a popup list; the prototype's "closed list not counted" rule still holds.
6. **Window controls drawn by the page** (Windows style): use `TitleBar`; on macOS the traffic lights are on the left and the logo moves.
7. **Hand-drawn `div` widgets**: the Now | After diagram and the countdown are the two listed exceptions; the Tag pills with leading icons, Stepper connector lines and GroupBox coloured left rules need `Tag`+`Icon` children, `Stepper`/`StepperItem` styling and a `.border_l_*` override respectively; none is a blocker.
8. `Alert::banner()` exists, so the update banner is a real widget (the spec listed it as an exception; corrected).
9. **Dev menu, mockup mark, URL parameters, `data-tip` attributes**: prototype scaffolding only; the `--demo --screen <name>` option replaces them.

## 6. Spec changes made as a result of this review

See `design/v2/UX-SPEC.md` ("Revision after the prototype review" at the end and the edited screen entries): persistent unit header; fact-derived risk Tag including Recovery, Boot manager, Cancel, Manage health; Plan *Now | After*; Manage *Overview / Health* with management-app update and its states; Operation plan dialog with New disk; counting rule for prose-in-data; stepper not a tab stop and focus-to-title; tooltip and disabled-button rules for GPUI; Confirm one-line recovery text and identity next to the checkbox; Boot manager confirm + progress + result; persistent rescue panel; Alert::banner; font formats; "Setup" merge proposal; Hybrid suggestion; Collapsed Destroyed list for 960x640.

## 7. Open questions for the UI expert and product owner

1. Merge Plan and System into one "Setup" step (6 steps total)? Recommended; not yet specified screen by screen.
2. Should desktop RAID operations beyond Replace/Scrub stay web-only for v1? If yes, the Storage tab needs one primary call to action ("Open web UI"), not a disabled *Create array*.
3. Is it acceptable to block *Update* (management app) while an array is degraded or rebuilding? I recommend a warning, not a block.
4. Can the app reattach to a running install and a running rescue-shell countdown after a restart? B3 assumes the countdown is the app's own timer of a deadline the NAS reports.
