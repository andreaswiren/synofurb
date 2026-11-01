# Recommendation

## Pick: Direction 1, "ForgeVault family", with three borrowed ideas

**Why.** synofurb has two surfaces but one job: make an irreversible operation feel understood. Direction 1 is the only one where (a) the web plane is already specified by an existing product look (ForgeVault), so a second visual language would be pure cost, (b) the palette is quiet enough that the single red destructive button and the data-loss banner are unmistakable, and (c) every element maps to cheap GPUI primitives (flat fills, 1 px borders, small radii, monochrome SVG). It also has the highest primary-text contrast (19.8:1 light, 17.2:1 dark on cards) and the fewest moving parts to keep accessible. Directions 2 and 3 each have a real strength (scan speed, approachability) that Direction 1 can borrow without importing their risks.

| Criterion (1 low, 5 high; my judgement, not user-tested) | D1 | D2 | D3 |
|---|:-:|:-:|:-:|
| Data-loss gate clarity (is the red button the loudest, rarest thing?) | 5 | 4 | 4 |
| Information density / expert scan speed | 3 | 5 | 2 |
| Approachability for non-experts | 3 | 2 | 5 |
| Contrast and non-colour cues (declared pairs pass in all three: 66/66, 66/66, 66/66) | 5 | 4 | 4 |
| Touch / motor friendliness (control height 36 / 30 / 44 px) | 3 | 2 | 5 |
| Cost in GPUI (fewer custom pieces is better) | 5 | 4 | 3 |
| Fit with ForgeVault family (web plane) | 5 | 3 | 2 |
| Differentiation / personality | 2 | 4 | 4 |
| **Total (40)** | **31** | **28** | **29** |

### Borrow into Direction 1
1. **From D2: status edges and an "expert details" switch.** A 3 px left edge on health tiles and table rows that need attention (ok / warn / err) for glanceability, and a "Show details" toggle that adds the dense columns (RTT, power-on hours, serials). Do not adopt all-caps tracked mono labels (legibility) or the 30 px control height.
2. **From D3: the large destructive gate and a collapsing stepper.** Use the 42 px "lg" control size for every button on the Confirm screen and for all phone controls (44 px). Below 1100 px window width, collapse the 212 px step rail into D3's horizontal stepper rather than a bare icon rail, which keeps step names for the current step. Optional drive-bay spot illustration for the Done and empty states only; never on the gate.
3. **Own accent decision.** Keep ForgeVault violet for the web plane (where the accent switcher stays) but consider giving the desktop installer a fixed, product-specific hue later; tokens make that a one-line change (`accent`, `accent_hover`, `accent_bg`, `accent_fg`, `ring`), after re-running the contrast script.

## Accessibility risks (highest first)

1. **GPUI exposes no screen-reader tree.** To my knowledge there is no AccessKit bridge in GPUI yet (verify against the pinned revision). The native installer may therefore be unusable with NVDA, JAWS, Narrator or VoiceOver. This is the largest risk of the whole surface choice, and no visual direction fixes it. Mitigations: keyboard-complete flows; every destructive step also reachable as a documented, non-interactive CLI path (`synofurb install --dry-run`, `--yes --hostname`), so blind users are never forced through the GUI; or watch for / contribute AccessKit support.
2. **Typed-confirmation input is custom.** GPUI has no stock text field, so IME, selection, clipboard, paste and password-manager behaviour all have to be built. The mock says "paste is disabled on purpose"; paste blocking harms assistive tech and password managers. Recommendation: allow paste, still require exact hostname equality (copying the name is already a deliberate act), and decide with usability data.
3. **Dark destructive button uses a near-black label on red** (`on_danger` #0C0403 on `danger` #E23439 = 4.63:1, the narrowest text pair, 13 px / 600). It passes AA but is the pair most likely to fail on cheap panels or with colour filters. Alternative to test: white label on a darker red (about #C62828) in dark mode.
4. **Small type.** Captions, badges and table headers are 12 px; log lines 12 px mono. Ship the 115 / 130% text-scale setting from day one and test the 960 x 640 minimum at 130%.
5. **Colour independence is designed in but unverified on people.** I checked greyscale and a deuteranopia simulation of Inventory and Install screens by eye (badges stay separable through icon, word and dashed border; step state through glyph). Protanopia / tritanopia and low-vision simulation were not run.
6. **Custom-drawn UI ignores OS high-contrast modes** (Windows Contrast Themes, macOS Increase Contrast). Provide a first-party high-contrast palette (`border` becomes `border_strong`, fills become `surface` / `fg` pairs) and honour `forced-colors` in the web plane.
7. **Motion.** One looping spinner; replace it by a static glyph with the word "running" under reduced motion. The 1-per-second countdown ring is not animation, but the text must be exposed in the live region at 10 s and 5 s, not every second.
8. **Focus order and accidental activation on the gate.** Enter on a focused field must never trigger the destructive action; the destructive button must not be the default button; initial focus on the Confirm screen goes to the heading, not to a checkbox or button.
9. **Fonts.** Inter and the mono font must be bundled, or GPUI falls back to whatever the OS has, changing metrics (the mocks use system fonts, so their line breaks will differ slightly).
10. **Web plane.** Tables become stacked cards below 600 px; 44 px targets; `aria-current`, `role=alert` for the safe-apply banner and `aria-live` for the countdown are in the mockup markup but untested with a screen reader.

## What to test

**Usability (n = 6-8, mixed: 3 homelab users, 3 non-technical; moderated, think-aloud)**
- *Gate comprehension.* Show the Confirm screen for a DS918+; ask them to say in their own words what will be erased and what will not. Pass: 100% name "all four disks / all data"; at least 90% understand DSM data cannot be brought back; nobody expects the PC copy to be erased.
- *Wrong-NAS guard.* Put two NAS on the network (DS918+ and DS923+); can they tell which one is about to be erased from the title bar, banner and typed hostname alone? Count near-misses.
- *Typed confirmation.* Compare "paste allowed" vs "paste blocked" on error rate, time, and annoyance; also hostname length 9 vs 20 characters.
- *Compatibility badges.* Five-second recognition test with a greyscale print: map each badge to its meaning and to "can I install here".
- *Failure screen.* Do users choose "Retry without sdc" vs "Restore DSM"; do they understand the NAS is not bootable and must stay powered?
- *Log filter.* Can they find the first warning in the install log in under 15 seconds.

**Technical / accessibility**
- Re-run the contrast script on every token change and for any accent other than violet (`tools/contrast.py` reads `tokens.json`; the numbers in `DESIGN-SYSTEM.md` section 3 come from the same formula).
- Keyboard-only walkthrough of all 10 desktop screens; focus ring visible on every control in both themes (needs a real GPUI build).
- Windows at 125 / 150 / 200% scaling, macOS Retina and a 1x external display; Inter and mono rendering and hinting on each.
- Windows Contrast Themes and macOS Increase Contrast; web `forced-colors`.
- NVDA + Firefox and VoiceOver + Safari on the web plane (login, dashboard, network rollback, boot manager); verify the safe-apply countdown announcement.
- 960 x 640 minimum window at 130% text scale: nothing clipped, action bar always visible.
- 360 px phone width for the web dashboard; touch target sizes.
- Colour-blind simulations (protan, deutan, tritan) and a 50% blur test on all screens, especially the gate and the log.
- Dark theme destructive button alternatives (item 3 above).

## What I did and did not verify
- Verified: every declared text/background pair (66 per theme in each direction) computed with the WCAG 2.x formula from the exact hex values used in the CSS; no unclosed tags; all pages open from disk with no network; screenshots of representative screens in light and dark checked in headless Chrome; sizes: each direction page is under 150 KB.
- Not verified: behaviour in GPUI (the mapping in `DESIGN-SYSTEM.md` section 9 is from knowledge of its API, check against the pinned revision), real screen readers, real users, Safari and Firefox rendering (`color-mix()` is used for tints in the HTML only; GPUI uses the opaque tokens).
