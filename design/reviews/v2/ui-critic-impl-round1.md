# UI critic, implementation fidelity review, round 1

Scope: the implemented desktop app (`crates/synofurb/src/ui/**`, GPUI Kit 0.8.1) against the owner-approved mockup (`design/mockups-v2/APPROVED.md`, `design/mockups-v2/desktop/`), the design system (`design/v2/DESIGN-SYSTEM-V2.md`, deviations DEV-UI-9 to DEV-UI-14, `tokens-v2.json`) and `docs/GUI.md`, `docs/GUI-COMPONENTS.md`. This is a fidelity review to find what to fix before the real gate. It is **not** a release-gate verdict, and nothing was written to `design/approvals/`. I did not read other reviewers' files and did not edit code.

Evidence used: the 27 shipped `design/screenshots/v2-app-*.png` (looked at 23 by eye, pixel-measured regions in 9), the Rust sources, the mockup screenshots `v2-mockup-desktop-*` for comparison. I did **not** run the app (no build here); every statement about behaviour that is not visible in a screenshot is from reading the source and is labelled as such.

## Summary

The implementation is a serious piece of work and mostly faithful: the theme set is the designer's file byte for byte, type styles and tokens are tested against `tokens-v2.json`, Inter and JetBrains Mono are bundled and visibly used, scrollbars are themed to the token colours and the 6 px geometry, Tags are 25 px tall as the widget draws them, the Stepper is the real widget, text scaling works (title width ratio 1.147 at 115 % and 1.30 at 130 %), and every rendered text pair I measured is at least 5.3:1. But five things break approved design rules, and the evidence set is too thin for a gate (13 of 14 screen families have no dark, 1180 or Largest shot).

**Blocking findings: 5 (B1 to B5). Non-blocking: 13. DEV-UI-9 to 14 judged: 6 acceptable (some with conditions), 1 acceptable only until the gate (DEV-UI-14). DEV-UI-15 does not exist in the repo.**

## Blocking findings (fix before the real gate)

### B1 Inventory: the selected unit is shown by a tint alone
* **Where:** `v2-app-inventory-light.png`; `screens/inventory.rs` (`render_td`, lines 79-102: no radio, no selection marker).
* **Evidence (pixels):** selected row `#F1EDFE`, neighbours `#FFFFFF`, separators `#E5E5E5`: 1.15:1, and no accent border (scan at x=300: `#E5E5E5`, `#F1EDFE`, `#E5E5E5`; horizontal scan at the row start: `#E5E5E5`, `#F1EDFE`). DESIGN-SYSTEM-V2 section 7 (Table row): "the selection is never colour alone: the first cell holds a `Radio` ... checked on the selected row"; the approved mockup has it (`v2-mockup-desktop-find-light.png`).
* **Must change:** a Radio in the first cell (or an accent left bar), and the accent border the widget paints at `table_active_border`.

### B2 Segmented choices have no visible boundary and a 1.06:1 selected state
* **Where:** `v2-app-settings-light.png` (Theme, Text size, Accent), `v2-app-plan-light.png` (Mode, Mirror copies), `v2-app-connect-light.png` (Sign in with), `v2-app-bootmgr-light.png` (SSH key), `v2-app-system-light.png` (SSH key); `kit::segmented` (`kit.rs:111-125`, DEV-UI-10).
* **Evidence (pixels):** selected segment `#F3F3F3` on canvas `#FAFAFA` = **1.06:1**; the label weight does not change; there is no container border and no separator (settings row scan at the Theme control: no border pixels). Unselected options read as plain text, so a user cannot tell these are controls nor which one is on. The mockup draws a bordered container (`border_strong` 3.95:1) with a white selected thumb and a bold label.
* **DEV-UI-10 verdict:** using `ToggleGroup` instead of `RadioGroup` is acceptable; the rendering is the defect. Needs a container border in `border_strong`, a selected fill with at least 3:1 against the track (or a selected-label weight plus a check), and the same on the Log level filter.

### B3 The disabled-reason pattern is not applied everywhere (DEV-UI-1/2)
Tooltips on disabled buttons are unreachable by keyboard, which the design system forbids; the app does it in six places:
* `screens/done.rs:207-212` Finish: `.disabled(!copied).tooltip(copy::DONE_FINISH.tip)`; `v2-app-done-light.png` shows a disabled Finish with no reason button (the mockup has one).
* `screens/install.rs:193-197` Cancel: disabled after the point of no return with a tooltip only. This is the most safety-relevant reason in the app ("why can't I stop this?"); the mockup places an info button beside it.
* `screens/settings.rs:213-219` offline bundle download (tooltip only), `screens/update.rs:122-127` Restart (tooltip only).
* `screens/connect.rs:104-109` Connect disabled with an empty host (no reason affordance), `screens/update.rs:165-172` Download (disabled by `key_missing` or `!installable()`; check that the explanation sits beside it).
* The pattern is correct where it is used: Confirm Erase (`v2-app-confirm-light.png`), Goal Boot manager, Failed Wait for DSM, Recovery Remove, Boot manager Install agent, Plan Next, System Next all use `kit::with_reason`. Route the remaining ones through the same helper.

### B4 Confirm hides the list of what is destroyed
* **Where:** `v2-app-confirm-light.png`, `confirm.rs:150` (`Accordion` "Details" collapsed by default); the approved mockup shows the disk list (names, sizes) under the triad (`v2-mockup-desktop-confirm-ready-light.png`).
* **Evidence:** the screen shows only "4 disks · 14.55 TiB" and a collapsed "Details". BRIEF-v2 rule 4 and DS rule 4: "the destroyed / changed / preserved lists (names and sizes only)" stay visible. The typed phrase protects the unit, but a user cannot see which disks, by name and serial tail, without opening an accordion. This is not in DEV-UI-9..14.
* **Must change:** show the list (as the mockup does) or record it as a deviation with the owner. At 960x640 it scrolls under the triad, which the layout already allows.

### B5 Non-Lucide glyphs in the UI
* **Where:** `logview.rs:12-17` log level markers are the Unicode characters `\u{25cf}` `\u{25b2}` `\u{2716}` and `\u{00b7}`; `screens/manage.rs:153` uses `\u{2713}` for done update steps.
* **Rule:** BRIEF-v2 item 4 (Lucide only) and DS sections 5 and 7 ("log level glyph = Lucide icon plus the word"). The bundled Inter is a Latin subset (`docs/GUI-COMPONENTS.md` section 5) and I would expect it to lack these symbols (not checked), in which case they fall back to whichever system font exists and differ per OS, the problem the bundled fonts exist to solve. The log viewer has no screenshot at all.
* **Must change:** Lucide `circle-alert`, `triangle-alert`, `circle-x` (or the roles already in `icons.rs`) plus the 3-letter word.

## Non-blocking findings

1. **Tag rendering differs from the design.** `kit::tag` (`kit.rs:62-68`) builds `Tag::custom(c.bg, c.fg, c.border)`: a tinted fill with a half-strength border, whereas the DS and the mockup use an outline Tag (transparent, border in the full `*_fg`). Visible in every shot (`v2-app-inventory-light.png`: pale green fill and a `#66B585` border). Text contrast is fine (7.3:1 verified, 6.1 known issues, 6.9 unsupported). In the inventory the Tag fills the whole Status cell (about 185 px, from x=626 to 810) instead of hugging its content. Height is correct (25 px, measured 237 to 261).
2. **Stepper.** It is the real widget (good). Differences from the mockup: upcoming labels are full ink (`#0A0A0A`, 18.97:1) instead of muted, so "current" is told by weight alone; and the failed state is not drawn: `v2-app-failed-light.png` shows step 6 in the current purple, whereas the DS says the failed step is a `circle-x` disc in danger with the step name kept.
3. **Install screen.** The mockup's big percentage, 12-segment strip with the point-of-no-return flag, and collapsed Steps are replaced by a thin bar, "13% Phase 5 of 7" and an expanded Steps list (`v2-app-install-light.png`). Two numbering schemes appear on one screen ("Phase 5 of 7" and steps 1 to 12), which is confusing; "(i) Install" beside it is unexplained. None of this is in DEV-UI-9..14.
4. **Recovery.** The row buttons say "Restore" and "Remove" but, per DEV-UI-12, they ask for a typed phrase and then only show a command with unfilled placeholders (`<backup>.img`, `<ip>`, `recover.rs:176+`). A typed confirmation for an action that does not run is friction without safety; rename the buttons to what they do ("Show command") or drop the phrase. Also `docs/GUI.md` says the primary is "Manual recovery" when the agent did not answer, while the approved mockup gives Boot the primary there (`v2-app-recovery-agent-light.png` vs `v2-mockup-desktop-recovery-agent-light.png`): record the change.
5. **Title bar.** The wordmark is plain semibold text (`render.rs:226-231`), not the two-tone logotype (`brand::logo`) that the mockup and About use. No window controls (minimise, maximise, close) appear in any shot; I cannot tell whether that is the Xvfb capture without a window manager or a missing CSD control set. A 2 px black strip runs down the left edge of every capture (`#000000` at x=0 and 1): probably the capture, but check it.
6. **Scrollbar mode.** `theme.rs:337` sets `ScrollbarMode::Always` for every scroller; the DS says `Always` only for the log and tables, `Scrolling` elsewhere. The look is right (6 px thumb `#808080` / `#868686` pixel-verified, transparent track), so this is a harmless but unrecorded deviation.
7. **Spacing off the DS scale.** Literal `px(10.0)` (8 uses), `px(14.0)` (6) and `px(3.0)` (2) in `screens/*.rs`, `render.rs`; the scale is 2, 4, 6, 8, 12, 16, 20, 24, 32. `docs/GUI.md` says screens contain no pixel constants; they do.
8. **Weight 500.** `Inter-Medium.ttf` is bundled (`assets.rs:21`) and used (`screens/about.rs:125`) although the DS says only 400, 600 and 700 ship. Remove the file or amend the DS.
9. **Mixed typefaces in the Confirm triad.** "Destroyed" shows its figure in monospace small, "Preserved" in sans (`v2-app-confirm-light.png`); the "Changed" card holds a lone "-". The alert's info icon sits outside the alert box on the far right. The ERASE heading is a red mono line above the card instead of a header band inside it.
10. **Done SSH line** is middle-truncated (`ssh -i /tmp/guihome2/.loc...rb/keys/...@192.168.1....`): the part a user needs, the address, is cut. Copy exists, but show it wrapped.
11. **Inventory table.** An empty trailing column and blank rows fill the table frame (`v2-app-inventory-light.png`); fixed px column widths (150, 140, 90, 210, 200 = 790 px) will not scale with the text size, so Largest text will truncate or scroll (no Largest shot of Find).
12. **Unit header** prints the raw CPU string ("Intel(R) Celeron(R) CPU J3455 @ 1.50GHz") where the mockup has "Celeron J3455".
13. **Documentation is stale and contradicts the code.** `docs/GUI.md` "Platform notes" says Inter and JetBrains Mono "are not bundled yet" and "Theme mapping" cites the withdrawn `design/mockups/ui/tokens.json` and `theme::apply(palette, cx)`; both sections predate the rebuild. It also says "screenshots in light and dark at 960 x 640 and 1180 x 760" which overstates what exists (below). DEV-UI-15 is referenced in the task but exists nowhere in the repo (the register ends at DEV-UI-14).

## Checked and fine

* **Tokens and type scale.** `theme.rs` type styles (24/32 700, 16/24 600, 14/20 400, 12/16 400, mono 13/20, log 12/18) are asserted against `tokens-v2.json` by a test; radii 4/8/12; layout constants match; `assets/theme/synofurb-theme.json` is identical to `design/v2/synofurb-theme.json` (`cmp`); `tokens.rs` is generated and tested against the JSON; the text-scale factors are 1, 1.15, 1.30 on the rem.
* **Rendered contrast (measured, not eyeballed, light and dark):** body 18.97 / 17.2, muted `#525252` on canvas 7.49, status-bar chips 6.8 to 7.5, Demo tag 7.3 / 7.7, warning alert text 7.41 (light) / 11.42 (dark), Destroyed label 10.1 (dark), Changed 12.1 (dark), install "No return" 6.56 (dark), recovery tags 7.5 to 8.2 (dark), step text 8.4 to 17.2 (dark), way-back link 8.93 / 9.97. Disabled Erase label 2.14 (light) and 2.98 (dark): exempt and equal to the widget style (DEV-UI-2). Progress track vs page 1.37:1 with the percentage as text beside it (the DS alternative). Checkbox and input borders `#808080` (3.95:1).
* **Scrollbars themed:** 6 px thumb, 4 px inset, `#808080` light / `#868686` dark, transparent track, on every scrolling page (pixel scans of four shots).
* **Fonts:** Inter 400/500/600/700 and JetBrains Mono 400/700 registered at start-up from embedded bytes (`assets.rs:20-25`, OFL texts included); the app and the screenshots use them. The wordmark is outlined paths (a unit test forbids live text), so no Montserrat file is needed, consistent with the DS.
* **Lucide:** every icon comes from the embedded Lucide subset through `icons.rs` roles (apart from B5).
* **One primary per screen:** one on every shot (Goal toggles between Boot manager and Quick setup by `try_first`, Recovery follows `primary == Some(kind)`); Confirm's Erase is a danger button.
* **Text size:** confirm at 115 % and 130 %, plan at 115 %: the title measures 163, 187, 212 px wide (x1.00, x1.15, x1.30) and layouts reflow without clipping in those shots.
* **Spacing and alignment vs the mockup:** content left edge 26 px (2 px capture strip plus the 24 px page padding), right edge at 938, action-bar primary aligned to the content edge, scrollbar in the gutter. Heights differ slightly (unit header taller, stepper taller because the labels sit below the discs as the widget draws them).

## Deviations DEV-UI-9 to DEV-UI-14

| Id | Verdict | Reason |
|---|---|---|
| DEV-UI-9 Enter handling | **Acceptable** | A global `enter` binding would swallow Enter on every button; the implementation uses a root key handler that acts only when the root is focused (`render.rs:547`) and a `capture_key_down` wrapper on Erase (`confirm.rs:251-252`). Matches DEV-UI-8. I could not run it; the claim of XTest verification is the author's. |
| DEV-UI-10 ToggleGroup, not RadioGroup | **Acceptable choice, defective rendering** | See B2. |
| DEV-UI-11 Static countdowns | **Acceptable with a condition** | The text must read as a fixed rule ("DSM returns in 60 s"), never as a remaining time. `v2-app-failed-light.png` does this. The 15-minute rescue window is the one place a live clock matters; add it when a repaint timer exists. |
| DEV-UI-12 Recovery produces commands | **Acceptable as a limit, defective as worded** | See non-blocking 4: rename the buttons or drop the typed phrase, and fill the placeholders. |
| DEV-UI-13 Update simulated, Storage read-only | **Acceptable** | Matches the owner's decision (no RAID mutations on the desktop); `mgmt::update_gate` is unit-tested. |
| DEV-UI-14 Not built: keychain, host-key trust dialog, rescue shell | **Acceptable now, a gate blocker later** | The missing host-key trust dialog is a security feature (first-connection verification), not polish; the approved mockup has it (`connect-hostkey`). Done shows the fingerprint after the fact, which is not the same. |
| DEV-UI-15 | **Not found** | Add it, or correct the task text. Unrecorded deviations I found: B4, non-blocking 1, 3, 5, 6, 8. |

## Evidence matrix: which of the 14 screen families lack dark, 960, 1180 or Largest evidence

Existing: 27 PNGs, all 960x640 except `confirm-light-1180` and `plan-light-1180` (1180x760); "large" is 115 %, "largest" is 130 %.

| Family | Light 960 | Dark 960 | 1180x760 | 115 % | 130 % | Other states shown |
|---|---|---|---|---|---|---|
| Find (inventory) | yes | **missing** | missing | missing | missing | no empty, scanning, no-result, update banner |
| Connect and Inspect | yes (2 shots) | **missing** | missing | missing | missing | no failure alerts (login, unreachable, host changed), no host-key dialog |
| Goal | yes | **missing** | missing | missing | missing | no Quick setup dialog (radio choice), no Customize |
| Setup: Storage | yes | **missing** | yes (light) | yes (light) | missing | disk diagram never fully in frame; no Keep mode, hybrid, slices dialog |
| Setup: System | yes | **missing** | missing | missing | missing | no offline source, no error |
| Confirm | yes | yes | yes (light) | yes (light) | yes (light) | no enabled-Erase shot, no keep, no twin, no Copy command dialog |
| Install | yes | yes | missing | missing | missing | no log expanded, no point-of-no-return passed, no cancel dialog |
| Failed | yes (pre only) | **missing** | missing | missing | missing | no post, agent, backup variants; no rescue panel |
| Done | yes | **missing** | missing | missing | missing | no copied state, no Report dialog |
| Manage | yes | yes | missing | missing | missing | one hybrid storage shot; no degraded, update, updating, failed |
| Recovery | yes (agent, fresh) | yes (debian) | missing | missing | missing | no Manual recovery dialog, no typed-confirm dialog, no result |
| Boot manager | yes | **missing** | missing | missing | missing | no progress or result |
| Settings | yes | **missing** | missing | missing | missing | other pages (Updates, Data, Logs, Privacy) not shown |
| Help | yes (sheet over Find) | **missing** | missing | missing | missing | no topic, search or shortcuts view |
| About | yes | **missing** | missing | missing | missing | Credits closed |

Totals: dark exists for 4 of 14 families (Confirm, Install, Manage, Recovery), 1180 for 2, Largest for 1. Never shown anywhere: the log viewer (a whole screen), any dialog (only the Help sheet appears), the first-launch and update dialogs, keyboard focus ring on any control, hover or pressed states, a Tab-order walk, the Select popup, the Popover from a disabled reason (info buttons are drawn but never open), and any state at Largest on 960x640 other than Confirm.

## What is missing for a real gate

* **Windows and macOS screenshots cannot be produced here** (Linux, Xvfb and lavapipe only). The release gate needs them, because GPUI draws text and the title bar per platform (DirectWrite, Core Text, ClearType, native window controls, scrollbar overlay behaviour, high-DPI). Everything above is Linux software rendering; text in the captures has subpixel colour fringes that other platforms will not show.
* **Linux with a real window manager:** window controls, resize, minimum size 960x640 enforced; the 2 px black edge in every capture explained.
* **Matrix above:** at least dark and Largest at 960x640 for every family; every dialog and sheet; the log viewer with levels; failure variants (post, agent, backup); Quick setup with mixed disks.
* **Interaction evidence:** keyboard focus rings (2 px, offset 2) on checkbox, input, segmented choice, row, info button; the Tab order of Confirm; Escape and Enter behaviour on dialogs and Erase; the disabled-reason Popover opened by keyboard; hover and pressed on primary and danger buttons in both themes.
* **Behaviour:** an automated walk (not just screenshots) that proves no horizontal scroll or clipping at 960x640 and Largest on every screen, and that the unit tests for the DS tokens pass in CI.
* **Not verifiable by anyone here:** screen readers (the AccessKit tree exists in the Kit; nothing proves labels read well), IME and password managers, real SSH discovery and install.

## Recommended order of fixes

B1 and B2 (visible selection, one afternoon), B3 (route six buttons through `with_reason`), B4 (show the destroyed list), B5 (Lucide glyphs), then regenerate the screenshots for the whole matrix at both themes and both text extremes before asking for the gate.
