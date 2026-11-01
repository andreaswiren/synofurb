# Recommendation

## Pick: Direction 1, "ForgeVault family" (revised after the UI critic review)

**Why.** The web plane must stay in the ForgeVault family, and Direction 1 is the only direction that does: same neutral ramp, one violet accent, 8 and 12 px radii, pill badges, lucide strokes. Its calm palette is what lets the one red thing on the data-loss gate be unmistakable (measured: red is on two elements on the gate and three once the erase button is enabled), and every element maps to cheap GPUI primitives. The independent critic approved it with conditions and rejected Directions 2 and 3 as a base; all six blocking conditions are addressed in this revision (log in `DESIGN-SYSTEM.md` section 10).

**Directions 2 and 3 are kept as alternatives, not revised after the review.** The critic's reasons, which stand: D2 uses 9.5 to 10.5 px labels, 30 px controls, uppercase tracked mono and a cyan accent close to the info blue, and leaves the ForgeVault family; D3 leaves the family, its 44 px controls and 16 px type do not fit a 960 x 640 window (the gate is about 1100 px tall), and the cream canvas weakens amber warnings. Their pages still open (`../direction-2/`, `../direction-3/`) but still show the old layout, the old tokens and the old copy (for example Verified models and the hostname phrase), so do not use them for decisions about wording or accessibility.

**Layout it serves.** Revised UX concept A (stepper across the top, persistent safety rail, pinned action bar, typed phrase `ERASE <MODEL>` with paste allowed, point of no return at installer step 3) and management plane mgmt-A with the status strip, "Needs attention", docked edit panel and a persistent safe-apply banner. Wording of failure and recovery states follows `docs/RECOVERY.md`.

### Ideas worth borrowing from the alternatives
1. From D2: a 4 px status edge on tiles that need attention, and a "show details" switch for expert columns (not all-caps labels, not 30 px controls).
2. From D3: the 42 px large control for the gate's buttons and 44 px everywhere on phones (done), optional spot illustrations on Done and empty states (never on the gate).
3. Own accent: tokens make a product-specific desktop accent a five-line change; re-run `tools/contrast.py` afterwards.

## What changed in this revision (summary; details in DESIGN-SYSTEM.md section 10)
- **B1** the enabled, hover, pressed, focus, disabled and loading destructive button is drawn and live; dark label 6.63:1 (was 4.63:1) with a light-red outline; red budget enforced and measured.
- **B2** renders at 960 x 640, 1180 x 760 and 1440 x 900, with scroll state, pinned bar, 130% text, three OS chromes; Find never clips Compatibility.
- **B3** the web plane reflows at 390 px with the same markup; the safe-apply banner is sticky and both buttons are 44 px.
- **B4** placeholder token fixed, tracks and bars outlined, borders 3:1 on every surface, log ring, and a checker that tests 472 pairs (token matrix 228, accent variants 100, extracted from CSS 144), fails on untested colour tokens and lints the files.
- **B5** one source of truth: no gradient, pattern, inset shadow or `color-mix`; 12 px floor; sentence case; tabular numerals everywhere; one font stack.
- **B6** `states.html` and live controls with focus rings.
- **Accents** only the derived accent sets are allowed on the web plane (green and amber of ForgeVault fail on white).

## Accessibility risks (highest first)
1. **GPUI exposes no screen-reader tree, to my knowledge, and this has not been verified.** The brief mentions possible AccessKit support in a GPUI kit; the two statements disagree, so the baseline is unknown. Do the one-day spike first (a button, a text field, a checkbox and a live region with NVDA, Narrator and VoiceOver). Until then the UI states plainly that screen readers are not supported in this window and offers the equivalent command (`synofurb install --host ... --have-backup --accept-data-loss --confirm`).
2. **The typed-phrase field is custom.** Caret, selection, clipboard, IME, undo, paste and password-manager behaviour all have to be built; paste is allowed by decision. Budget it before any screen.
3. **Text selection and copy.** Fingerprints, device paths, serials and commands must be copyable; the design gives them Copy buttons (log, fingerprint, passwords), but the serial numbers and the boot-flash backup path on the gate do not have one yet.
4. **Small type and scaling.** Floor is 12 px (lint-enforced); the 130% setting was checked on 960 x 640 for Find and Confirm only. Check every screen.
5. **Colour independence is designed in but only partly tested.** Greyscale and a deuteranopia simulation were eyeballed on the first-round mock; the revised screens have not been simulated. Protanopia, tritanopia and low vision were never run.
6. **OS contrast modes.** Custom-drawn UI ignores Windows Contrast Themes and macOS Increase Contrast; no high-contrast palette is drawn. Web: `forced-colors` is untested.
7. **Dark destructive button** now has 6.63:1 and a light outline; test that a dark-red fill with a white label still reads as the loudest element on cheap panels.
8. **Focus order and accidental activation.** Enter in the phrase field must never trigger the erase button; initial focus on Confirm goes to the heading; the destructive button is never the default button; rings are not clipped by `overflow: hidden` parents.
9. **Motion.** One busy glyph; static under reduced motion. Countdown announced at 10 s and 5 s only.
10. **Web plane.** `aria-current`, `role=alert`, `aria-live` are in the markup but not screen-reader tested.

## What to test
**Usability (6 to 8 people, mixed home-lab and non-technical):** gate comprehension (what is erased, what is changed, what survives, what happens if it fails), wrong-unit detection with two DS918+ units on the network (does the serial catch it), the typed phrase with and without paste, the Untested story (do people choose Boot manager first), failure screens before and after the point of no return, the safe-apply banner on a phone, and a greyscale print of the compatibility badges.
**Technical:** keyboard-only walkthrough of every desktop screen in a real GPUI build; 125 / 150 / 200% OS scaling; real Inter and JetBrains Mono metrics (the mock uses installed fonts); Windows Contrast Themes and macOS Increase Contrast; NVDA with Firefox and VoiceOver with Safari on the web plane; 360 px phones; colour-blindness simulators on every screen.

## What I verified and did not
- **Verified:** `python3 tools/contrast.py` passes 472 of 472 pairs (lowest text 4.78:1, lowest non-text 3.25:1) and its lint finds no gradient, pattern, inset shadow, `color-mix`, backdrop filter, uppercase transform or text under 12 px; it fails on deliberately bad tokens. Playwright screenshots of the desktop screens at 960 x 640, 1180 x 760 and 1440 x 900 (light and dark, 130% text, macOS chrome) and of the web screens at 1180 and 390 px; no horizontal overflow and no web control under 44 px at 390; red budget on the gate counted by script.
- **Not verified:** GPUI behaviour (mapping is from knowledge of the API), real screen readers, real users, Safari and Firefox rendering, real fonts, OS-native window chrome.
