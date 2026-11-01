# UI critic, implementation fidelity review, round 3

Scope: re-check of `ui-critic-impl-round2.md` (NB1, M1 to M3, m1 to m12) against GUI round 3 (up to `00c1fc3`): the 107 `design/screenshots/v2-app-*.png`, `crates/synofurb/src/ui/**`, `crates/synofurb/assets/tools/gui-walk.sh`, `design/v2/DESIGN-SYSTEM-V2.md` section 13 (DEV-UI-12, 18 to 21) and `design/mockups-v2/APPROVED.md` ("Changes after approval (owner sign-off pending)"). No gate verdict; nothing written to `design/approvals/`; no code edited.

How I checked: I read the sources; looked at 34 of the 107 screenshots; measured contrast from pixels (dominant background against the farthest text colour in a box, so every number below is rendered, not a token); diffed screenshots byte for byte; ran `gui-walk.sh` myself (details below, `CARGO_TARGET_DIR=/tmp/gpui-probe-target`, no new target dir; scratch files are in the session scratchpad, nothing was written into the repository except this file). I did not run `cargo test` (disk); I ran `gui-walk.sh` at Largest in both themes, the engine's SSH tests, or anything on Windows or macOS.

## Verdict in one paragraph

The round-2 blocker is fixed and verified, and the three majors are closed or accepted: Find at Largest no longer scrolls sideways, the Goal screen says "Erases disks", Quick setup is close to the mockup again, and the rescue-shell removal is written down. The work is now at the point where the **evidence**, not the design, is the weak part: four of the new screenshots do not show what their names say (two are byte-identical to another file, one shows a table when it should be empty, one dialog contradicts the page behind it), the keyboard focus ring renders at 2.3:1 and 2.5:1, and the owner sign-off for the post-approval changes is still pending. **No blocking design finding remains; there are 3 major findings and 11 minor ones, plus a list of evidence the real gate still lacks.** Counts: **blocking 0, major 3, minor 11.**

## Round-2 findings: status

| # | Round-2 finding | Status | Evidence |
|---|---|---|---|
| NB1 | Find at Largest: horizontal scroll, Status clipped | **Fixed, verified** | `v2-app-inventory-light-largest.png`: all five columns and the Status tags (including "Community-reported") fit, no horizontal bar, "Found by" ends in an ellipsis. The old screenshot (commit `1996dce`) fails the walk's scrollbar detector (it finds the 6 px bar); the new one passes, so the detector would have caught NB1 (see the walk section). The trailing blank header cell and blank rows remain (m7 below). |
| M1 | Goal: no data-impact statement | **Fixed** | `goal-light.png`, `goal-light-largest.png`: Install Debian carries an "Erases disks" Tag (outline, trash icon, `#A70311` text 7.55:1 on `#FAFAFA`), Boot manager "Disks untouched", Recovery "Reads state first". Tags are text plus icon, not colour alone. |
| M2 | Quick setup thinner than the mockup | **Fixed** | `dlg-quick-light.png` / `dlg-quick-dark.png`: real Radio rows with usable size and survival, the Hybrid info Alert, System and Boot rows in sans (no longer a monospace sentence), and the two notes ("password generated and shown once", "Confirm shows what is erased"). "Use these" disabled with a visible (?) reason button (2.20 / 2.82:1, exempt). Contrast all above 6.2:1 except disabled. |
| M3 | DEV-UI-18 needs sign-off and an annotation | **Annotated; sign-off still pending** | `APPROVED.md` has a "Changes after approval" table whose first row is the missing rescue panel, with an explicit statement that nothing in it is an approval. That is the right form. Still open: the owner has not signed it (see M-A below). |
| m1 | Disabled controls with tooltip-only reasons | **Fixed** | `settings.rs:153` (refresh-compat) and `plan.rs:79-89` (Recommended size) now use `with_reason`. |
| m2 | Failed log excerpt plain; empty bar | **Fixed** | `failed-post-light.png`: INF / WRN rows with Lucide icons and level colour, amber bar on WRN; `failed-agent-dark.png` shows "No log lines yet.". |
| m3 | Triad typography, detached info icon | **Fixed** | Destroyed, Changed and Preserved are all sans, "Nothing else" replaces the lone dash, the ERASE heading has a red icon. At Largest the cards still wrap, acceptable. |
| m4 | Dark dialog background | **Fixed** | Dark dialogs are `#171717` (popover) with a `#2A2A2A` edge on a `#040404` scrim; the page canvas is no longer reused. Dialog against scrim is 1.14:1 in dark (edge hairline only); acceptable, a dialog is not a control. |
| m5 | Dialog density | **Not changed, accept** | Manual recovery is about 90 words plus four commands in a 440 px dialog; changed-key is about 60 words plus two fingerprints and a command. Readable in the shots; no budget test covers dialog bodies that I could find. |
| m6 | Install strip pending segments 1.5:1 | **Not changed, accept** | Text carries the state ("Step 6 of 12 . name"). |
| m7 | Inventory trailing empty column / blank rows | **Not changed** | Still visible at 960 and at Largest. |
| m8 | Recovery rows have two controls | **Not changed, accept** | `recover-restore-nobackup-light.png`: `>_` icon plus labelled action. |
| m9 | Failed-post primary is "Export diagnostics" | **Not changed** | Needs the UX critic's view. |
| m10 | Raw lowercase engine text in the failed alert | **Not fixed** | `failed-post-light.png`: "the installer reported a failure: grub-install failed on sdc: ...". The string comes straight from `demo.rs:341` (and from the engine in real runs); a `capital()` helper exists in `worker.rs:267` and is not applied here. |
| m11 | Help topics dense | **Not re-checked** | `help-light.png` unchanged in structure. |
| m12 | Dialog X accessible name | **Unverified** | I found no `aria` / accessibility label on the dialog close button in `overlays.rs`; the first-launch dialog states that screen readers are unsupported (UX-SPEC accepted baseline), so this is low. |

## Findings, ranked

### Blocking
None found in the design or the implementation. The gate-level blockers are evidence the critics cannot produce here (last section) and the pending owner sign-off.

### Major

**M-A. Owner sign-off for the post-approval changes is still pending, and the register is longer than the list.** `APPROVED.md` lists six changes. Section 13 also records user-visible departures that are not in that table: DEV-UI-11 (failure clocks are static rules, not the animated countdowns the mockup shows), DEV-UI-13 (Manage update runs the engine; "unverified against a real unit"), DEV-UI-14 (no keychain, nothing offers to save a password), DEV-UI-15 (Confirm reorder and hidden unit header at Largest not ported), DEV-UI-17 (custom 12-segment strip). The recorded hashes for `UX-SPEC.md` and `DESIGN-SYSTEM-V2.md` no longer match the files on disk (expected, the file says they were extended), so the approved versions can only be recovered from git. Must change: have the owner sign the table (or reject rows), and add the five rows above or state why they are not user-visible; record the approved file hashes' commit.

**M-B. Four screenshots do not show what their name says.**
* `v2-app-inventory-empty-light.png` is **byte-identical** to `inventory-light.png` (md5 `f0a26464`): it shows the five-unit table, not the empty state. I rendered `gui --demo --screen inventory-empty` with the current binary and got the same pixels, so this is the demo, not a stale file: `app.rs:588` clears the inventory and the demo discovery then fills it again before the capture. The Find empty state (the `Empty` component, "Help" link) has never been seen.
* `v2-app-hover-primary-light.png` and `-dark.png` are **byte-identical** to `goal-light.png` / `goal-dark.png`. The Quick setup button is `#7139CB` (the idle accent), not `accent_hover` `#622CB5`; no hover is shown.
* `v2-app-pressed-primary-light.png` differs from `goal-light.png` only in the "Erases disks" tag border (`#B01C28` against `#A70311`); the button is idle. The mouse was over the wrong element.
* `v2-app-dlg-next-light.png` says "No disk was changed. This installer does not reboot the NAS by itself ... DSM starts as before" on a page that says **Disks changed**, "Step 11 of 12" and shows the POINT OF NO RETURN log line. The demo target sets `past_no_return: true`, but `next_item` is fed `install.past_no_return` (`app.rs:738`, `overlays.rs:83`) and evidently reads it as false when the demo opens the overlay, most likely because the demo's event replay sets the flag after the dialog opens (I did not trace the order). In the product the dialog opens on click, after the state exists, so I believe the code is right; but as evidence it shows exactly the wrong text for the dangerous case, and `DLG_NEXT_WIPE_HALT` (the text that should appear here) has never been shot. `docs/GUI.md` (line 150) says hover and pressed were captured: that sentence is not supported by the files.
Must change: fix the demo ordering, re-shoot these four, or delete the files and the claim. Not a design defect, but a reviewer approving from these files would approve the wrong thing.

**M-C. The keyboard focus ring renders at 2.3:1 (light) and 2.5:1 (dark).** `focus-ring-confirm-light.png`: the ring around the menu button is 3 px of `#B397E0` against `#F5F5F5` = **2.29:1**; dark `#5F4B8C` against `#1B1B1B` = **2.50:1**. The design system specifies a solid `ring` of `#7139CB` / `#A37BFE` (6.67 and 6.43:1) and `contrast_v2.py` checks that token, but the widget paints it at roughly 40 % alpha (the shadcn-style `ring/50`), so the rendered indicator is under the 3:1 non-text minimum the brief relies on ("visible focus ring"; WCAG 1.4.11). It is visible, and the dark ring is noticeably faint. The capture shows only one control (the menu button, not a Confirm control as the file name says): ring on a text button, an input, a checkbox and a table row is still unseen. Must change: set the ring colour (or the theme's focus-ring alpha) so the blended result is at least 3:1 on every surface, add the rendered pixel to `contrast_v2.py`, and shoot the ring on four control classes in both themes.

### Minor

* **m-1 Update dialog shows "0.2.0 to 0.2.0".** `update-light.png`: the version Tag has the same number on both sides (demo constants). The body is honest ("Installing updates is switched off in this build: the signing key is a placeholder", Download disabled with a visible reason).
* **m-2 Demo paths leak into evidence.** `/tmp/guihome2/.local/share/synofurb/dom-backups` (`recover-restore-nobackup-light.png`) and `ssh -i /tmp/guihome2/.loc...rb/keys/nas_ed25519` (`done-light.png`, `dlg-report-light.png`). Harmless, but the shots look like a machine-specific build.
* **m-3 Cancel install dialog.** `dlg-cancel-install-light.png`: the primary (filled) button is "Stop install", the safe choice "Keep going" is outline. The wording is clear and the dialog says nothing is written before step 3; confirm with the UX critic that the destructive-looking verb should hold the primary, since Escape and the safe action are the less prominent pair.
* **m-4 Copy-command line wraps inside flags.** `dlg-command-light.png`: `--mode wipe --` ends one line and `raid auto` starts the next (also `--` / `username`); a reader sees a stray `--`. A narrower break rule (no break after `--`) or a non-wrapping, horizontally scrolling block would read better. The line also carries `--have-backup --accept-data-loss --confirm 'ERASE DS918+'` once the gate is satisfied; that is correct for the CLI equivalent but the caption ("Edit the line if you like") does not say that the line answers the data-loss questions for you.
* **m-5 Keep mode wording.** `confirm-keep-light.png`: "Changed: Nothing else" while "Rewritten disks" lists the disks whose layout is rewritten; the title is still "Confirm erase". Check with the UX critic that the three-way split is true for Keep.
* **m-6 Find skeleton.** `inventory-scanning-light.png`: three grey placeholder rows, a spinner floating right of "Add by IP", and the primary Connect button enabled although nothing is selected (idle `#7139CB`). Connect should be disabled with a visible reason while nothing is selected.
* **m-7 Find table.** The trailing blank header cell and blank space below the last row remain (round-2 m7). At Largest "Found by" loses information to the ellipsis with no way to read it (no tooltip on a focusable element, DEV-UI-1).
* **m-8 Largest leaves a small working area.** At 960 x 640 and Largest the pinned top bar, stepper, unit header, action bar and status strip take about 290 px, leaving about 345 px (54 %) for content (`system-light-largest.png`, `recovery-fresh-light-largest.png`: one and a half rows visible). This is the cost of DEV-UI-15 (hidden unit header not ported); acceptable but the first thing to fix if anyone tests at Largest.
* **m-9 Failed-post primary and the engine text** (round-2 m9 and m10, unchanged, see the status table).
* **m-10 Dialogs at Largest, never shot.** The manual-recovery dialog already fills 557 of 640 px at default size (`dlg-manual-light.png`); at Largest it must scroll, and no capture shows that.
* **m-11 Walk helper not in CI.** `gui-walk.sh` is referenced only by `docs/GUI.md` and DEV-UI-21; `.github/workflows/ci.yml` does not run it, so the guard against another NB1 depends on someone remembering.

## DEV-UI-12 and 18 to 21

| Id | Verdict | Reason |
|---|---|---|
| 12 Recovery runs the engine | **Acceptable as a design; evidence incomplete** | Two shots of the actions (`recovery-agent`, `recover-restore-nobackup`, `recovery-debian`) and the "How recovery works" dialog (`dlg-recovery-light`, which is the explainer, not an action dialog). Never shot: Restore flash with a saved image (the typed phrase `RESTORE FLASH <model> <last four>`), Remove entries typed phrase, Open agent warning on a Debian unit, Boot DSM with the reboot choice, and every outcome state (success, "not confirmed", `AgentTimeout`). These are the riskiest dialogs in the product. "Never run against a real unit" stays a gate evidence gap. |
| 18 No rescue panel | **Acceptable and honest; owner has not signed** | The page shows disks, flash, whether the unit reboots, the log, Copy log, Export diagnostics and Recovery options (`failed-post-light`, `failed-agent-dark`). The remaining copy ("there is no rescue shell", "We promise nothing about your data") is consistent across `dlg-recovery-light`, `dlg-manual-light` and the Failed page. Open: M-A (sign-off) and the wrong dialog in M-B. |
| 19 Host-key trust dialog | **Acceptable, and the security claim is now tested in the engine** | `dlg-hostkey-light/dark`: fingerprint on three lines with Copy, "Answer within 2:51", Cancel and Trust; `dlg-hostchanged-light/dark`: danger dialog, both fingerprints, the forget command, Cancel only. Contrast 6.3 to 19.8:1 (fingerprint block 15.6:1 dark, 17.8:1 light; red address 7.88 / 9.19). I read `crates/synofurb-core/src/remote.rs`: `check_server_key` runs during the handshake before authentication and the test `a_refused_host_key_closes_the_connection_before_the_password_is_sent` uses a real russh server that counts password attempts, so "not even your password" is a tested property, not only copy. I could not run it. Not shot: the "Connection ended" state, Trust disabled with its reason. |
| 20 Manage sign-in | **Acceptable** | `manage-signin-light/dark`: the Alert says whose address is asked ("192.168.1.60: Use the user and password or key of the NAS at this address"), segmented Password / Key (the selected segment is filled and bold, 6.30:1 dark), placeholder 6.24:1. The unit binding is a logic claim (tests in the report); I did not verify it. |
| 21 Find shares | **Acceptable, verified** | See NB1 above and the walk below. Shares of the table width never exceed it, so a horizontal bar cannot appear from column widths. |

## The walk (`gui-walk.sh`)

* **What it does.** For each demo screen: Xvfb, the binary, a root capture; fails if (a) the 7 px strip x = W-22 to W-15, y = 210 to H-120 has pixel deviation above 0.3 (content in the right margin) or (b) a 6 px tall run of the scrollbar thumb colour at least 60 px long exists inside the page (a horizontal bar). The Help sheet is exempt from (a).
* **My runs, 960 x 640, Largest text: light, 51 of 51 screens `ok`; dark, 51 of 51 screens `ok`** (about 9 minutes each under software Vulkan; the 51 are the demo screens `--screen list` offers). No overflow, no gutter content, no horizontal bar at Largest in either theme on the first viewport of any screen.
* **Does it work?** Yes for NB1: I ran its two detectors on the round-2 screenshot of Find at Largest (from git, commit `1996dce`) and on the current one: round 2 gives horizontal-scrollbar = 1 (gutter deviation 0.00, so only detector (b) fires), the current file gives 0 and 0.
* **What it cannot see.** (1) Only the first viewport is captured, so a wide table below the fold is not tested (Manage Storage, Settings, Inspect disks tables). (2) Clipping by `overflow: hidden` with no scrollbar is invisible to it. (3) It walks `--screen list` only; the log viewer, the Cancel install, Command, Report and Add-by-IP dialogs and the Popover are separate demo paths (11 of the 52 screenshot families come from other paths and are not in the list) and are never walked, and neither is any dialog at Largest. (4) It takes one theme and one text size per run (I ran Largest in both; default and Large not run). (5) It is not in CI (m-11).

## Contrast, measured on the new dialogs and states

Rendered pixels; text is at least 6.2:1 everywhere except disabled controls.
* Light dialogs: title 19.8; body prose 19.8; muted text (`System`, footnotes, "Answer within", cancel-install body) 7.49 to 7.81; alert text 8.05 (blue tint `#F5F8FB`); red address 7.88; code blocks 17.8 (`#F3F3F3`); Tag texts 8.4 to 8.8; primary buttons 6.67.
* Dark dialogs (`#171717`): title and body 17.2; muted 8.0 to 8.4; alert 8.98; red address 9.19; code block 15.6; primary `#A37BFE` with `#0E0C15` 6.30; outline Cancel 11.2.
* States: Erase enabled light `#C60014` with white 6.16, dark `#E84642` with `#0A0A0A` 5.08; "Matches" tag 8.42 / 11.17; checkbox 6.39 / 6.43; log level words on the always-dark panel: INF 10.1 / 10.9, WRN 8.6 / 9.8, DBG 7.6, timestamps 7.02; level icons 7.6 to 9.8; amber row bar 11.4 / 12.3.
* Non-text: radio ring 3.68 light, 3.18 dark; selected Find row border `#B193E5` is 2.24:1 against its row with the radio as primary cue (unchanged, accepted); **focus ring 2.29 / 2.50 (M-C)**; install strip pending 1.5.
* Disabled (exempt, reasons visible): Quick setup "Use these" 2.20 / 2.82, install Cancel 2.33, update Download 2.20.

## Evidence the real gate still lacks

**Cannot be produced here (Linux, Xvfb, lavapipe, no window manager, no xdotool):**
1. Windows and macOS captures (DirectWrite and Core Text rendering, native window controls, DPI at 125, 150 and 200 %), and Linux under GNOME or KDE on Wayland and X11 with a real window manager. All 107 captures have a 2 px black strip at x = 0 and 1; `docs/GUI.md` explains it as the X root window and that client-side controls are not drawn without a window manager. Plausible, but unproven: only a capture under a window manager that shows minimise, maximise and close settles it, and it is the one thing that says whether a user can close or move the window on GNOME.
2. Real-hardware behaviour: scan, SSH, trust against a real unit, the recovery actions (DEV-UI-12), Manage update and rollback (DEV-UI-13), a failed install path.
3. Screen readers, IME, password managers, high-contrast OS modes (the first-launch dialog states that screen readers are unsupported; that baseline is already accepted in UX-SPEC).

**Can be produced here, still missing or wrong:**
4. **Hover and pressed states that actually show** (M-B): primary, outline, ghost, icon button, table row, checkbox, radio, segment, in both themes. Today none exists except an incidental row hover.
5. **Focus ring on four control classes in both themes**, and the tab order walk (Confirm, Connect, Setup, every dialog, Help sheet) as a recorded sequence; focus return after a dialog and a Popover closes.
6. **Find empty state** (M-B), the **"What happens next" dialog in all seven variants** with the correct state (`DLG_NEXT_*`), and the **"Connection ended" host-key state**.
7. **Recovery action dialogs and outcomes** (DEV-UI-12): Restore flash and Remove entries typed phrases, Open agent on Debian, Boot DSM with reboot, success, "not confirmed" and `AgentTimeout`.
8. **Never shown at all:** these demo screens have no screenshot in any theme: `manage-rebuild`, `manage-faildisk`, `manage-services`, `manage-update`, `manage-storage-degraded`, `manage-health-ok`, `inventory-offline`, `system-offline`, `recover`, `help-topic`; also About with Credits open, Settings pages other than General, Help search and shortcuts, Select popup open, Connect failures, Install with the Steps list open.
9. **Dark missing for 22 families**, among them Confirm keep and twin, Find empty and scanning, `install-after-ponr`, `failed` (pre-install), `update`, `manage-unreachable`, `manage-storage-hybrid`, `plan-keep`, the Popover, and the Report, Command, Cancel-install, Add-IP, Keep, Next and Recovery dialogs. (Light missing for `manage-storage`.)
10. **Largest text:** present for 17 families (dark only for Confirm); missing for every dialog (13 including First launch and Update), the log viewer, Update, Failed-pre, Recovery with entries (`recovery-agent`, `recovery-debian`), Manage sign-in, Confirm ready, keep and twin, and all dark pages except Confirm.
11. **Walks that go beyond the first viewport**: scroll each page to the end at Largest and check for horizontal overflow, in light and dark, including the dialogs and the Help sheet; and run `gui-walk.sh` in CI.
12. **1180 x 760** exists for 8 families only (Confirm, Find, Goal, Install, Failed-post, Manage, Plan, System).

## Recommended next steps

1. M-C: make the rendered focus ring at least 3:1 on every surface and add the rendered value to `contrast_v2.py`.
2. M-B: fix the demo ordering (`dlg-next`, `inventory-empty`), replace or delete the hover and pressed files, and re-shoot with the pointer on the control; correct `docs/GUI.md`.
3. M-A: the owner signs `APPROVED.md`; add the five missing rows.
4. Fill evidence items 4 to 8 (about 40 captures), extend the walk to scroll and to the dialogs, and run it in CI.
5. Minors: capitalise the engine text on the failed alert, the Connect-disabled-with-reason on Find, and the dialog line wrapping.
