# Direction 3: Friendly appliance

**Personality in three words: warm, roomy, reassuring.**

## Concept
Treat the installer like setting up a consumer appliance. Warm cream surfaces (`#F9F4EA`), brown-black text, a deep teal accent (`#086C68` light / `#60CCC5` dark), 16 px base type, 44 px controls, 12-24 px radii and pill badges with 16 px icons. The wizard is a horizontal stepper along the top (done steps collapse to checks, only the current step is named) so the page below gets the full width. Spot illustrations (a NAS with drive bays) appear on Login, Choose goal and Done. Copy is conversational ("Last check before we erase this NAS"); the data-loss gate keeps the same hard red banner, two lists and typed confirmation, so tone never lowers the guard.

## What is shown
| # | Surface | Screen |
|---|---|---|
| 1-10 | Desktop app (GPUI, 1180 px wide window mock, min 960 x 640) | Inventory, Connect, Inspect / compatibility, Choose goal, Storage plan (with blocking error), Confirm data loss, Install progress, Failure / recovery, Done, Update available |
| W1-W7 | NAS web UI (browser frame; mobile = 390 px phone) | Login, Dashboard, Storage + create-volume wizard with plan preview, Network with safe-apply countdown, Users, Boot manager, Mobile dashboard |

## How to view
Open `index.html` in a browser (no server, no network). Use the screen selector or the left / right arrow keys, the **Light / Dark** switch (the first load follows your OS setting) and, on *Install progress* and *Failure / recovery*, the log-level buttons. The "Update available" entry is the Inventory screen with its dialog open.

## Strengths
- Most approachable for non-experts; large targets (44 px) and generous spacing help motor and low-vision users.
- The horizontal stepper frees width for the storage diagram and tables.
- Illustrations double as empty-state and success art and are cheap (a few SVG paths).
- Passes every contrast pair (66/66 per theme; min text 5.51:1).

## Risks
- Needs more vertical room: Storage plan and Confirm exceed a 640 px window height and must scroll with a pinned action bar.
- Teal accent (hue 190) sits close to the success green (hue 155); an accent-coloured "selected" row can be mistaken for "OK" unless the check icon and word are always present.
- A friendly tone next to an erase action can read as casual; test the gate with real users.
- Warm cream plus amber warnings has less separation than a neutral background; warn-vs-canvas relies on the border.
- Departs from ForgeVault's look, so the web plane would diverge from the product family. System rounded font only exists on Apple platforms; elsewhere it falls back to the plain system UI font.
