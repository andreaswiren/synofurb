> **Alternative, not revised after review.** The independent UI critic rejected this direction as the base for synofurb: D2 uses 9.5 to 10.5 px labels, 30 px controls, uppercase tracked mono and a cyan accent that is close to its info blue, and it leaves the ForgeVault family. It is kept for reference and for the ideas listed in `../RECOMMENDATION.md`. It still shows the **first-round** layout (left step rail, no safety rail), tokens, copy and data (including a Verified model and a hostname phrase), none of which match the revised UX mock or `docs/RECOVERY.md`. Its contrast numbers below cover declared pairs only. The recommended, revised direction is `../direction-1/`.

# Direction 2: Ops console

**Personality in three words: dense, technical, status-forward.**

## Concept
A tool for people who already live in terminals. Monospace carries headings, navigation, table headers (uppercase, wide-tracked), badges and every identifier; body copy stays sans. 13 px base, 30 px controls, 3 px radii, hairline cell grids, flat panels with strip headers. Status is loud and positional: tiles carry a 3 px coloured left edge, banners a 4 px edge, badges are square uppercase tags. The accent is a cold cyan-blue (`#06658E` light / `#43CAE7` dark) so the red, amber and green of status are the only warm colours on screen. The step rail uses numbered square cells. Extra power-user columns (RTT, power-on hours) are on by default.

## What is shown
| # | Surface | Screen |
|---|---|---|
| 1-10 | Desktop app (GPUI, 1180 px wide window mock, min 960 x 640) | Inventory, Connect, Inspect / compatibility, Choose goal, Storage plan (with blocking error), Confirm data loss, Install progress, Failure / recovery, Done, Update available |
| W1-W7 | NAS web UI (browser frame; mobile = 390 px phone) | Login, Dashboard, Storage + create-volume wizard with plan preview, Network with safe-apply countdown, Users, Boot manager, Mobile dashboard |

## How to view
Open `index.html` in a browser (no server, no network). Use the screen selector or the left / right arrow keys, the **Light / Dark** switch (the first load follows your OS setting) and, on *Install progress* and *Failure / recovery*, the log-level buttons. The "Update available" entry is the Inventory screen with its dialog open.

## Strengths
- Fastest to scan: more rows and columns per screen, aligned numbers, status visible from across the room.
- Logs, device paths, fingerprints and commands feel native; the install log screen is the strongest of the three.
- Works well as a dark theme for rack-side use; the dark theme is the hero of this direction.
- Still passes every contrast pair (66/66 per theme; min text 5.05:1).

## Risks
- Intimidating for non-experts, which is part of the audience (people reinstalling a family NAS).
- Uppercase tracked monospace labels are slower to read and are tiring for dyslexic readers; 10.5-11 px micro labels sit at the floor of legibility and 30 px controls are small for touch (the phone layout would need its own sizing).
- Accent cyan (hue 235) and info blue (hue 240) are close; "accent" and "info" can be confused unless icons are always present.
- Mono-heavy UI needs a bundled monospace font on every OS; mixed-width alignment (sans body in mono frames) needs care in GPUI.
- Further from ForgeVault: the web plane would no longer look like a ForgeVault sibling.
