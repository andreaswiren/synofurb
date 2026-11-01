# UI critic, round 1: desktop mockup v2

Artifact: `design/mockups-v2/desktop/index.html` (prototype, not the real app). Criteria: `design/BRIEF-v2.md` (binding), `design/v2/DESIGN-SYSTEM-V2.md`, `design/v2/tokens-v2.json`, gpui-component 0.8.1 source (`~/.cargo/registry/src/*/gpui-component-0.8.1`, `gpui-base-0.8.1`, `gpui-kit-assets-0.8.1`). I did not read any other reviewer's file.

State URL prefix used below: `file:///home/user/synofurb/design/mockups-v2/desktop/index.html?bare=1&theme=<light|dark>&` followed by the query shown (from `screens.json`) and `#<screen>`.

## Verdict: REJECT

Nine blocking findings. The visual direction is good (calm, scannable, Lucide-exact, Inter and JetBrains Mono bundled and loading, text contrast solid) but the artifact does not yet show what the real application will render, and three of the owner's four complaints (themed scrollbars, widgets used properly, scalable/960x640 behaviour) are not demonstrated or are contradicted by the prototype itself.

## Evidence base

* Looked at 30 of the 118 supplied screenshots in both themes (find, find-loading, find-firstlaunch, connect-hostkey, inspect, goal, plan, plan-keep-error, plan-slices, popover-raid, system, confirm-partial, confirm-ready (1280 and 960), tooltip-erase, install (1280 and 960), install-log, failed-post (1280 and 960), done, boot, recovery, manage-health-faildisk, settings, about, about-credits, help-topic).
* Re-rendered live in Chrome with Playwright, 1280x800 and 960x640, both themes, with the scrollbar suppression removed (see B1), at 3x for the logo and stepper, plus keyboard Tab runs on `unit=ds918&gate=part#confirm`.
* Recomputed every contrast figure myself: (a) 1,335 rendered text nodes across all 55 states in both themes, foreground composited over the real ancestor backgrounds; (b) the token pairs from `tokens-v2.json` for non-text (borders, ring, scrollbar thumb, switch, progress, tag and alert borders, selected row, hover/pressed).
* Compared the mock's CSS with the widget source for Stepper, Tag, Alert, Button (disabled style), Table (selected row), Scrollbar, Select.

## Blocking findings

### B1 (High) Themed scrollbars are not demonstrated, and what Chrome really draws is not the spec
* **Where:** all 118 screenshots; live at `unit=ds918&step=12#install` at 960x640 (page plus Steps list scrollbars) and `unit=ds918&step=12&acc=ilog#install` at 1280x800 (log panel).
* **Evidence:** `shots.js` uses Playwright's default headless args, which include `--hide-scrollbars`, so no screenshot shows a scrollbar at all. With the flag removed, the `*{scrollbar-width:thin;scrollbar-color:...}` rule (app.css line 17) makes Chrome ignore the `::-webkit-scrollbar` pill rules below it (lines 18-20). The result is the native thin bar with arrow buttons and a 10 px reserved gutter (measured `offsetWidth-clientWidth` = 10), not the specified 6/8 px pill thumb with no arrows and a 16 px overlay hit area. The 10 px gutter also shifts the content 5 px left versus every supplied screenshot (title x=75, not 80). On Install there are two vertical bars side by side (page `main` plus `.stp-scroll`), and the log box has a horizontal bar (`logbox sw=540 > 516`). That breaks two stated rules: "one scroll container per screen" (DS section 6) and "no horizontal scrolling" (brief). The same horizontal scroll appears in the Help sheet, `nofirst=1&sheet=help&htab=Shortcuts` at 960x640 (`sheet-b 529 > 479`).
* **Convince me:** screenshots with scrollbars visible (idle, hover, dragging) in both themes on a table, the log, the Steps list and the Help sheet, taken from the real GPUI build; if the web UI uses CSS, a test in Chromium and Firefox that the pill actually renders. Remove nested scrollers and the log's horizontal scroll (wrap lines or give the log a copy/save affordance instead).

### B2 (High) The text-size setting does nothing, so "scalable text" and "130% at 960x640" are unverifiable
* **Where:** `#settings`, then Text size > Largest (also `settings`, `settings-data`).
* **Evidence:** `src/js6.js:72` maps all three choices to the empty string (`{Default:'',Large:'',Largest:''}`); I clicked Largest at 960x640 and `h1` stayed at 22 px, root font size unchanged. All sizes are `px`, not `rem`, so even a working setting would not scale. The brief requires scalable text and the DS requires layouts checked at 130% on 960x640.
* **Convince me:** the three sizes actually applied (all type tokens multiplied), and screenshots of Find, Plan, Confirm, Install, Failed at 130% on 960x640 with nothing clipped and the action bar and the erase gate fully usable.

### B3 (High) The 960x640 screenshots are not at the design-system metrics, and the safety lists fall below the fold
* **Where:** `confirm-ready`, `plan`, `install`, `failed-post`, `manage-health-rebuild` at 960x640 (the only six states captured at 960); `unit=ds918&gate=ready#confirm` at 960x640.
* **Evidence:** `app.css` line 244 `@media(max-height:720px)` silently compacts the layout: stepper 56 to 46 px (DS: 48), action bar 56 (DS: 64), title bar 36 (DS: 38), page title 24 to 22 px, unit name 18 to 16 px, unit meta hidden, content padding 24 to 14 px, gap 12. None of this is in the DS and GPUI has no media queries, so the real 960x640 will be more cramped than the shots. Even compacted, Confirm scrolls (`main` 530/420 px): "Changed" is cut mid-row and "Preserved" is not visible at all, so the destroyed / changed / preserved triad that the brief calls safety-critical cannot be seen together at the minimum window size, with no visible cue that more exists. Plan at 960 clips the left panel at "Usable" and cuts the summary row under the action bar. For the other 49 states (system 550/420, inspect 535/420, done 490/420, manage 776/466 and so on) no 960 image exists.
* **Convince me:** the DS compaction rule written down (or removed), then every state at 960x640 at exact DS metrics, showing Destroyed, Changed and Preserved together on Confirm (a compact summary strip above the fold is acceptable).

### B4 (High) The stepper, the most visible element on every screen, is not the `Stepper` widget
* **Where:** every wizard screen (e.g. `find`, `inspect`, `install`, `failed-post`).
* **Evidence:** `gpui-component-0.8.1/src/stepper/{trigger,item}.rs`: the indicator is a filled disc (`secondary` for upcoming, `primary` for every step `<= checked_step`, which includes the current one), with the label in a vertical flex below the disc at `text_sm` (14 px), and the connector is 2 px in the `border` token (#E5E5E5 / #2A2A2A, 1.26:1 on surface). The mock draws hollow 2 px rings in `border-strong` (#808080), a 3 px glow around the current step, 13 px labels beside the disc, and connectors in `border-strong` (`app.css` lines 33-40). In the real widget, current and done are the same colour, so "current = ring plus label" (DS section 7) cannot be built without a custom item. Under 1100 px the mock hides labels except the current one, but the widget has no such mode. Neither the custom stepper nor the failed state is listed in DS section 12 exceptions.
* **Convince me:** either render the real `Stepper` (screenshot from the GPUI build) and adjust the DS to what it does, or list a justified exception and state exactly which parts are custom.

### B5 (High) Disabled and gate states do not look like the widgets' disabled style
* **Where:** `unit=ds918&gate=part#confirm` (Erase and install), `plan-keep-error` (Next: System), `find` (Connect before selection), `done` (Finish), `install` (Cancel).
* **Evidence:** `button.rs` `ButtonVariant::disabled()`: label `muted_foreground` at 50% opacity, fill the variant colour at 15%, border the variant colour at 15%. I computed the resulting label contrast at about 2.2:1 light and 2.8-2.9:1 dark for Danger and Primary. The mock draws a 1 px dashed `border-strong` outline, `surface_2` fill and an `fg_2` label (7.0:1) (`app.css` lines 67-68, 101). The real disabled Erase button is therefore a pale red tint with a barely legible label, not a legible dashed grey one. The DS never says dashed, and the widget does not do it. The mock also shows a tooltip on hover over the disabled button (`tooltip-erase`); that the widget delivers hover events when disabled is unverified, and DS section 7 itself says disabled controls should carry an info icon instead.
* **Convince me:** the disabled style taken from the widget (or a new theme-level decision with its legibility numbers), and a gate screenshot that shows how a user learns why Erase is disabled without hover.

### B6 (Medium) Tag and Table selection: the mock is more visible than the widgets will be
* **Where:** `find` (selected row, Untested tags), `inspect`, `plan`, `manage-*` (all tags).
* **Evidence:**
  * Tag (`tag.rs`): Medium is `text_xs`, line height 1.25 (15 px), `py_1` (4 px each), `px_2p5` (10 px) and a 1 px border, about 25 px high. The mock tag is 22 px high with 8 px padding (`.tag`, `app.css` line 73), weight 500 (the widget sets no weight), so every tag in the real app is 3 px taller and about 4 px wider than shown. `Secondary` outline draws its border in the `border` token (1.26:1 on surface), so the "Untested" tag, the most frequent badge, will have an almost invisible boundary in the real app. The mock draws it with `border-strong` (3.95:1) and a dashed line. The icon plus the word still carry the meaning, but the shape cue the DS relies on is not there.
  * Selected row (`table/state.rs` around lines 2311 and 2421): the widget paints the `table_active` fill plus a 1 px border at 50% of `table_active_border` all around the row. Recomputed: that edge is 2.2:1 against the row and 2.6-2.8:1 against the surface, and the tint is 1.15:1 (light) / 1.28:1 (dark) against the surface. This fails the 3:1 non-text rule. The mock draws a solid 3 px left bar (`inset 3px`), 5.8:1, which the widget cannot do (DS section 7 claims "3 px table.active.border"; the source says otherwise).
* **Convince me:** the tag and selected-row rendering from the real widgets, and a decision (custom row with a left bar, or a stronger theme value) with numbers.

### B7 (Medium) Alerts with actions, and four native `<select>` elements
* **Where:** `plan-keep-error` ("Too large" alert with "Set 9 GiB"), `manage-health-failed` (Retry), `connect` with SSH off ("How to enable"), `system` with offline source ("Download"); selects at `src/js4.js` lines 17, 34, 109, 110 (Data RAID, Time zone, Auto-update, Offline bundle), visible in `plan`, `system`, `settings`.
* **Evidence:** `alert.rs` has `message: Text`, an optional title (stacked above the message, truncated), and only a close button; there is no action slot. The mock puts inline bold titles and buttons inside the alert. The DS lists no exception for "alert plus actions". The four native selects use the OS popup, which cannot be themed and is not what `Select` renders in GPUI; no screenshot shows an open select, so its popup, hover, selected and keyboard states are unspecified. The brief explicitly asked to flag native selects.
* **Convince me:** an alert-plus-actions pattern built from real widgets (Alert with the button as a sibling), and the open `Select` popup in both themes.

### B8 (Medium) The type scale in the mock is not the DS type scale
* **Evidence (live computed styles over all 55 states):** the DS has six styles (24, 16, 14, 12, mono 13, log 12). The mock also uses 13 px proportional text (stepper labels, table headers; 583 + 219 text nodes; the widgets draw these at `text_sm` 14), 18 px (unit name, goal card titles, `connect` and `goal` pages, 46 nodes), 28 px and 36 px mono (rescue countdown, install percentage), 22 px and 16 px under the compaction rule (B3), weight 500 on 376 tag/link nodes although only 400, 600 and 700 are bundled (Chrome falls back to 400, so the displayed weight differs from the declared one), and six 6 px radii where the DS has 4, 8, 12 and pill. Stroke-width overrides (3.5 on step check glyphs, 1.2 on window controls) cannot be reproduced by GPUI because the stroke is baked into the SVG file; an `ic.xl` 28 px size is outside 14/16/20/24. The 12 px floor is respected everywhere (no text under 12 px found).
* **Convince me:** every text node maps to a DS style; either add 18, 28 and 36 px styles to the DS (and check the widgets can do it) or change the screens.

### B9 (Medium) The destructive button gives no visible hover or pressed feedback in dark
* **Where:** `confirm-ready`, dark.
* **Evidence:** `tokens-v2.json` dark `danger` #CE292C, `danger_hover` #CA2429, `danger_pressed` #C71F26: ratio base to hover 1.05:1, base to pressed 1.09:1 (light: 1.23 and 1.50). The DS admits "hover and pressed differ by small steps in dark (use the focus ring and the label change for loading)". The button that erases four disks has no perceivable press feedback in dark.
* **Convince me:** a hover and pressed pair that is visible (at least a clearly perceptible step) while keeping the label at 4.5:1, or a documented additional cue, shown in a screenshot of both states.

## Non-blocking findings

* **N1 Alignment and rhythm.** Four left edges on every wizard screen: stepper and unit tile at x=24, action bar Back text at x=38, page content at x=80. The primary action ends at x=1256 while tables and cards end at x=1200. Narrow centred pages use other anchors: Boot manager and About at x=360, Recovery at x=260, Settings at x=80 (`boot`, `recovery`, `about`, `settings`). Spacing values are mostly on the DS scale (off-scale: gap 10 x9, padding 14, margin 18, heights 22, 26, 30, 34). The management page repeats the unit name twice in 60 px (`manage-health-ok`: unit header "DS918+" plus h1 "DS918+").
* **N2 Gate details (confirm).** Red appears four times (destroyed edge, ERASE card border and header, "No match", the button), which matches the DS rule, but the ERASE card is a 2 px red frame that is louder than the button in light. `gate=part` shows "No match" in red while the user is still typing ("ERASE DS9"): error colour before an actual error. The tooltip on the disabled Erase button overlaps the "Way back" link in the status bar (`tip=%23f-erase`). Checkboxes are 16 px (hit area is label-wide; fine).
* **N3 Log.** In `install-log` levels are shown by colour alone (blue, amber), not glyph plus word as the DS requires; the last line is hard-clipped with no ellipsis ("(about 9"). `install` collapsed shows a clipped line (`CLIP mono sub 526>350` at 960).
* **N4 Help.** `sheet=help&topic=hybrid-raid`: one 115-word paragraph, no structure, ambiguous "More" link. Brief asks for short topics.
* **N5 Dialog.** `connect-hostkey`: the 64-char fingerprint wraps leaving an orphan "c" on its own line and has no Copy button (DS: give copyable cells a copy button). The "mockup" chip floats above the scrim.
* **N6 Recovery.** `recovery`: "Remove our entries" and "Reinstall DSM" have the same neutral Run button as "Open agent"; destructive actions are not marked at the button level.
* **N7 Done.** Copy icons sit about 900 px from the values they copy (`done`); the Finish button is disabled until an unrelated "Password copied" checkbox is ticked, with no stated reason except hover.
* **N8 Logo and icons.** Mark reads at 16 px (title bar) in both themes; recycle-arrows plus a small unit glyph. The title-bar wordmark is single-colour while About is two-tone (`syno` ink, `furb` accent). Credits list Montserrat although the DS says no Montserrat file ships. App-icon sizes and favicon (brief item 7) are not part of this prototype and I could not review them. All 66 icons used are genuine Lucide: I compared each symbol's geometry with `gpui-kit-assets-0.8.1/assets/icons/*.svg` and every one matches; none is missing from the shipped set. The "Installing" tag uses `layers` instead of the spec's running glyph (`loader-circle`).
* **N9 Targets.** Tags used as buttons are 22 px high; checkboxes 16 px; step buttons 32 px high. Mostly acceptable under WCAG 2.2 target-size spacing, but tags at 22 px are under 24.
* **N10 Settings.** General page leaves about 70 px of empty card below the last row.
* **N11 Keyboard.** Six stepper buttons are tab stops before page content; focus ring itself is good (2 px, offset 2, visible on checkboxes and links in both themes).

## Contrast (recomputed)

* **Rendered text, all 55 states, both themes:** no text node below 4.5:1. Lowest: dark destructive label 5.28:1, dark primary label 6.30 (6.12 mid-animation on first-launch), light destructive label 6.16. Matches the DS claims.
* **Tokens, non-text (light / dark):** `border_strong` on surface 3.95 / 4.92, on `surface_3` 3.25 / 3.89, on `accent_bg` 3.44 / 3.84; scrollbar thumb on surface 3.95 / 4.92 (hover vs log 3.38 / 8.62); ring on surface 6.67 / 5.82 (lowest 4.46 on `warn_bg` dark); switch off 3.95 / 4.92; progress fill vs its track 4.86 / 4.34. Pass.
* **Fail or weak:** progress track vs surface 1.37 / 1.34 (a 0% bar has no visible boundary; the DS prescribes an `input` border, which the mock adds with an outline that GPUI cannot do); Secondary tag border 1.26 / 1.25 (B6); real selected-row border 2.2-2.8 (B6); dark `danger` fill vs `surface_2` 3.09 and vs `err_bg` 2.66 (the button sits on the surface bar, 3.40, so fine, but any placement on a tinted panel fails); alert 30% border 1.65-2.1 (decorative, text carries); danger hover/pressed step (B9); real disabled label about 2.2-2.9 (B5).

## What is genuinely good

* Inventory, Confirm and Install scan in seconds; the text budget is visibly honoured (Find has about 8 static words). Titles are 1-3 words.
* Confirm: the destroyed / changed / preserved structure with names and sizes only, the unit identity strip, the typed phrase with a live `Matches` tag. Checkbox focus is clearly visible.
* Fonts really are bundled and load (Inter 400/600/700, JetBrains Mono 400, embedded as data URIs; `document.fonts` reports loaded) and tabular numerals are on at body level. No sub-12 px text.
* Lucide: 66 of 66 icons identical to the shipped set.
* Dark theme is re-derived (accent lightened, status tints re-chosen), not inverted; both themes stay calm, and red stays rare.
* Tooltips, popovers, dialogs, sheet and info icons are used as the disclosure ladder the brief asks for; the Help sheet and About page exist in both themes.

## What I could not verify

* GPUI rendering of anything (no real-app screenshots were supplied); the `Select` popup, `Tooltip` on a disabled `Button`, `border_dashed` with rounded corners, `tnum` font-feature exposure, the Windows/macOS/Linux title bar, HiDPI.
* Accessibility tree in GPUI (the mock honestly says screen readers are unsupported in the first-launch dialog).
* App-icon sizes and favicon (not in this prototype); the web surface.

## What would convince me (summary)

1. Real-app (or widget-faithful) screenshots, light and dark, of: the stepper, a table with a selected row, tags (all five compatibility states), alerts, disabled and hover/pressed buttons, an open `Select`, scrollbars at rest and on hover.
2. A working text-size setting with 130% screenshots at 960x640 of the five heaviest screens.
3. All states at exact DS metrics at 960x640, with Destroyed, Changed and Preserved visible together on Confirm.
4. The DS and the mock reconciled: type scale, radii, weights, exceptions list (stepper, selected row, alert actions), and one scroll container per screen with no horizontal scroll.
