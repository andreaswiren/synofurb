# UI critic, Welcome guide, round 1

Scope: commit `41280f8` ("Welcome guide on first launch"): `crates/synofurb/src/ui/screens/welcome.rs`, `ui/welcome.rs`, the `WELCOME_*` strings in `copy.rs`, `overlays.rs::open_welcome`, the Help row and menu item, `design/v2/UX-SPEC.md` D1b, DEV-UI-23 in `design/v2/DESIGN-SYSTEM-V2.md`, and the 21 `design/screenshots/v2-app-welcome-*.png`. No gate verdict; nothing written to `design/approvals/`; no code edited; no build (I read the vendored `gpui-component-0.8.1` / `gpui-kit-0.8.1` sources instead). Every contrast value is measured from pixels (dominant background against the farthest text colour in a box). I looked at 10 of the 21 screenshots and checked all 21 programmatically (dialog extent, position of the current dot, duplicates). Scratch files are in the session scratchpad and are deleted.

## Verdict

The dialog is tidy and readable: one icon, one headline, two short lines, one primary, a visible reason for the disabled Back, all colours from tokens, nothing under 5.8:1 except the disabled Back (exempt), dark and Largest both exist and are clean. It is **not** the "buttons do not move" dialog the spec promises, the DEV-UI-23 justification for the custom dots rests on a premise that is false, and one line of its copy contradicts the product's own recovery text. **Blocking 0, major 4, minor 11.**

## Major

**M1. DEV-UI-23's dots: reject as written.**
* The register says there is "no stock widget for the dots row". There is: `gpui_kit::component::carousel` re-exports `Carousel`, `CarouselPagination` and `CarouselPaginationItem` (gpui-component 0.8.1, `src/carousel/carousel.rs`). A pagination item is a compact `Button` with a selected state, a generated "Go to slide N" accessibility label and arrow-key handling. I did not render it, so its default look may need a style override to get the accent pill; but the premise "no widget" is wrong and the register must say why the widget was not used.
* The justification "the pill shape and the Back / Next buttons carry the same information" is also wrong: Back and Next carry no position. Position is carried only by an 8 px shape: no text ("2 of 5"), no label, no focus stop, and at Largest the dots stay 8 px (`px(8.0)`, `px(20.0)` in `welcome.rs:dots`) while everything around them scales.
* It adds a third progress idiom to the product beside the window Stepper and the 12-segment install strip (DEV-UI-17), with a different shape each time.
* The pill itself is fine: accent `#7139CB` 6.67:1 and `#A37BFE` 5.82:1, inactive dots `#808080` 3.95:1 (light) and `#868686` 4.92:1 (dark), all at least the 3:1 non-text minimum, and the state is shape plus colour (a longer pill), not colour alone.
* Verdict: **justify only with a visible text counter** ("2 of 5", caption, 12 px muted) next to the dots, or replace them with the carousel pagination and a label. Correct the register line either way.

**M2. The spec promises one height on every step and the dialog does not deliver it.** UX-SPEC D1b: "The text block keeps one height on all steps, so the buttons do not move." Measured dialog heights (identical in light and dark): steps 1, 3, 4, 5 are 300 px, step 2 is **313 px**; at Largest 310 px and **348 px**. The dialog is anchored at y = 65, so on step 2 Skip, Back and Next sit 13 px lower (38 px at Largest) than on steps 1 and 3 (`welcome-1-light.png`: buttons at y = 332; `welcome-2-light.png`: 346). `BODY_MIN_H = 136` is a fixed pixel minimum, so it fits the one-line-plus-Stepper step only at 100 % text. Must change: make the minimum height a rem value that covers the Stepper step at every text size (or put the Stepper row in all steps' reserved space), then re-shoot step 2 and 3 at both sizes.

**M3. Step 4's headline line contradicts the product's own recovery text.** "The way back. DSM stays the default boot entry; Recovery restores it." The Confirm help dialog ("How recovery works", `dlg-recovery-light.png`) says from installer step 3 "DSM's own system may be gone; reinstall it from Synology. There is no rescue shell. We promise nothing about your data", and DEV-UI-18 exists because the product must not over-promise. A first-time user reads "the way back" as "I can return to DSM". Not a design-system defect, but it is the first screen about safety the user sees; the UX critic and the owner should decide the wording before this ships (for example: "Before step 3 you can always go back. After it, DSM may need a reinstall").

**M4. The screen-reader disclosure moved behind an info button.** UX-SPEC's accessibility baseline is "stated in the UI". The old first-launch dialog showed the sentence in the body. In the Welcome guide it is the tooltip and popover body of a 24 px (?) at the end of step 3's second line (`WELCOME_3_INFO`), a hover-only tip (DEV-UI-1) and a click-to-open popover, and the word budget excludes info bodies. The people for whom it matters are the least able to find it. Must change: one short visible line on step 3 ("Screen readers are not supported in this window; use the command line."), which fits the two-line budget by shortening line B, or move it to step 1.

## Minor

* **m1. Horizontal inset is 32 px, every other dialog uses 16.** Welcome content starts at x = 195 in a dialog whose left edge is 163 (`welcome.rs` adds `px(16.0)` on top of the dialog's own 16 px); Update, Command, Manual recovery and the rest start text 16 px from the edge. The close X still sits at the 16 px inset (781 against a right content edge of 769), so it floats outside the column. Drop the extra `px`.
* **m2. No dialog title.** Headline, close X and footer only; the headline is a plain `div` in `type.title` (24 px, 700), where `theme.rs` says dialog titles use `SECTION` (18 px). The dialog therefore has no accessible name and a different title size from the other dialogs. Registered in DEV-UI-23 and visually defensible for a welcome; still call it out as a deviation from "Dialog = title, 2 to 4 short paragraphs, one primary action".
* **m3. The six-step row is a second, inert Stepper.** `welcome-2-light.png`: a 6-circle row with Find filled as current while the real Stepper behind the scrim is also on Find, so two identical steppers say the same thing and one cannot be clicked. The dialog copy of it also differs from the window's: labels are near-black regular (`#1B1922` 17.4:1, current label not bold) where the window uses `#525252` for upcoming and bold for current. Draw all six neutral (no current), or use a tile row instead of the Stepper look.
* **m4. Back on step 1 is disabled, with a (?) button that says "Already at the start."** It follows the disabled-reason pattern correctly (visible reason button next to the control, Popover; `welcome-1-light.png`) and the disabled label is 2.35:1 / 2.85:1 (exempt). But the reason explains the obvious and shifts Back 23 px left of its position on steps 2 to 5. A first step of a wizard has no Back: do not render it; keep the pattern for controls that are unavailable for a reason the user cannot see.
* **m5. Skip is misaligned with the column.** Its text starts at x = 205, the tile, headline, lines and dots at 195 (ghost button padding). Align the ghost label to the edge (negative margin) or give it an outline.
* **m6. Focus on open is invisible and Enter does nothing.** `open_welcome` focuses the view's own `FocusHandle` (a container, no ring), so keyboard focus is not on any control; `Guide::key` handles Left and Right only and the tests assert that Enter, Escape and Tab are "not ours". A user who presses Enter on the dialog gets nothing, where in every other dialog Enter activates the default. Unverified by me (no key injection tool here); the code and tests say so.
* **m7. Esc, X and Skip on steps 1 to 4 all mean "never again".** Per `welcome.rs` rules the guide is marked seen when dismissed early; an accidental Esc ends the first-launch guide forever (replay stays in Help and the menu). Probably right, but confirm with the UX critic; the Don't show again box appears only on step 5, so the user is never asked.
* **m8. Help row.** `welcome-help-light.png`: "Welcome guide" is the first row with a sparkle icon and no chevron; its text starts at x = 535 while the 12 topic rows start at 511. The sparkle is also the "Recommended size" icon on Plan; one icon, two meanings. Use the icon column for all rows or for none.
* **m9. Body text size.** By eye the Welcome lines are 16 px foreground (about 7.3 px per character) where Cancel install's body is 14 px muted (about 6.5 px per character). Both sizes are on the scale; pick one body size for dialogs.
* **m10. Dots and Largest.** At Largest the headline and lines grow 30 % and the dots, tile (44 px) and icon (24 px) do not. Acceptable for a decoration, wrong for the only position indicator (see M1).
* **m11. Offscale values.** `44` for the tile is a control height in the DS (lg input), not a tile size; `6` for the dot gap is on the scale, `16`, `4`, `8` too. Nothing else is off-scale; no hex colours, no Medium weight, Lucide only (`SERVER`, `LAYERS`, `WARN`, `RECOVER`, `ROCKET`, `BACK`, `SPARKLES` through `icons::role`).

## Conformance, checked and fine

* **One primary:** Next (steps 1 to 4), Start (step 5); Skip is a ghost button, Back an outline; Start replaces Skip so the last step has exactly one way out plus the checkbox. Test `one_primary_per_step_start_only_at_the_end` covers it.
* **Tokens and type:** colours come from `cx.pal()` / theme (`accent`, `border_strong`, `surface_2`, `border`), type from `theme::TITLE` and the theme default; no `Medium` weight; tile uses `surface_2` and `border` as the DS asks; the warning icon uses the warning text colour with a word beside it (never colour alone).
* **Dark mode:** dialog `#171717` (popover) as in the other dark dialogs; every state measured below.
* **Overflow:** none at 960 x 640, default or Largest. The 640 px dialog (`DIALOG_W_WIDE`, the same width as Update and Command) leaves 160 px either side; the six-step row fits at Largest (`welcome-2-light-largest.png`, "Confirm" and "Install" labels fully visible), no scrollbar, no clipping. Text block min height only grows.
* **Word budget:** 23 visible words on the worst steps plus the shell, 34 of 60, enforced in `copy.rs` and `welcome.rs`.

## Contrast, measured in pixels

Light (dialog `#FFFFFF`): headline and lines `#0A0A0A` 19.80; icon tile `#F3F3F3` with accent icon `#5C29AA` 8.04, warning icon `#6F4606` 7.41; Skip 19.80; Next, Start and checkbox fill `#7139CB` with white 6.67; close X 19.03; info (?) 17.58; Back outline label 18.97; disabled Back 2.35 (exempt, reason visible); Stepper circles 17.84 (upcoming), 6.67 (current); connector line `#E5E5E5` 1.26 (decorative, as in the window).
Dark (dialog `#171717`): headline and lines `#FAFAFA` 17.18; tile `#202020` with icon `#C8B7FF` 9.06, warning icon `#F1C45E` 9.93; Next and Start `#A37BFE` with `#0E0C15` 6.30; checkbox 5.82; Back outline 11.23; disabled Back 2.85 (exempt); info (?) 16.01; close X 16.59; connector 1.25 (decorative).
Non-text: dots 3.95 / 4.92, current pill 6.67 / 5.82.

## The screenshots

| Check | Result |
|---|---|
| Count and names | 21 files: steps 1 to 5 in light, dark, light-largest and dark-largest (20) plus `welcome-help-light.png` (the Help sheet showing the new first row). No duplicates (md5 distinct). All 960 x 640. |
| Each shows its state | Yes. I located the current dot in the 15 files where the step is not 2 and it matches the file name; for the five step-2 files (the first accent run found was the Stepper's current circle) I looked at `welcome-2-light`, `-light-largest` and `-dark` and the second dot is the pill. Step 3 shows the warning tile and the (?) after line 2, step 5 the ticked "Don't show again" with Start in place of Skip. |
| Dark and Largest variants | Present for all five steps. Largest in dark exists for every step. |
| Not shown | Focus ring on any control in the dialog; hover and pressed; the two popovers (step 3 and step 4 info; the disabled-Back reason); the "Don't show again" tooltip; the dialog unticked; the Welcome opened from the menu; the Help sheet in dark or at Largest; the dialog at 1180 x 760; the step 2 Stepper row at default text with Find shown neutral. The "Good to know" popover is the one carrying the screen-reader notice (M4), so the most important new text is in no screenshot. |
| Geometry evidence | Dialog heights per step measured from the files: 300 / 313 / 300 / 300 / 300 px (310 / 348 / 310 / 310 / 310 at Largest): M2. |

## Evidence the gate would still need for this dialog

1. Popover open on step 3 (info), step 4 (info) and the Back reason, light and dark.
2. Keyboard: Tab order through Skip, Back, Next and the checkbox, the focus ring on each (DEV-UI-22's solid line), what Enter and arrow keys do; a recorded walk, not a still.
3. Largest on 1180 x 760 and on a short window; dark Help sheet.
4. A screen-reader or at least accessibility-tree dump is not producible here (accepted baseline); the dialog's lack of title (m2) means nothing names it.

## Recommended next steps

1. M2: reserve the Stepper row's height in all steps (rem based); re-shoot steps 2 and 3, both sizes.
2. M1: add a visible "2 of 5" caption (or switch to `CarouselPagination`) and correct DEV-UI-23.
3. M3 and M4: owner and UX critic settle the step 4 wording and where the screen-reader sentence lives.
4. m1, m5 and m4: 16 px inset, aligned Skip, no Back on step 1.
5. Add the popover, keyboard and focus captures listed above.
