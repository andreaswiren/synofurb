# UI critic, round 3: desktop mockup v2 (revised again)

Artifact: `design/mockups-v2/desktop/` (104 states, `index.html` built 18:38 from `src/app.css` and `js1-7`; I confirmed each source file is embedded verbatim in `index.html`). Criteria: `design/BRIEF-v2.md`, `design/v2/DESIGN-SYSTEM-V2.md` (now with section 13, deviations register DEV-UI-1..8), `design/v2/tokens-v2.json`, gpui-component 0.8.1 source. I did not read other reviewers' files and did not edit the mockup. Earlier reports of mine: `ui-critic-round1.md`, `ui-critic-round2.md`.

State URL prefix: `file:///home/user/synofurb/design/mockups-v2/desktop/index.html?bare=1&theme=<light|dark>&` + query from `screens.json` + `#<screen>`.

## Verdict: REJECT (one new blocking finding, NB-R3-1)

NB1 from round 2 is genuinely fixed, and so are all of my round-2 non-blocking items that I could reproduce. The new Largest-text layout of the data-loss screen introduces a safety-ordering regression: at 130% text the consent controls come before the destroyed / changed / preserved summary, and the Erase button can be enabled without that summary ever having been on screen. It is a one-rule layout change. After it I expect to approve.

## Evidence base

* Viewed 24 shipped screenshots (confirm-ready 1280 / 960 / 960-130, confirm-keep 1280 / 960, find-none, goal, plan, boot, recovery, help-topic, failed states, done, manage, install and others) and rendered 27 more states live in Chrome with scrollbars visible, in both themes at 1280x800 and 960x640, and at 960x640 with Largest text. Shipped screenshots and live renders are pixel-identical for the five states I diffed (384 of 426 files predate the final `index.html` timestamp; see N5).
* Live probes over all 104 states: type scale and weights (text nodes), scrollers, horizontal scroll and clipped text at 1280, 960 and 960 Largest; tooltip inventory; Tab order with the tooltip element watched; popover keyboard behaviour; interactive target sizes; primary and danger button counts per screen; word counts; dialog and popover word counts.
* Contrast recomputed by me over 1,769 rendered text samples in both themes (composited over real ancestor backgrounds) and from `tokens-v2.json` for the new dark danger set and the five accent sets; I ran `design/v2/tools/contrast_v2.py` and `check_tips.js` / `check_scale.js` (both pass).
* Widget source for Stepper, Tag, Alert, Button, Table, Tooltip, Scrollbar re-read where relevant.

## Round-2 findings: status

### NB1 (tooltips): fixed
* **Tooltips only on focusable controls.** My own scan of every `[data-tip]` in all 104 states finds 0 on non-focusable elements (round 2: 21 distinct); `check_tips.js` reports 987 tooltips, 0 offenders. The sentences I named have moved to visible text or info buttons: the password sentence now sits behind the info button on the Done "Password" row, "DSM becomes the default" is in the failed-state card info (`clockInfo`) and the "What happens next" dialog, the 30-minute agent timeout is on a label info button and a "30 min watchdog" tag (a Popover trigger), sudo and host-key hints are info buttons.
* **No tooltip on keyboard focus.** Tabbing 14 stops through `unit=ds918&gate=part#confirm` raised 0 tooltips; hover still raises them on title-bar buttons (1 shown).
* **Keyboard path to the content.** An info button opens its Popover with Enter ("Backup": two sentences plus Open Help), Escape closes it and focus returns to the trigger (`About backups`).
* **Deviation recorded.** DEV-UI-1 states the hover-only limit against BRIEF-v2's "hover + focus" and names the enforcement. Accepted.

### Round-2 non-blocking items

| Item | Status | Evidence |
|---|---|---|
| N1 stepper connectors | **Fixed** | Completed connectors are `primary` (`confirm-*`, `failed-*` screenshots), as `StepperSeparator` does; DS section 12 now says so. |
| N2 "No match" while typing | **Fixed** | Typing "E", "ERASE D", "ERASE DS918+ 34": neutral "1 of 17", "7 of 17", "15 of 17"; "Matches" only on equality; "No match" only on a full-length wrong phrase (live, `gate=part`). |
| N3 130% usability | **Partly** | At Largest the unit header is hidden on Confirm and alerts shrink to the `Size::XSmall` metrics (px 12, py 6), which buys 70 px; Plan and Confirm still need 2-3 screens of scrolling. Acceptable; the ordering side effect is NB-R3-1. |
| N4 one primary per screen | **Fixed** | Per-screen count of `.btn.primary` over 104 states: only dialog states have two (the page beneath plus the dialog's own). Done: "Copy password" is now neutral. Manage health: one "Open web UI"; row actions are now text links "Replace sdc" and "Repair md1" with an external-link icon. |
| N5 header Tag above the scrim | **Withdrawn** | Not reproducible: `elementFromPoint` over the unit-header Tag with the Help sheet open returns the scrim; the shipped `help-topic` crop is uniformly dimmed. My round-2 impression came from a lighter-looking pill, not a stacking bug. |
| N6 disabled Erase a Tab stop | **Fixed** | `tabindex=-1`, `aria-disabled=true`; Tab order reaches "Why locked" (focusable) and skips the button (DEV-UI-2). |
| N7 Select popup over its own label | **Fixed** | Data RAID is the first control of the panel; the popup opens downward over empty space (`plan-select-open-light`). |
| N8 brand mark at 16 px | **Fixed** | `assets/brand/` now has arrows-only `*-small` artwork for 16 to 48 px (icon, mono, colour mark, favicon). At 16 px the colour and mono marks read as the recycling symbol; the 512 px mark keeps the NAS block. |
| N9 accent source of truth | **Fixed** | `accent.purple.*` equals `color.*` for accent, hover, pressed, `accent_fg`, `accent_bg` in both themes (script comparison). The five accent sets still pass (label 5.0 to 6.7:1). |
| N10 DS housekeeping | **Fixed** | Contrast gate moved to `design/v2/tools/contrast_v2.py` and the DS cites it; run result 642 of 642, lowest text 4.74, non-text 3.25. The stale "danger hover/pressed" open item is gone. |
| N11 container queries documented | **Fixed with a gap** | DEV-UI-5 documents the rem breakpoints; it omits two Confirm rules visible in `app.css` lines 326-327 (unit header hidden at or below 52 rem; alert compaction). Add them. |

### Earlier fixes still hold (spot re-verified)
Type scale on 104 states and 1,825 text nodes: sizes 12, 13 mono, 14, 16, 24 only (31.2, 18.2, 15.6 px appear only in the Largest state); weights 400, 600, 700; no scroller other than `.main` (plus the Help sheet and dev menu overlays at Largest); no horizontal scroll or clipped text at 1280, 960 and 960 Largest; interactive targets all at least 24 px except the mock-chrome close (20 px), slider tracks and the Web UI switch (widget sizes); lowest enabled rendered text contrast 4.99:1 (dark Erase label); disabled buttons use the widget style; Tag 25 px; selected row radio; no native `<select>`.

## New blocking finding

### NB-R3-1 (Medium): at 52 rem or narrower (Largest text on a 960 window) the Confirm screen puts the consent controls above the destroyed / changed / preserved summary
* **Where:** `?size=largest&unit=ds918&gate=ready#confirm` at 960x640 (also `confirm-ready-960-130-{light,dark}.png`, `confirm-keep` and `confirm-twin-*` at Largest); `app.css` lines 323-327 (`grid-template-areas: "a" "g" "r"`).
* **Evidence:** at Largest the order is alerts, then the ERASE card (three checkboxes and the typed phrase), then the Destroyed / Changed / Preserved triad, then the list. Measured in the scroller: the gate starts 181 px from the top, the triad starts 602 px down, the main region is 403 px high. After a scroll of about 330 px the three checkboxes and the matching phrase are on screen with the Erase button pinned and enabled (my screenshot `gateL-ready-330`), while the triad (and "Preserved: Nothing") has never been on screen. At 100% (`confirm-ready-960-light`) the two-column layout shows the triad and the gate together, which is the right design. DS section 1 rule 4 says the destroyed / preserved content stays visible; round 2 accepted this screen because at Largest the triad sat above the gate.
* **Why it matters:** this is the one screen the product exists for, and Largest text is the case for people who most need it. DEV-UI-5's parenthesis "(Confirm order: alerts, gate, summary)" records it as a decision, but it reverses the reading order of "what will be lost" before "I accept".
* **What would convince me:** in the single-column layout order the areas alerts, triad (three compact cards, about 90 px), gate, list; or keep the triad pinned above the gate at every width. Add a screenshot at 960x640 Largest showing the triad above the first checkbox. Also add the two omitted rules to DEV-UI-5 (unit header hidden on Confirm, alert compaction).

## Declared exceptions: my ruling

| Exception | Ruling |
|---|---|
| **DEV-UI-3**, info buttons and inline links 24 px | **Accepted.** 24 x 24 is exactly the WCAG 2.2 AA 2.5.8 minimum and I measured no interactive control in 104 states below 24 px apart from mock chrome and widget-defined sliders and switch. One correction: the register says "the brief's 32 px minimum"; BRIEF-v2 contains no such number (32 px is the DS's own default control height). Reword to "the DS default control height". |
| **DEV-UI-5**, container-query layout in rem | **Accepted in principle**, needed because GPUI has no media queries, and it replaces the arbitrary viewport-height rules I rejected in round 1. Conditions: NB-R3-1 and the two undocumented Confirm rules. |
| DEV-UI-2, disabled style and reason button | Accepted (verified: widget style, reason on a focusable info button, button not a Tab stop). |
| DEV-UI-7, dark danger `#E84642` with `#150A09` | Accepted (4.99:1 label; hover and pressed 1.28:1 steps; fill vs surface 4.60). |
| DEV-UI-8, Enter does not activate Erase | Accepted as a deliberate guard, but it deviates from the widget's activation; the real app needs an explicit key interceptor, as the register says. |

## New states: regression check

Looked at confirm-keep, confirm-twin-keep, failed-agent (+ waiting / notback), failed-backup, failed-post-waiting-keep, boot manager and boot-returned, find-boot-agent, quick-mixed (Quick setup dialog), cancel-too-late, system-ds1621, goal-quick. No overflow, clipping or contrast failure; type scale and tag sizes hold; the Quick setup dialog has three buttons with one primary ("Continue to Confirm"); the failed states always carry the three-card state (Disks, Boot flash, DSM) with word plus icon status. Only the Confirm ordering above is blocking. Non-blocking observations below.

## Non-blocking items

* **N1 Duplicate action:** `failed-agent-notback` shows "Check again" twice (neutral button in the alert row and primary in the action bar). Keep one.
* **N2 Tag crowding in the inventory:** with a twin or boot agent (`find-boot-agent`), two tags sit between the model/serial and the address column and push the cell width; consider a single "Twin" marker plus the row's info.
* **N3 DS diagram exception:** section 12 still says the disk diagram "scrolls horizontally inside its card below 640 px"; BRIEF-v2 forbids horizontal scrolling. Not triggered in any state (no h-scroll at 960 Largest), but make the exception wrap instead.
* **N4 Decorative primary on Reinstall guide:** Recovery shows the single violet button on "Open guide" for the riskiest option; consider a neutral button and a primary only on the safe default; every row also repeats its label as the button text ("Open agent" / "Open agent"), which wastes the text budget.
* **N5 Screenshot freshness:** 384 of 426 PNGs have older timestamps than the final `index.html` (18:38). The five I diffed are pixel-identical to a fresh render, so this is process only: re-run `shots.js all` before approval so the evidence matches the build.
* **N6 Slider and switch targets** (20 px high) are widget sizes; WCAG 2.5.8 is met through the spacing exception, but record it in DEV-UI-3.
* **N7 Real-app gap (not a defect of this mock):** none of this has been rendered by gpui-component. Round-3 approval would still be an approval of the design, not of the build.

## Contrast (recomputed)

* Rendered text, 1,769 samples, both themes: none below 4.5:1 for enabled controls; lowest 4.99:1 (dark Erase label), next 5.13 (pressed primary specimen); disabled buttons follow the widget style (about 2.2 to 2.9:1, exempt, reason on a visible info button).
* Tokens (unchanged from round 2 unless noted): dark danger fill vs surface 4.60, canvas 5.08, `surface_2` 4.18, `err_bg` 3.61; hover and pressed label 6.40 and 8.21; border-strong on surfaces 3.25 to 5.44; scrollbar thumb 3.95 / 4.92 on surface.
* `design/v2/tools/contrast_v2.py`: 642 of 642 pass; lowest text 4.74, lowest non-text 3.25.

## What I could not verify

GPUI rendering of any of this; hover and pressed states live (the Components page shows them as specimens); the hosted Artifact copy; a real `Select` popup placement; `tnum` exposure in the pinned GPUI.

## What would convince me to approve

1. NB-R3-1 closed: triad before the gate in the single-column layout (or pinned), screenshot at 960x640 Largest, DEV-UI-5 updated with the unit-header and alert rules.
2. DEV-UI-3 wording corrected.
3. Fresh `shots.js all` so the screenshots are the final build.
