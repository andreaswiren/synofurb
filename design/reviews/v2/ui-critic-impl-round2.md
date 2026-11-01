# UI critic, implementation fidelity review, round 2

Scope: re-check of my round-1 findings (`ui-critic-impl-round1.md`, B1 to B5 plus the non-blocking list) against the latest push (`crates/synofurb/src/ui/**` after GUI fix rounds 1 and 2, commit "GUI fix round 2"), the 55 regenerated `design/screenshots/v2-app-*.png`, `design/v2/DESIGN-SYSTEM-V2.md` section 13 (now DEV-UI-1 to DEV-UI-19) and `docs/GUI.md`. No gate verdict; nothing written to `design/approvals/`; no code edited. I read the sources, looked at 33 of the 55 screenshots, and measured pixels for every contrast number below. I did not build or run the app.

## Verdict in one paragraph

Round 1's five blockers are all fixed, and the fixes are visibly good (outline tags, bordered segmented controls with an accent fill, selected-row radio and border, disabled-reason buttons, the list of erased disks, Lucide icons, a failed step in the Stepper, a 12-segment install strip, a proper two-tone logotype, four new dialogs that mostly match the mockup). **One blocking finding remains and it is new evidence, not new code: at Largest text the Find table scrolls horizontally and clips the Status column** (`v2-app-inventory-light-largest.png`), which breaks the brief's "no horizontal scrolling" rule. Three majors, twelve minors, and a clear list of evidence the real gate still lacks (below).

Counts: **blocking 1, major 3, minor 12.**

## Round-1 blockers: status

| # | Round-1 finding | Status | Evidence |
|---|---|---|---|
| B1 | Selected inventory row shown by tint only | **Fixed** | `screens/inventory.rs:111-122`: a `Radio` in the first cell plus a 1 px accent border. Pixels (`v2-app-inventory-light.png`, x=700): border `#B193E5` above and below the `#F1EDFE` row (2.24:1 against the row, 2.57:1 against the surface), checked radio filled `#7139CB` (6.67:1). Dark: border `#6A50A5`, row `#31254D`. Selection is no longer colour alone. |
| B2 | Segmented choices invisible, selected 1.06:1 | **Fixed** | `kit.rs:135-155`: `ToggleGroup` outline, container border `border_strong`, selected = accent fill with bold label. Pixels (`settings-light`): container border `#808080` (3.95:1), selected fill `#7139CB` with white label (6.67:1); dark selected `#A37BFE` with `#0E0C15` label 6.30:1, unselected labels 18.97:1. |
| B3 | Disabled buttons with tooltip-only reasons | **Fixed, one leftover** | `kit::with_reason` now wraps Done Finish (`done.rs:206`), Install Cancel (`install.rs:188-196`), Settings bundle download (`settings.rs:213`), Update Restart and Download (`update.rs:122,167`), Connect (`connect.rs:118`, reason when the host is empty), plus the six that already used it. Visible in `done-dark` (disabled Finish, then the (?) button). Leftover: `settings.rs:160-161` refresh-compat is disabled when offline with a tooltip as the only explanation (m1). |
| B4 | Destroyed list hidden behind a collapsed accordion | **Fixed** | A permanent "Erased disks" block under the triad: `sda · WDC WD40EFRX-68N32N0 · …K0000 · 3.64 TiB` per disk (`v2-app-confirm-light.png`). Names, serial tails and sizes are visible (partly on the first screen at 960x640, the rest after a scroll with the triad). |
| B5 | Non-Lucide glyphs (log levels, manage check) | **Fixed in code** | A search for the old code points (`\u{25cf}`, `\u{25b2}`, `\u{2716}`, `\u{2713}`) in `ui/**` returns nothing; `screens/logviewer.rs:119-121` maps levels to Lucide `INFO` / `WARN` / `ERR` roles and colours (`log_inf`, `log_wrn`). **But the log viewer still has no screenshot**, so the rendering is unseen (see m2). |

## Round-1 non-blocking list: status

| # | Item | Status |
|---|---|---|
| 1 | Tags tinted and stretched | **Fixed.** `kit::tag` is now `Tag::custom(transparent, fg, full border)`; pixels: border `#004B96` / `#005725` / `#6F4606` / `#A70311` (the tone's text colour); tags hug their content; height 25 px. |
| 2 | Stepper: upcoming labels, failed state | **Fixed.** Upcoming labels `#525252` (7.49:1) light and `#B1B1B1` (9.23:1) dark; failed step is a red `circle-x` disc with the red bold label (`failed-post-light`, `failed-agent-dark`; pixel 7.55:1). |
| 3 | Install: two numbering schemes, no strip | **Fixed** (DEV-UI-17): "13%  Step 2 of 12 · Preflight", 12 segments (done `#047E39`, running `#7139CB`, pending `#D0D0D0`), a "No return" tag with info over segment 3; Steps and Log collapsed. |
| 4 | Recovery buttons misleading; placeholders | **Fixed** (DEV-UI-12 rewritten): rows run the engine; each has a "Show command" icon button. Placeholders now appear only inside the Manual recovery dialog, labelled (`/dev/sdX`, `<saved boot-flash image>`) with a warning to check twice. |
| 5 | Wordmark, window controls, black strip | **Wordmark fixed** (two-tone logotype in every shot). **Window controls and the strip are not explained**, see "Evidence the real gate still lacks". |
| 6 | Scrollbar mode | **Recorded** as DEV-UI-16; acceptable. |
| 7 | Off-scale spacing | **Fixed:** no `px(10.0)`, `px(14.0)`, `px(3.0)` left in `ui/**`. |
| 8 | Inter Medium bundled and used | **Fixed:** no 500 weight left in `ui/**`. |
| 9 | Triad inconsistency, detached info icon | **Not fixed** (m3). |
| 10 | Done SSH line truncated | **Fixed:** wraps onto two lines (`done-dark`). |
| 11 | Inventory columns, trailing empty column, Largest | **Partly:** trailing empty column and blank rows remain at 960; **Largest is worse than predicted** (NB1). |
| 12 | Raw CPU string | **Fixed:** "Celeron J3455". |
| 13 | Stale docs | **Fixed:** `docs/GUI.md` cites DEV-UI-1 to 19, no "not bundled yet", no withdrawn token file. |

## Blocking finding

### NB1 Find at Largest text: horizontal scroll and clipped Status column
* **Where:** `v2-app-inventory-light-largest.png` (960x640, 130 %).
* **Evidence:** the table shows its own horizontal scrollbar (dark bar at y=437), "Community-reported" is cut at the right edge of the Status column, and "Found by" is truncated mid-word. The columns are fixed-pixel widths in `inventory.rs:60-66` (150, 140, 90, 210, 200 px) that do not follow the rem scale. BRIEF-v2 "also required of every screen": no horizontal scrolling and scalable text; Find is the first screen every user sees.
* **Not covered by a deviation:** DEV-UI-15 only says the Confirm reorder is not ported; it does not cover tables.
* **Must change:** column widths in rems (or flexible), moving "Found by" into a tooltip or the row detail at Largest, or wrapping the Status tag; then re-shoot Largest with no horizontal bar. Other tables (Inspect disks, Recovery, Manage) have no Largest shot, so check them in the same pass.

## Major findings

* **M1 Goal: the destructive path carries no data-impact statement.** `v2-app-goal-light.png` and `goal-dark.png`: "Install Debian: Replace DSM with Debian." with "Quick setup" as the primary and "Customize", and nothing says "Erases disks". The mockup tags each goal ("Disks untouched", "Erases disks", "Reads state first"; BRIEF principle: every action that writes says its effect up front). Add the three Tags as in `v2-mockup-desktop-goal-light.png`.
* **M2 Quick setup dialog is thinner than the approved one.** `v2-app-dlg-quick-light.png`: the recommended defaults are one line of monospace prose ("Wipe all · system 32 GiB × 3 · free 20 % · Newest kernel · choose a layout"), the radio rows are plain, and the mockup's System / Boot rows and the "password is generated and shown once" and "Confirm shows what is erased" sentences are gone. Monospace is for code and identifiers, not a sentence. The mixed-disk choice itself (RAID 5 vs Hybrid with usable size and survival, the Hybrid info Alert, "Use these" disabled with a visible reason) is right and uses the real `Radio`.
* **M3 DEV-UI-18 changes the approved failure experience and needs an explicit sign-off.** The owner approved a mockup with an "Open rescue shell", an SSH command and a one-time password on the Failed page. The implementation removes it because the engine has none (`docs/RECOVERY.md`: no rescue shell and no SSH server in the installer) and rewrites the copy accordingly (`copy.rs:354,407-413,660`). That is the honest decision and I accept it, but the approved mockup is now wrong about the product: annotate `design/mockups-v2/APPROVED.md` or the mockup README, and have the owner and UX critic acknowledge that the 15-minute window is now "read the log and export diagnostics", not a shell.

## Minor findings

* **m1** `settings.rs:160-161`: Refresh is disabled when offline with a tooltip only; use `with_reason`. `plan.rs:79-89`: the system-size slider and the Recommended icon button are disabled when `max < min` with no reason beside them.
* **m2** Failed pages (`failed-post-light`) show the log excerpt as a dark panel with plain `INF` / `WRN` words in one colour (`#DEDEDE`, 13.98:1; pixel-checked), no icon and no level colour, unlike the log viewer's design. In `failed-agent-dark` the expanded excerpt is an empty black bar (no "No log lines" text).
* **m3** Confirm: "Destroyed" prints its figure in small monospace while "Preserved" uses sans; "Changed" shows a lone "-"; the alert's info icon sits outside the alert, far right; the "ERASE DS918+" heading is a loose red line above the card (the mockup has a header band inside). At Largest the three cards wrap and their heights diverge.
* **m4** Dark dialogs use the canvas (`#0A0A0A`) as their background (`dlg-hostchanged-dark`): against the dimmed page the dialog edge relies on a 1.4:1 hairline. The DS maps Dialog to `background`, so this is by design, but the mockup's dialogs use the surface step; consider `popover` in dark.
* **m5** The trust and recovery dialogs are text-heavy: the changed-key dialog about 85 words plus two fingerprints and a command, the manual recovery dialog about 95 words plus two commands (DS dialog budget 80, my estimate by eye). Check that the `copy.rs` budget tests count dialog bodies, and trim.
* **m6** Install strip: pending segments are 1.54:1 (light) and 1.48:1 (dark) against the page, running 4.32 / 3.93 against pending. The text carries the state, so acceptable; add a 1 px outline to pending segments if the strip is meant to stand alone.
* **m7** Inventory: the trailing empty table column and a blank block under the last row remain at 960; the grey row in `inventory-light` is a hover capture, not a stripe.
* **m8** Recovery rows now have two controls (the action and an icon-only "Show command" `>_`, 24 px, tooltip on hover plus an accessible label). Acceptable under DEV-UI-3, but the icon has no visible label and the pair looks cluttered at 960 (`recovery-agent-dark`).
* **m9** Failed-post puts the primary on "Export diagnostics" (`failed-post-light`) while the agent variant correctly puts it on "Manual recovery"; confirm with the UX critic that exporting is the right first action after a post-point-of-no-return failure.
* **m10** Failed alerts print raw engine text starting with a lowercase letter ("the installer reported a failure: grub-install failed on sdc: ..."); capitalise it or put the engine text under a fixed heading.
* **m11** Help topics are still one dense block of about 90 words in a 480 px sheet (`help-dark`).
* **m12** The dialogs have both a close X and explicit buttons; for the trust dialogs the X means refuse (DEV-UI-19), which is correct, but give the X an accessible name that says so.

## DEV-UI-15 to DEV-UI-19

| Id | Verdict | Reason |
|---|---|---|
| 15 container queries not ported | **Acceptable for Confirm only** | At Largest on 960x640 the two columns stay, the summary and the gate are on the first screen together, and Erase stays pinned (`confirm-light-largest.png`): equivalent safety to the mockup's reorder. It is **not** acceptable as a general excuse: NB1 is the same missing work on a table. |
| 16 `Always` scrollbars | **Acceptable** | Thumb, inset and colours pixel-verified in round 1 (6 px, `#808080` / `#868686`). |
| 17 custom 12-segment strip | **Acceptable** | No widget exists (DEV-UI-6); status is also text. |
| 18 no rescue panel | **Acceptable and honest, needs sign-off** | See M3. |
| 19 host-key trust dialog fed by a channel | **Acceptable, verify the security claim** | Cancel, Escape, closing the window and a 3-minute timeout all refuse; a changed key opens a danger dialog with both fingerprints, the forget command and Cancel only (`dlg-hostchanged-dark`), which matches the mockup's intent. The dialog says nothing, "not even your password", is sent before the user trusts it: that must be a tested property of the engine's SSH layer, not only copy (the author reports tests in `ui::trust`; I could not run them). |

Also changed since round 1 and worth recording: DEV-UI-12 (recovery now runs the engine; "never run against a real unit"), DEV-UI-13 (update calls `push_update` / `push_rollback`; "unverified against a real unit") and DEV-UI-14 (keychain only). The two "never run against a real unit" caveats are gate evidence gaps, not design defects.

## Contrast, measured on the new screenshots

All values are from pixels (dominant background and the farthest text colour in the region), not from tokens. Text everywhere is at least 5.3:1 except disabled controls.
* Light: title 18.97; alert text on tinted fill 7.01 to 7.41; Tag texts 6.2 to 8.0 (`#A80717` 7.43, `#065A29` 8.04); stepper failed label 7.55; log text on the always-dark panel 13.98.
* Dark: alert 9.68 to 11.42; tags 10.25 to 11.71; red address in the changed-key dialog 10.14; command block 15.34; muted text 8.38 to 9.23; stepper upcoming 9.23.
* Non-text: segmented container and checkbox border `#808080` 3.95:1; selected row border 2.24:1 (with the radio as the primary cue, as the DS specifies); install strip pending 1.54:1 (m6).
* Disabled (exempt): Erase 2.14 light / 2.98 dark; "Use these" 2.17. Reasons are on visible info buttons.

## Evidence the real gate still lacks

**Cannot be produced here (Linux, Xvfb and lavapipe only):**
* Windows and macOS screenshots (DirectWrite and Core Text text, native window controls, scrollbar behaviour, DPI scaling at 125 / 150 / 200 %). Everything here is Linux software rendering; text has subpixel colour fringes.
* Real-hardware behaviour: scan, SSH, the engine's recovery (DEV-UI-12) and update (DEV-UI-13) paths, real trust-on-first-use against a unit.
* Screen readers, IME and password managers.

**Can be produced here but is missing:**
* **Window controls and the 2 px black strip.** No capture shows minimise, maximise or close, and all 55 captures have `#000000` at x=0 and 1 (checked on every file) with the app drawn from x=2. Needs a run under a real window manager (or an explanation that it is the Xvfb root showing) and a screenshot with the controls visible.
* **Hover, pressed and keyboard-focus states** for every control class in both themes (the 2 px, offset-2 ring is not shown anywhere; the only hover is incidental, the grey DS923+ row). Also the scrollbar thumb on hover (8 px).
* **Tab order** of Confirm, Connect, the dialogs and the Help sheet (a recorded walk, not a screenshot), and focus return from dialogs and Popovers.
* **Screens and states never shown:** the log viewer (a whole screen); the Copy command, Keep, Cancel-install, Report, Slices, Add-by-IP, typed-confirm (restore / remove), Update and First-launch dialogs; the Popover opened by a disabled-reason info button; the Select popup; Confirm with Erase enabled, in keep mode and for a twin unit; Find empty, scanning and failed; Connect failures (login, unreachable); Install with the log open and past the point of no return; Manage degraded and updating; Settings pages other than General; Help topic, search and shortcuts; About with Credits open.
* **Dark missing for:** System, Failed (pre-install), Recovery fresh, Manage storage, and the Manual-recovery and Quick-setup dialogs (dark exists for 19 families).
* **Largest text missing for** Connect, Inspect, Goal, System, Install, Done, Manage, Recovery, Boot manager, Settings, Help, About and all dialogs; no Largest dark shot at all (Largest exists for Find, Setup, Confirm and the two Failed variants). 1180x760 exists for Confirm, Find, Setup, Install and Failed-post only.
* **An automated walk** over every screen at 960x640 and Largest proving no horizontal scroll or clipping (NB1 shows screenshots alone miss it), run in CI next to the token tests.

## Recommended next steps

1. NB1: flexible or rem column widths in the Find table, then re-shoot Find at Largest; check every other table at Largest.
2. M1 and M2: the three goal Tags; Quick setup content closer to the mockup.
3. M3: record the owner's acknowledgement of the no-rescue-shell decision next to `APPROVED.md`.
4. Fill the evidence list above, starting with a window-manager capture (controls and strip), the focus and hover set, the log viewer, and Largest for the remaining families.
