# Direction 1: ForgeVault family (recommended, revised after the UI critic review)

**Personality in three words: calm, neutral, dependable.**

## Concept
The installer and the NAS management plane look like a sibling of ForgeVault: a neutral ramp, white cards with hairline borders, one violet accent, pill badges, lucide-style strokes, 8 and 12 px radii. Colour is kept for meaning. On the data-loss gate red is on exactly two things: the enabled erase button and the "Will be destroyed" marker and heading. The layout follows the revised UX concept A (stepper across the top, persistent safety rail on the right, pinned action bar) and mgmt-A (sidebar shell, status strip, "Needs attention", docked panel, persistent safe-apply banner).

## Files
- `index.html` + `styles.css`: the viewer and all screens. `states.html`: every component in every state, light and dark side by side.
- Controls in the viewer: screen selector, **Window** 960 x 640 / 1180 x 760 / 1440 x 900, **Text** 100 / 130%, **Chrome** Windows / macOS / Linux, **Scroll to end**, **Viewport** (web) 1180 / 768 / 390, Light / Dark. URL parameters: `screen`, `theme`, `size`, `vp`, `ts`, `os`, `scroll=end`, `embed=1`.
- Desktop screens: Find, Connect, Inspect, Goal, Plan (valid, and blocking error with a live slider), Confirm (incomplete, and satisfied; the gate is live: tick the boxes, type `ERASE DS918+`), Install (staging, after the point of no return; the log filter works), Failed before and after the point of no return, Done, Update dialog, States sheet. Web: Login, Dashboard, Storage, Network (safe apply pending), Users, Boot, plus Dashboard, Network and Users at 390 px.
- `../DESIGN-SYSTEM.md`, `../tokens.json`, `../tools/contrast.py`, `../RECOMMENDATION.md`.

## Check it
`python3 ../tools/contrast.py` checks 472 pairs and lints the files: all pass (lowest text 4.78:1, lowest non-text 3.25:1).

## Strengths
Same tokens and components on both surfaces and as ForgeVault; flat, cheap-to-build shapes; status by icon, word and border style; one dominant red control; reflows from 960 x 640 to 1440 x 900 and from 1180 to 390 px without horizontal scrolling.

## Risks
GPUI accessibility tree and text input are unverified (the largest risk); the mock uses installed fonts, not the bundled Inter and JetBrains Mono; can look generic; OS-native window chrome is only suggested; screens not drawn are listed at the top of `DESIGN-SYSTEM.md`.
