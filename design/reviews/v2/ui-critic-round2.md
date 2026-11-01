# UI critic, round 2: desktop mockup v2 (revised)

Artifact: `design/mockups-v2/desktop/` (index.html built from `src/app.css`, `js1-7`; I confirmed every source file is embedded verbatim in index.html and that the 378 screenshots (17:33-17:37) are newer than the build (17:29)). Criteria: `design/BRIEF-v2.md`, `design/v2/DESIGN-SYSTEM-V2.md`, `design/v2/tokens-v2.json`, gpui-component 0.8.1 source. I did not read other reviewers' files and did not edit the mockup. My round-1 report is `design/reviews/v2/ui-critic-round1.md`.

State URL prefix: `file:///home/user/synofurb/design/mockups-v2/desktop/index.html?bare=1&theme=<light|dark>&` + query from `screens.json` + `#<screen>`.

## Verdict: REJECT (one new blocking finding, NB1)

The author fixed all nine round-1 blocking findings, and I verified each against the files, not the claims. The prototype is now much closer to what the widgets will render. One new blocking issue remains: the tooltip behaviour in the mock misrepresents the widget in both directions, and the design system quietly dropped a binding brief rule to cope with it. It is a small, cheap fix; once NB1 is closed I expect to approve.

## Evidence base

* Viewed 26 of the 378 screenshots (plus my own live renders) in both themes, at 1280x800, 960x640 and 960x640 at 130% (find, find-loading, find-firstlaunch, connect-hostkey, goal, plan, plan-select-open, plan-960, confirm-ready (1280, 960, 960-130), confirm-partial, confirm-locked-reason, confirm-twin-a, install (1280, 960, 960-130), install-log, install-lost, failed-post-960-130, done, boot, recovery, recovery-remove, manage-health-faildisk, settings-largest-960, about-credits, help-topic, specimens, plus my own live renders).
* Live Chrome runs with scrollbars visible (`ignoreDefaultArgs: --hide-scrollbars`, the same way the author's `shots.js` now does it): computed styles on every text node of all 92 states at 1280x800, 960x640 and 960x640 at Largest (type scale, weights, scrollers, h-scroll, clipped text), Tab order, focus-trap in the Help sheet, interactive target sizes, per-screen primary and danger button counts, tooltip inventory.
* Contrast recomputed by me: 1,707 distinct rendered text samples over all states in both themes (composited over real ancestor backgrounds), plus token pairs from `tokens-v2.json` for non-text, the new dark danger set, all five web accent sets.
* Widget source read again: `stepper/item.rs` and `trigger.rs`, `tag.rs`, `alert.rs`, `button.rs` (`ButtonVariant::disabled`), `table/state.rs`, `tooltip.rs`, `gpui-base/src/scrollbar.rs`.
* Brand assets in `assets/brand/` (mark, app icon 16-1024, favicon, mono) looked at; all 73 inline icons compared against `gpui-kit-assets-0.8.1/assets/icons/*.svg`.

## Round-1 blocking findings: status

| # | Status | Evidence |
|---|---|---|
| **B1** themed scrollbars | **Fixed** | `shots.js` now keeps scrollbars; screenshots show a 6 px pill thumb, transparent track, no arrows (zoomed crop of `install-960-light`). `app.css` line 22: `scrollbar-width` is now inside `@supports not selector(::-webkit-scrollbar)` (Firefox only), so Chromium honours the pill rules. One scroller per screen: my probe finds exactly one scrolling element in every state at 1280x800 and 960x640; at 960x640 at 130% the only second scroller is the Help sheet over the page (modal, acceptable). No horizontal scroll or clipped text anywhere (0 hits in all 92 states at all three sizes); the log now wraps. Declared exception accepted, with a correction: the author says Chromium reserves 12 px; measured with `scrollbar-gutter: stable both-edges` it reserves 24 px (12 per side), which matches the 24 px content edge, so layouts are equivalent to GPUI's 16 px overlay. |
| **B2** text size no-op | **Fixed** | `html[data-size=large]` 18.4 px, `largest` 20.8 px; all lengths rem (only 1-2 px hairlines remain px). Largest: h1 24 to 31.2 px (x1.3). Five states are captured at 960x640 and 130% (find, plan, confirm-ready, install, failed-post); no clipping or h-scroll in any state at 130%. Usability caveat in N3. |
| **B3** 960x640 not at DS metrics | **Fixed** | No height media queries remain (only `prefers-reduced-motion`; container queries on width). All 92 states have `-960` images. `confirm-ready-960-dark/light`: the Destroyed / Changed / Preserved triad and the red "No backup" alert sit above the fold, and the Erase button is pinned. |
| **B4** stepper not the widget | **Fixed** (residual N1) | Now text-centred: 24 px disc (`secondary` upcoming, `primary` done and current), 14 px label below, 2 px `border` connector (`app.css` lines 43-53), matches `trigger.rs`/`item.rs`. The custom parts (check icon, semibold current label, `circle-x` failed) are declared in DS section 7 and are reasonable. Not Tab stops (my Tab run: Help, Settings, Menu, then page content). All six labels fit at 960 and at 130%. |
| **B5** disabled style | **Fixed** | Computed colours on `gate=part#confirm`: label `fg_2` at 50%, fill and border `danger` at 15% (light `color(srgb .7765 0 .0784 / .15)`), exactly `ButtonVariant::disabled()`. Reason is a visible info button next to the control (`confirm-locked-reason`), no tooltip on the disabled button. Residual in N6. |
| **B6** Tag and selected row | **Fixed** | Tag measured 91x25 px (padding 4/10, line-height 1.25, weight 400). Untested uses `border_strong` with the `circle-dashed` icon (custom border, as the DS now says; dashed borders are not used on tags). Selected row = inset 1 px border like `table/state.rs` plus a Radio in the first cell (`find-light`): selection no longer relies on colour. Radii are 4/8/12/pill only. |
| **B7** alerts with actions, native selects | **Fixed** | 0 `<select>` in any source file; custom listbox popup with a check mark on the selected item (`plan-select-open-dark`). Alerts with actions are `.alrow` (Alert plus sibling Button), as `alert.rs` allows. |
| **B8** type scale | **Fixed** | Over 92 states and 1,707 text nodes: sizes only 12, 13 (mono only), 14, 16, 24 (31.2 at Largest, 15.6 and 18.2 only in the Largest state); weights 400, 600, 700 only; no 13 px proportional text; no stroke-width overrides (one value, 2); `xl` icon is 24 px. |
| **B9** dark danger hover and pressed | **Fixed** | `tokens-v2.json` dark danger `#E84642`, hover `#ED6B68`, pressed `#F18C8A`, label `#150A09`. My numbers: label 4.99 / 6.40 / 8.21; steps base-to-hover 1.28, hover-to-pressed 1.28, base-to-pressed 1.65 (was 1.05 and 1.09); fill vs surface 4.60, canvas 5.08, `surface_2` 4.18, `err_bg` 3.61 (all above 3). `specimens-dark` shows the three states clearly different. `synofurb-theme.json` carries the new danger and label. |

## Round-1 non-blocking items: status

* **N1** alignment: **fixed**. Left edge x=24 on every screen (also Boot, Recovery, About, Settings), pinned action bar and content both end at x=1256; the Manage page no longer repeats the unit name. Small off-scale values remain (gap 6, padding 10, an 11 px connector offset).
* **N2** typed phrase: **not fixed**. `confirm-partial` still shows a red "No match" tag while the user is mid-typing ("ERASE DS9"). Use neutral "n of m" until a wrong character is typed. The tooltip overlapping the status bar is gone (popover).
* **N3** log: **fixed** (glyph plus word `INFO`, no clipping, no horizontal bar).
* **N4** Help: **fixed**, structured paragraphs, table for Recovery; heading sits flush under the Back button with no gap (cosmetic).
* **N5** fingerprint dialog: **fixed** (three clean lines, Copy button).
* **N6** destructive actions on Recovery: **partly**. "Remove entries" now opens a typed-confirmation dialog with a red top edge (`recovery-remove`), "Reinstall DSM" became "Open guide"; the rows still use identical neutral buttons, so nothing marks the destructive one before it is clicked.
* **N7** Done: **fixed** (copy icons beside values, Finish disabled with an info button giving the reason).
* **N8** logo and icons: **fixed** in the mockup (two-tone wordmark in the title bar, Montserrat credit removed, running glyph is `loader-circle`; 73 of 73 icons geometry-identical to the shipped Lucide set). Brand assets exist for 16 to 1024 px, favicon, mono; the mark reads at 48 px and above, at 16 px the mono mark is an unreadable blob (see N8 below).
* **N9** targets: **fixed** for tags (25 px) and buttons; remaining interactive elements below 24 px are widget-defined (slider track 20 px high, switch 36x20, checkbox label rows) and the mock chrome close (20 px).
* **N10** Settings: **fixed**. **N11** Tab stops: **fixed**.

## New blocking finding

### NB1 (Medium) Tooltips: the mock shows behaviour the widget does not have, hides what it does, and the design system silently changed a brief rule
* **Where:** `src/js2.js` line 16 (`focusin` shows a tooltip); 21 distinct tooltips on non-focusable elements (found by scanning every `[data-tip]` in all 92 states); `DESIGN-SYSTEM-V2.md` section 7 (Tooltip row).
* **Evidence:**
  * `tooltip.rs` has no focus path (no `focus` anywhere in the file), and the DS now says so: "hover only ... Anything needed to act or to stay safe is on a focusable trigger". BRIEF-v2's disclosure table says tooltips are for "hover + focus". There is no deviations register entry for this change (grep of all `design/**/*.md` finds none).
  * The mock still shows a tooltip on keyboard focus of title-bar buttons, tags and info buttons (my Tab run shows the `.tip` element appear on focus), so a reviewer sees a behaviour GPUI cannot produce.
  * 21 tooltips sit on plain `span`/`li`/`div` elements with no keyboard route, and several carry safety-relevant sentences, contradicting the DS rule above: Done password "It is not stored or shown again." (`done`, `done-copied`); Failed "Then DSM becomes the default and the NAS reboots." (`failed-stopped`, `failed-pre`); Boot "The agent reboots to DSM after 30 minutes without login" (`boot`, `boot-confirm`); Confirm "Wipe mode preserves nothing on the disks." and "Entries and files are added; the whole flash is backed up first" (`confirm`, `confirm-partial`); Update dialog "Never offered while an install is running." (`find-update`); Connect "DSM asks for the password to read disk details."
* **What would convince me:** (1) remove the focus tooltip from the mock, or specify a custom focus tooltip and cost it; (2) move every sentence a keyboard user needs to a focusable trigger (an info button opening a Popover, as the Plan and Done screens already do elsewhere) or make it visible text; leave only decorative hints as hover tooltips; (3) record the hover-only limit as a deviation from BRIEF-v2 in the brief or DS so it is a decision, not a silent edit.

## Non-blocking items

* **N1 (fidelity) Stepper connectors for completed steps** are painted `border` in the mock; `StepperSeparator` paints them `primary` when passed (`item.rs`: `.when(self.checked, |this| this.bg(primary))`). DS section 7 also omits this. Fix the mock and the DS text.
* **N2 Gate:** red "No match" while typing (see above). The ERASE card is a red-bordered, red-headed block; the red button is still the strongest element in dark and light (4 red cues on the screen, within the DS rule).
* **N3 130% usability:** Plan at 130% and 960x640 has a 331 px working area and needs 1,346 px of scrolling; its first screen shows only "Mode". Confirm needs scrolling to reach the checkboxes and typed phrase (the Erase button is pinned). Acceptable for a minimum-size, maximum-text corner, but consider hiding the unit meta strip at 130%.
* **N4 One primary per screen** is broken on non-dialog screens: `done` ("Copy password" and the disabled "Finish"), `manage-health-faildisk` (the header "Open web UI" plus the failed-disk row's "Open web UI", and a third neutral one on md1), `manage-health-update-degraded`. The author's declared "3-word button label" ("Open web UI") is compliant (a verb and two words), but repeating it on three rows that are about a failed disk or array is a copy error, not a label-length question.
* **N5 Help sheet over page:** the unit-header `Untested` tag is left undimmed and bright above the scrim in `help`, `help-topic` and also under dialogs (`connect-hostkey`); the author declared it for the sheet only. Fix the stacking (the real overlay covers everything).
* **N6 Disabled Erase** stays a Tab stop in the mock (my Tab run: "Why locked", then the disabled Erase button focusable); DS says a disabled Button is not focusable. Make the mock consistent with the widget rule.
* **N7 Select popup** opens upward over its own "Mode" label in `plan-select-open` at 1280x800; cosmetic, but check against the widget's popup placement.
* **N8 Brand:** at 16 px the mono mark is a blob and the colour icon is barely readable; an `icon_16` artwork should drop the central unit glyph. The app icon's arrow heads read slightly inconsistent.
* **N9 Token duplication:** `accent.purple.*` (e.g. `accent_fg #5E1EB3`, hover `#6327BA`, pressed `#560EA9`) differs from `color.light.accent_fg #5C29AA`, hover `#622CB5`, pressed `#541EA0`; two sources of truth for the default accent. All five accent sets pass my recomputation (label 5.0-6.67 light, 6.3-6.6 dark; ring on surface at least 5.0 light, 5.8 dark).
* **N10 DS housekeeping:** DS section 12 "Open" list still names "dark danger hover/pressed subtlety" (resolved); the contrast gate it cites (`design/mockups/ui/tools/contrast_v2.py`) lives in the withdrawn folder (it still passes: 642 of 642, lowest text 4.74, non-text 3.25, I ran it). Move it.
* **N11 Container queries** (`@container win`, 52 rem and 68.75 rem) switch the layout to one column at large text sizes; GPUI has no equivalent, so the real app needs explicit layout rules. Document them.

## Declared exceptions: my ruling

| Declared by author | Ruling |
|---|---|
| Chromium scrollbar reserve vs GPUI 16 px overlay | Accepted (measured 12 px per side; layouts equal). |
| Stepper custom parts (check icon, semibold current label, `circle-x`) | Accepted, they are children the widget lets us add. Fix the passed-connector colour (N1). |
| Help sheet keeps the Tag of the underlying screen | Rejected as a defect, not an exception (N5); it also happens under dialogs. |
| Dark danger `#E84642`, near-black label | Accepted, numbers verified (B9). |
| "Open web UI" 3-word button | Compliant with the text budget; the real problem is copy and repetition (N4). |

## Doc and token edits made outside `mockups-v2`: weakening check

* **Not weakening:** Tag Untested border strengthened to `border_strong`; scrollbar CSS corrected; Alert "no action slot"; Table selection rule now honest about 2.2-2.8:1 with a Radio as the non-colour cue; disabled style aligned with the widget and compensated by a visible reason button; Stepper reduced to six steps with its widget limits stated; type and radius sets cleaned; dark danger upgraded; contrast gate updated and still green.
* **Weakening, to be recorded:** Tooltip downgraded to hover-only against the brief (NB1); disabled-control label legibility accepted at about 2.2-2.9:1 (WCAG-exempt, mitigated by the visible info button; fine, but state it in the register).
* **Stale or inaccurate:** N1, N9, N10 above.

## What I could not verify

GPUI rendering itself (no real-app screenshots); `Select` popup placement; `Tag::custom` plus `circle-dashed` in a real build; `tnum` font feature exposure; Windows/macOS/Linux title bars; the web surface; the published Artifact URL (I reviewed the files and screenshots, not the hosted copy).

## What would convince me to approve

1. NB1 closed as described (focusable carriers for every safety sentence, no focus tooltip in the mock, deviation recorded).
2. N2 (neutral progress feedback while typing the phrase) and N1 (passed connector colour) fixed, so the gate screen and the stepper match the widgets.
3. One primary per screen on Done and Manage, with correct row actions on a failed disk or array.
