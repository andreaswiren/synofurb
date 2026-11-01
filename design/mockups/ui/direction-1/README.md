# Direction 1: ForgeVault family

**Personality in three words: calm, neutral, dependable.**

## Concept
The desktop installer and the NAS web UI look like the ForgeVault product they sit beside: a neutral grey ramp, white cards with hairline borders, 8-12 px radii, lucide-style outline icons and exactly one accent, ForgeVault's violet (`oklch(0.50 0.21 295)`, `#7139CB` light / `#A37BFE` dark). Colour is kept for meaning: the accent says "primary / selected / progress", green-amber-red-blue say status, and red fills appear only on the data-loss banner, the destructive button and errors. Safety comes from structure and words (destroyed-vs-preserved lists, typed hostname, point-of-no-return strip) instead of decoration. Density is shadcn-default: 14 px text, 36 px controls.

## What is shown
| # | Surface | Screen |
|---|---|---|
| 1-10 | Desktop app (GPUI, 1180 px wide window mock, min 960 x 640) | Inventory, Connect, Inspect / compatibility, Choose goal, Storage plan (with blocking error), Confirm data loss, Install progress, Failure / recovery, Done, Update available |
| W1-W7 | NAS web UI (browser frame; mobile = 390 px phone) | Login, Dashboard, Storage + create-volume wizard with plan preview, Network with safe-apply countdown, Users, Boot manager, Mobile dashboard |

## How to view
Open `index.html` in a browser (no server, no network). Use the screen selector or the left / right arrow keys, the **Light / Dark** switch (the first load follows your OS setting) and, on *Install progress* and *Failure / recovery*, the log-level buttons. The "Update available" entry is the Inventory screen with its dialog open.

## Strengths
- Same tokens, components and tone on both surfaces and as ForgeVault; one design system to maintain (`../DESIGN-SYSTEM.md` documents this direction).
- Cheapest direction to build in GPUI: flat fills, 1 px borders, one small shadow, no gradients or blur.
- Status never depends on colour: badge + icon + word + border style (Untested is dashed).
- Highest text contrast of the three for the main text (primary text is 19.8:1 light / 17.2:1 dark on cards) and the smallest set of loud colours, so the one red button actually stands out.

## Risks
- Can feel generic or "just another dashboard"; personality has to come from copy and the product's own diagrams.
- Violet is also ForgeVault's brand, so synofurb is visually a sibling rather than its own product (acceptable or even desired here).
- Neutral hierarchy relies on spacing and weight; at 960 px wide the 212 px step rail takes a fifth of the window (collapse to 56 px icons below 1100 px).
- The destructive button is red; red/green colour-blind users depend on the trash icon and the verb in the label (both present).

## Contrast
66 of 66 declared pairs pass in each theme (min text ratio 4.63:1, min non-text 3.18:1). Full table in `../DESIGN-SYSTEM.md` section 3.
